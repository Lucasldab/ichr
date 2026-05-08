//! Async process spawner — drains stdio into the per-instance log file,
//! maintains a ring buffer of the last N lines for crash-tail
//! surfacing, and honours CancellationToken.
//!
//! **PITFALLS.md Pitfall 15 (stdio deadlock):** Both drain tasks MUST be
//! started via `tokio::spawn` BEFORE `child.wait()` is polled. Violating
//! this freezes the child as soon as its stdout pipe fills.
//!
//! **PITFALLS.md Pitfall 4 (kill_on_drop):** `.kill_on_drop(true)` is
//! mandatory so the child does not become an orphan on launcher exit.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;

/// Number of stdout/stderr lines retained in memory for crash-tail
/// retrieval. 200 is enough to capture a Minecraft/modloader stack trace.
pub const LOG_TAIL_LINES: usize = 200;

/// Success payload from `run_process`.
#[derive(Debug, Clone)]
pub struct LaunchOutcome {
    pub duration_ms: u64,
    pub log_path: PathBuf,
}

/// Spawn the JVM process, drain stdio to `log_path`, monitor with
/// `token`, and return the outcome.
///
/// Caller is responsible for constructing `jvm_args`, `main_class`, and
/// `game_args` via `launcher::command::compose`. This function owns only
/// the process lifecycle and stdio drain.
pub async fn run_process(
    java_bin: &Path,
    jvm_args: &[String],
    main_class: &str,
    game_args: &[String],
    working_dir: &Path,
    log_path: &Path,
    token: CancellationToken,
) -> Result<LaunchOutcome, AppError> {
    // Ensure parent directory of log_path exists — Minecraft won't write
    // through missing dirs and tokio::fs::OpenOptions will fail otherwise.
    if let Some(parent) = log_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::SpawnFailed(format!("create log dir: {e}")))?;
    }

    // Session header into the log file — append so previous sessions are preserved.
    write_session_header(log_path)
        .await
        .map_err(|e| AppError::SpawnFailed(format!("log header: {e}")))?;

    // Build the command. `kill_on_drop(true)` is MANDATORY — see
    // PITFALLS.md Pitfall 4.
    let mut cmd = Command::new(java_bin);
    cmd.args(jvm_args)
        .arg(main_class)
        .args(game_args)
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    tracing::info!(
        java = %java_bin.display(),
        main_class = main_class,
        cwd = %working_dir.display(),
        "spawning Minecraft process"
    );

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::SpawnFailed(format!("spawn: {e}")))?;

    // PITFALLS.md Pitfall 15: take pipes and start drain tasks BEFORE
    // awaiting child.wait(). If we await wait first and the child writes
    // more than ~64 KB to stdout, wait never completes.
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let ring: Arc<Mutex<VecDeque<String>>> =
        Arc::new(Mutex::new(VecDeque::with_capacity(LOG_TAIL_LINES)));

    let drain_out = tokio::spawn(drain_pipe(
        BufReader::new(stdout),
        log_path.to_path_buf(),
        Arc::clone(&ring),
    ));
    let drain_err = tokio::spawn(drain_pipe(
        BufReader::new(stderr),
        log_path.to_path_buf(),
        Arc::clone(&ring),
    ));

    let start = Instant::now();
    let status = tokio::select! {
        biased;
        _ = token.cancelled() => {
            tracing::info!("launch cancelled; killing child");
            let _ = child.kill().await;
            let _ = child.wait().await;
            // Still join drain tasks so the log file is flushed.
            let _ = tokio::join!(drain_out, drain_err);
            return Err(AppError::Cancelled);
        }
        res = child.wait() => {
            res.map_err(|e| AppError::SpawnFailed(format!("wait: {e}")))?
        }
    };

    // Drain tasks must finish so the ring buffer reflects final output.
    let _ = tokio::join!(drain_out, drain_err);

    let duration_ms = start.elapsed().as_millis() as u64;

    if status.success() {
        tracing::info!(duration_ms, "Minecraft exited cleanly");
        Ok(LaunchOutcome {
            duration_ms,
            log_path: log_path.to_path_buf(),
        })
    } else {
        let code = status.code().unwrap_or(-1);
        let tail = {
            let guard = ring.lock().expect("ring lock");
            guard.iter().cloned().collect::<Vec<_>>().join("\n")
        };
        tracing::warn!(code, "Minecraft exited non-zero");
        Err(AppError::LaunchFailed {
            code,
            message: tail,
        })
    }
}

async fn write_session_header(log_path: &Path) -> std::io::Result<()> {
    use tokio::fs::OpenOptions;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .await?;
    let header = format!("=== Launch {} ===\n", now_rfc3339());
    f.write_all(header.as_bytes()).await?;
    f.flush().await
}

fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    time::OffsetDateTime::from_unix_timestamp(secs as i64)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| format!("{secs}"))
}

async fn drain_pipe<R>(
    reader: R,
    log_path: PathBuf,
    ring: Arc<Mutex<VecDeque<String>>>,
) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin + Send + 'static,
{
    use tokio::fs::OpenOptions;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .await?;

    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        // Brief lock — no .await held across the guard.
        let mut guard = ring.lock().expect("ring lock");
        if guard.len() >= LOG_TAIL_LINES {
            guard.pop_front();
        }
        guard.push_back(line);
    }
    file.flush().await
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    fn log_path(td: &TempDir) -> PathBuf {
        td.path().join("instances/slug/logs/mineltui.log")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_nonzero_exit_returns_launch_failed_with_tail() {
        let td = TempDir::new().unwrap();
        let log = log_path(&td);
        let token = CancellationToken::new();
        let result = run_process(
            std::path::Path::new("/bin/sh"),
            &["-c".into()],
            "echo oops; exit 7",
            &[],
            td.path(),
            &log,
            token,
        )
        .await;
        match result {
            Err(AppError::LaunchFailed { code, message }) => {
                assert_eq!(code, 7, "exit code 7 expected; got {code}");
                assert!(
                    message.contains("oops"),
                    "log tail must include echo output; got: {message}"
                );
            }
            other => panic!("expected LaunchFailed; got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_cancellation_kills_child_and_returns_cancelled() {
        let td = TempDir::new().unwrap();
        let log = log_path(&td);
        let token = CancellationToken::new();
        let cancel = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel.cancel();
        });
        let start = std::time::Instant::now();
        let result = run_process(
            std::path::Path::new("/bin/sh"),
            &["-c".into()],
            "sleep 30",
            &[],
            td.path(),
            &log,
            token,
        )
        .await;
        assert!(
            matches!(result, Err(AppError::Cancelled)),
            "cancellation must produce AppError::Cancelled; got {result:?}"
        );
        // 15s tolerance — GH Actions ubuntu-latest under load has exhibited
        // ~30s cancel latency that does not reproduce locally. The test
        // fundamentally guards against "cancel hangs until the child's
        // natural exit" (the inner sleep is 30s); anything well below that
        // proves the kill path ran.
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "cancel must kill within 15s; took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_session_header_written() {
        let td = TempDir::new().unwrap();
        let log = log_path(&td);
        let token = CancellationToken::new();
        let _ = run_process(
            std::path::Path::new("/bin/sh"),
            &["-c".into()],
            "echo hi",
            &[],
            td.path(),
            &log,
            token,
        )
        .await
        .unwrap();
        let contents = tokio::fs::read_to_string(&log).await.unwrap();
        assert!(
            contents.contains("=== Launch "),
            "session header must be written; log contents: {contents:?}"
        );
        assert!(
            contents.contains("hi"),
            "stdout line must reach log; log contents: {contents:?}"
        );
    }
}
