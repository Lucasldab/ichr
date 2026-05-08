//! Integration-level smoke for `launcher::spawn::run_process`.
//!
//! Gated with `#[ignore]` because it spawns a real `java -version`.
//! Run via: `cargo test --test launch_smoke -- --ignored`.

use std::path::PathBuf;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use mineltui::launcher::spawn::{run_process, LaunchOutcome};

fn java_bin() -> PathBuf {
    std::env::var("MINELTUI_JAVA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("java"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn launch_smoke_java_version_exits_clean() {
    let td = TempDir::new().unwrap();
    let log = td.path().join("instances/smoke/logs/mineltui.log");
    let token = CancellationToken::new();
    // `java -version` writes to stderr and exits 0.
    let result = run_process(
        &java_bin(),
        &["-version".to_string()],
        "", // no main class — java stops after -version
        &[],
        td.path(),
        &log,
        token,
    )
    .await;
    // Some java installs put version info to stderr AND exit 0; others may
    // handle `-version` with no main class as an "invalid" argv. Accept
    // either cleanly-exit or LaunchFailed with captured output — what
    // matters is that drain captured SOMETHING and the function returned
    // without hanging.
    let log_contents = tokio::fs::read_to_string(&log).await.unwrap_or_default();
    match result {
        Ok(LaunchOutcome { duration_ms, .. }) => {
            assert!(duration_ms < 60_000, "java -version should exit quickly");
            assert!(
                !log_contents.is_empty(),
                "drain must capture java -version output; log was empty"
            );
        }
        Err(e) => {
            // LaunchFailed is acceptable (java may reject empty main-class);
            // what we MUST verify is that the drain captured output — i.e.,
            // the pipe-deadlock fix works.
            assert!(
                !log_contents.is_empty(),
                "drain must capture output even on non-zero exit; log was empty; err = {e:?}"
            );
        }
    }
}
