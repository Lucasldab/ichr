//! Async installer subprocess runner — drains stdio into a per-install
//! log file, maintains a ring buffer of the last LOG_TAIL_LINES lines,
//! and honours CancellationToken with a 5-second SIGTERM grace before
//! hard kill (per 07-CONTEXT.md "Cancel Mid-Subprocess" entry).
//!
//! **07-RESEARCH.md Pitfall 4 (stdio deadlock):** Both drain tasks MUST
//! be `tokio::spawn`'d BEFORE `child.wait()` is polled.
//!
//! **07-RESEARCH.md Pitfall 6 (cleanup on cancel):** This module does
//! NOT clean staging — the caller (LoaderService) does, after this fn
//! returns Cancelled.
//!
//! **D-02 live tail:** Every ~500ms while the subprocess runs, we emit
//! a `TaskEvent::Progress { msg: "[log-tail] <last line>" }` with
//! `pct: 50` (fixed midpoint; the run.rs forwarder filters by the
//! `[log-tail]` prefix to avoid clobbering the real progress percentage).

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::loader::error::LoaderError;
use crate::tasks::{JobId, TaskEvent};

pub use crate::launcher::spawn::LOG_TAIL_LINES;

/// How long to wait between SIGTERM and SIGKILL on cancel.
pub const CANCEL_GRACE: Duration = Duration::from_secs(5);

/// How often to emit a [log-tail] progress event for D-02 live tail.
const LOG_TAIL_PERIOD: Duration = Duration::from_millis(500);

/// Run the JVM installer subprocess and drain its output. Returns Ok(())
/// on exit code 0; SubprocessExit on non-zero; Cancelled on token fire.
///
/// Both stdout AND stderr drain tasks are `tokio::spawn`'d BEFORE
/// `child.wait()` is polled — Pitfall 4 (stdio deadlock) prevention.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip_all, fields(?job_id, cwd = %cwd.display()))]
pub async fn run_installer(
    java_bin: &Path,
    jvm_args: &[String],
    args: &[String],
    cwd: &Path,
    log_path: &Path,
    progress_tx: mpsc::Sender<TaskEvent>,
    job_id: JobId,
    token: CancellationToken,
) -> Result<(), LoaderError> {
    let mut cmd = Command::new(java_bin);
    cmd.args(jvm_args)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Pre-create the log file directory.
    if let Some(parent) = log_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            LoaderError::ProfileWrite {
                path: log_path.display().to_string(),
                reason: format!("create log dir: {e}"),
            }
        })?;
    }

    // Open the log file (append). Wrap in Arc<TokioMutex<File>> for shared writes.
    let log_file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .await
        .map_err(|e| LoaderError::ProfileWrite {
            path: log_path.display().to_string(),
            reason: format!("open log file: {e}"),
        })?;
    let log_file = Arc::new(tokio::sync::Mutex::new(log_file));

    // Session header.
    write_session_header(&log_file, java_bin, jvm_args, args).await?;

    let mut child = cmd.spawn().map_err(|e| LoaderError::ProfileWrite {
        path: cwd.display().to_string(),
        reason: format!("spawn installer: {e}"),
    })?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let ring: Arc<StdMutex<VecDeque<String>>> =
        Arc::new(StdMutex::new(VecDeque::with_capacity(LOG_TAIL_LINES)));

    // PITFALL 4: drain tasks spawned BEFORE child.wait() is polled.
    let drain_out = tokio::spawn(drain_pipe(
        BufReader::new(stdout),
        Arc::clone(&log_file),
        Arc::clone(&ring),
        "[stdout] ",
    ));
    let drain_err = tokio::spawn(drain_pipe(
        BufReader::new(stderr),
        Arc::clone(&log_file),
        Arc::clone(&ring),
        "[stderr] ",
    ));

    // Periodic log-tail forwarder (D-02 live tail).
    let tail_forwarder = {
        let ring_c = Arc::clone(&ring);
        let progress_c = progress_tx.clone();
        let token_c = token.clone();
        tokio::spawn(async move {
            let mut last_sent: Option<String> = None;
            loop {
                tokio::select! {
                    biased;
                    _ = token_c.cancelled() => break,
                    _ = tokio::time::sleep(LOG_TAIL_PERIOD) => {}
                }
                let snapshot = {
                    let g = ring_c.lock().expect("ring lock");
                    g.iter().cloned().collect::<Vec<_>>()
                };
                if let Some(last) = snapshot.last().cloned() {
                    if Some(&last) != last_sent.as_ref() {
                        last_sent = Some(last.clone());
                        let _ = progress_c
                            .send(TaskEvent::Progress {
                                id: job_id,
                                pct: 50,
                                msg: format!("[log-tail] {last}"),
                            })
                            .await;
                    }
                }
            }
        })
    };

    let status = tokio::select! {
        biased;
        _ = token.cancelled() => {
            let _ = child.start_kill();
            match tokio::time::timeout(CANCEL_GRACE, child.wait()).await {
                Ok(_) => {}
                Err(_) => { let _ = child.kill().await; }
            }
            let _ = tokio::join!(drain_out, drain_err);
            tail_forwarder.abort();
            write_session_footer(&log_file, None).await?;
            return Err(LoaderError::Cancelled);
        }
        res = child.wait() => res.map_err(|e| LoaderError::ProfileWrite {
            path: cwd.display().to_string(),
            reason: format!("wait: {e}"),
        })?,
    };

    let _ = tokio::join!(drain_out, drain_err);
    tail_forwarder.abort();
    write_session_footer(&log_file, status.code()).await?;

    if !status.success() {
        let tail = {
            let g = ring.lock().expect("ring lock");
            g.iter().cloned().collect::<Vec<_>>().join("\n")
        };
        return Err(LoaderError::SubprocessExit {
            code: status.code().unwrap_or(-1),
            tail,
        });
    }
    Ok(())
}

