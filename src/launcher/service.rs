//! Launch orchestrator — loads an instance, composes the command, spawns
//! Minecraft, and updates the manifest on exit. Emits TaskEvents at
//! each step so the TUI progress indicator can track the launch.
//!
//! See `.planning/phases/03-launcher-process-and-offline-launch/03-RESEARCH.md`
//! §"System Architecture Diagram" for the flow.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::instance::store::{mark_launch_started, update_play_time};
use crate::instance::read_instance_manifest;
use crate::mojang::inherits::resolve_inherits;
use crate::mojang::rules::RuleContext;
use crate::mojang::types::VersionJson;
use crate::persistence::paths::AppPaths;
use crate::tasks::job::{JobId, TaskEvent};

use super::argfile::{argfile_path, write_argfile};
use super::command::compose;
use super::offline::offline_auth;
use super::spawn::run_process;

/// Phase 3 Java resolver: env var override, fall back to `"java"` from PATH.
///
/// Phase 5 replaces this with per-instance JRE management (auto-download
/// Mojang-blessed / Adoptium JRE keyed on the version's `javaVersion.component`
/// field).
pub fn resolve_java_bin() -> PathBuf {
    if let Ok(p) = std::env::var("MINELTUI_JAVA") {
        return PathBuf::from(p);
    }
    PathBuf::from("java")
}

/// Launch `slug` in offline mode under `username`. Emits `TaskEvent::Progress`
/// messages at each step to `tx`.
///
/// Returns the play duration in milliseconds on a clean exit.
/// Returns `AppError::Cancelled` if the `token` is cancelled during the game.
/// Returns `AppError::LaunchFailed { code, message }` on a non-zero JVM exit,
/// where `message` contains the ring-buffered log tail from `spawn::run_process`.
/// Returns `AppError::VersionNotInstalled { slug }` if the client jar is absent
/// (short-circuits before anything is spawned).
pub async fn launch_instance(
    paths: &AppPaths,
    slug: &str,
    username: &str,
    tx: mpsc::Sender<TaskEvent>,
    token: CancellationToken,
    job_id: JobId,
) -> Result<u64, AppError> {
    // Step 1 — load instance manifest
    send_progress(&tx, job_id, 1, "loading instance").await;
    let manifest = read_instance_manifest(paths, slug).await?;

    // Step 2 — verify client.jar present before we do anything expensive
    send_progress(&tx, job_id, 5, "checking version installed").await;
    let client_jar = paths.version_jar(&manifest.mc_version_id);
    if !tokio::fs::try_exists(&client_jar).await.unwrap_or(false) {
        return Err(AppError::VersionNotInstalled { slug: slug.to_string() });
    }

    // Step 3 — load root version JSON from disk (no network)
    send_progress(&tx, job_id, 10, "loading version JSON").await;
    let root_version = read_version_json_from_disk(paths, &manifest.mc_version_id).await?;

    // Step 4 — walk inheritsFrom chain from disk only; call pure-sync resolve_inherits
    send_progress(&tx, job_id, 15, "resolving inheritsFrom chain").await;
    let parents = collect_parents_from_disk(paths, &root_version).await?;
    let version = resolve_inherits(&root_version, &parents)?;

    // Step 5 — compose the LaunchCommand
    send_progress(&tx, job_id, 25, "composing command").await;
    let auth = offline_auth(username);
    let ctx = RuleContext::current();
    let java = resolve_java_bin();
    let cmd = compose(&version, &auth, paths, slug, &ctx, &java)?;

    // Step 6 — probe Java binary is invocable
    send_progress(&tx, job_id, 35, "checking java").await;
    probe_java(&java).await?;

    // Step 7 — on Windows write @argfile and replace jvm_args with the @-reference;
    //           on Linux pass jvm_args through unmodified
    send_progress(&tx, job_id, 40, "preparing argfile").await;
    let effective_jvm_args: Vec<String> = if cfg!(target_os = "windows") {
        let ap = argfile_path(paths, slug);
        write_argfile(&cmd.jvm_args, &ap)
            .map_err(|e| AppError::SpawnFailed(format!("write argfile: {e}")))?;
        vec![format!("@{}", ap.to_string_lossy())]
    } else {
        cmd.jvm_args.clone()
    };

    // Step 8 — mark launch started (sets last_played_at; play_time untouched until exit)
    send_progress(&tx, job_id, 45, "marking launch started").await;
    mark_launch_started(paths, slug).await?;

    // Step 9 — spawn Minecraft and wait
    send_progress(&tx, job_id, 50, "spawning Minecraft").await;
    let outcome = run_process(
        &cmd.java_bin,
        &effective_jvm_args,
        &cmd.main_class,
        &cmd.game_args,
        &paths.instance_minecraft_dir(slug),
        &paths.instance_log_file(slug),
        token,
    )
    .await;

    match outcome {
        Ok(launch_outcome) => {
            // Step 10 — clean exit: update play time and return duration
            send_progress(&tx, job_id, 100, "exited cleanly").await;
            update_play_time(paths, slug, launch_outcome.duration_ms).await?;
            Ok(launch_outcome.duration_ms)
        }
        Err(AppError::Cancelled) => {
            // Step 11 — cancelled: propagate without updating play time
            Err(AppError::Cancelled)
        }
        Err(e) => {
            // Step 12/13 — LaunchFailed or SpawnFailed: propagate as-is
            Err(e)
        }
    }
}