async fn write_session_header(
    log: &Arc<tokio::sync::Mutex<tokio::fs::File>>,
    java_bin: &Path,
    jvm_args: &[String],
    args: &[String],
) -> Result<(), LoaderError> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let header = format!(
        "=== Loader install: {} {} {} — unix_ts={secs} ===\n",
        java_bin.display(),
        jvm_args.join(" "),
        args.join(" ")
    );
    let mut f = log.lock().await;
    f.write_all(header.as_bytes()).await.map_err(|e| LoaderError::ProfileWrite {
        path: "log file".into(),
        reason: format!("write header: {e}"),
    })?;
    f.flush().await.ok();
    Ok(())
}

async fn write_session_footer(
    log: &Arc<tokio::sync::Mutex<tokio::fs::File>>,
    code: Option<i32>,
) -> Result<(), LoaderError> {
    let footer = format!(
        "=== Exit code: {} ===\n",
        match code {
            Some(c) => c.to_string(),
            None => "cancelled".into(),
        }
    );
    let mut f = log.lock().await;
    f.write_all(footer.as_bytes()).await.map_err(|e| LoaderError::ProfileWrite {
        path: "log file".into(),
        reason: format!("write footer: {e}"),
    })?;
    f.flush().await.ok();
    Ok(())
}

async fn drain_pipe<R>(
    reader: R,
    log_file: Arc<tokio::sync::Mutex<tokio::fs::File>>,
    ring: Arc<StdMutex<VecDeque<String>>>,
    prefix: &'static str,
) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin + Send + 'static,
{
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        let stamped = format!("{prefix}{line}");
        // Append to log file
        {
            let mut f = log_file.lock().await;
            let _ = f.write_all(stamped.as_bytes()).await;
            let _ = f.write_all(b"\n").await;
        }
        // Push into ring (briefly held lock — no .await)
        {
            let mut g = ring.lock().expect("ring lock");
            if g.len() >= LOG_TAIL_LINES {
                g.pop_front();
            }
            g.push_back(stamped);
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_log_path(td: &TempDir) -> PathBuf {
        td.path().join("install.log")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_zero_exit_returns_ok() {
        let td = TempDir::new().unwrap();
        let log = make_log_path(&td);
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let result = run_installer(
            Path::new("/bin/sh"),
            &[],
            &["-c".into(), "echo hi".into()],
            td.path(),
            &log,
            tx,
            JobId(0),
            token,
        )
        .await;
        assert!(matches!(result, Ok(())));
        let body = tokio::fs::read_to_string(&log).await.unwrap();
        assert!(body.contains("[stdout] hi"), "log missing stdout line: {body}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_nonzero_exit_returns_subprocess_exit_with_tail() {
        let td = TempDir::new().unwrap();
        let log = make_log_path(&td);
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let result = run_installer(
            Path::new("/bin/sh"),
            &[],
            &["-c".into(), "echo bad >&2; exit 17".into()],
            td.path(),
            &log,
            tx,
            JobId(0),
            token,
        )
        .await;
        match result {
            Err(LoaderError::SubprocessExit { code, tail }) => {
                assert_eq!(code, 17);
                assert!(tail.contains("[stderr] bad"), "tail missing stderr: {tail}");
            }
            other => panic!("expected SubprocessExit, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_cancellation_kills_child_and_returns_cancelled() {
        let td = TempDir::new().unwrap();
        let log = make_log_path(&td);
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let token_c = token.clone();
        // Cancel 100ms in.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            token_c.cancel();
        });
        let start = std::time::Instant::now();
        let result = run_installer(
            Path::new("/bin/sh"),
            &[],
            &["-c".into(), "sleep 30".into()],
            td.path(),
            &log,
            tx,
            JobId(0),
            token,
        )
        .await;
        let elapsed = start.elapsed();
        assert!(matches!(result, Err(LoaderError::Cancelled)));
        assert!(elapsed < Duration::from_secs(7), "cancel took too long: {elapsed:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_session_header_and_footer_written_to_log() {
        let td = TempDir::new().unwrap();
        let log = make_log_path(&td);
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let _ = run_installer(
            Path::new("/bin/sh"),
            &[],
            &["-c".into(), "true".into()],
            td.path(),
            &log,
            tx,
            JobId(0),
            token,
        )
        .await;
        let body = tokio::fs::read_to_string(&log).await.unwrap();
        assert!(body.contains("=== Loader install:"), "header missing: {body}");
        assert!(body.contains("=== Exit code: 0 ==="), "footer missing: {body}");
    }
}