// ----- private helpers -------------------------------------------------------

/// Read a version JSON from `{versions_dir}/{id}/{id}.json`. Returns
/// `AppError::VersionNotInstalled` if the file is absent (the caller must
/// ensure the version was installed before launching).
async fn read_version_json_from_disk(
    paths: &AppPaths,
    version_id: &str,
) -> Result<VersionJson, AppError> {
    let json_path = paths.version_json(version_id);
    if !tokio::fs::try_exists(&json_path).await.unwrap_or(false) {
        return Err(AppError::VersionNotInstalled {
            slug: version_id.to_string(),
        });
    }
    let raw = tokio::fs::read_to_string(&json_path).await?;
    let v: VersionJson = serde_json::from_str(&raw)?;
    Ok(v)
}

/// Walk `root.inherits_from` from disk ONLY — no network. Populates a
/// `HashMap<id, VersionJson>` for every ancestor in the chain and returns it
/// for `resolve_inherits` (which is pure-sync).
///
/// Cycle protection: the loop stops if it sees a `parent_id` already in the
/// map; `resolve_inherits` enforces MAX_INHERITS_DEPTH (= 3) as a hard cap.
///
/// If any parent JSON is absent from disk, returns `AppError::VersionNotInstalled`
/// — launch must not hit the network (Phase 2 install pre-fetched all parents).
async fn collect_parents_from_disk(
    paths: &AppPaths,
    root: &VersionJson,
) -> Result<HashMap<String, VersionJson>, AppError> {
    let mut parents: HashMap<String, VersionJson> = HashMap::new();
    let mut current = root.inherits_from.clone();
    while let Some(parent_id) = current {
        if parents.contains_key(&parent_id) {
            // Cycle detected — resolve_inherits will reject with InheritsFromCycle
            break;
        }
        let pv = read_version_json_from_disk(paths, &parent_id).await?;
        current = pv.inherits_from.clone();
        parents.insert(parent_id, pv);
    }
    Ok(parents)
}

/// Probe that the configured Java binary is invocable. Spawns
/// `<java> -version` with all stdio piped to null; any spawn error is
/// returned as `AppError::JavaNotFound` (user-actionable).
///
/// The child is killed immediately after a successful spawn — we only care
/// that the binary exists and the OS can exec it.
async fn probe_java(java: &Path) -> Result<(), AppError> {
    let mut cmd = tokio::process::Command::new(java);
    cmd.arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|_| AppError::JavaNotFound)?;
    // Kill immediately — we only checked that spawn succeeded.
    // T-03-04-04: prevent zombie by waiting after kill.
    let _ = child.kill().await;
    let _ = child.wait().await;
    Ok(())
}

/// Send a `TaskEvent::Progress` to `tx`. Failures are silently ignored —
/// a dropped receiver is a legitimate shutdown signal, not an error.
async fn send_progress(tx: &mpsc::Sender<TaskEvent>, id: JobId, pct: u8, msg: &str) {
    let _ = tx
        .send(TaskEvent::Progress {
            id,
            pct,
            msg: msg.to_string(),
        })
        .await;
}

// ----- tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::InstanceManifest;
    use crate::instance::store::write_instance_manifest;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn paths_in(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    #[tokio::test]
    async fn test_launch_returns_version_not_installed_when_jar_missing() {
        let td = TempDir::new().unwrap();
        let paths = paths_in(&td);
        let m = InstanceManifest::new("x".into(), "x".into(), "1.21.4".into());
        write_instance_manifest(&paths, &m).await.unwrap();
        // client.jar intentionally NOT created
        let (tx, _rx) = mpsc::channel::<TaskEvent>(16);
        let token = CancellationToken::new();
        let result = launch_instance(&paths, "x", "TestUser", tx, token, JobId(1)).await;
        assert!(
            matches!(result, Err(AppError::VersionNotInstalled { .. })),
            "expected VersionNotInstalled; got {result:?}"
        );
    }

    #[test]
    fn test_resolve_java_bin_respects_env_var() {
        let prior = std::env::var("MINELTUI_JAVA").ok();
        std::env::set_var("MINELTUI_JAVA", "/custom/java");
        let got = resolve_java_bin();
        assert_eq!(got, PathBuf::from("/custom/java"));
        match prior {
            Some(v) => std::env::set_var("MINELTUI_JAVA", v),
            None => std::env::remove_var("MINELTUI_JAVA"),
        }
    }

    #[test]
    fn test_resolve_java_bin_falls_back_to_path() {
        let prior = std::env::var("MINELTUI_JAVA").ok();
        std::env::remove_var("MINELTUI_JAVA");
        let got = resolve_java_bin();
        assert_eq!(got, PathBuf::from("java"));
        if let Some(v) = prior {
            std::env::set_var("MINELTUI_JAVA", v);
        }
    }
}
