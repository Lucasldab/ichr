//! End-to-end #[ignore]d test: install vanilla 1.20.4 + launch +
//! verify log file population. Requires network + Java + ~90 seconds.
//!
//! Run with: `cargo test --test launch_integration -- --ignored`

use std::path::PathBuf;
use std::time::Duration;

use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use mineltui::auth::AuthContext;
use mineltui::error::AppError;
use mineltui::install::install_version;
use mineltui::launcher::service::launch_instance;
use mineltui::mojang::client::MojangClient;
use mineltui::mojang::types::VersionEntry;
use mineltui::persistence::paths::AppPaths;
use mineltui::services::create_instance;
use mineltui::tasks::job::{JobId, TaskEvent};

fn paths_in(td: &TempDir) -> AppPaths {
    AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn test_end_to_end_launch_1_20_4() {
    let td = TempDir::new().unwrap();
    let paths = paths_in(&td);
    let mojang = MojangClient::new().unwrap();

    let manifest_cache: PathBuf = paths.cache_dir.join("manifest_v2.json");
    let manifest = mojang
        .fetch_manifest(&manifest_cache)
        .await
        .expect("fetch manifest");
    let entry: VersionEntry = manifest
        .versions
        .iter()
        .find(|v| v.id == "1.20.4")
        .cloned()
        .expect("1.20.4 in manifest");

    let inst = create_instance(&paths, "SmokeTest", "1.20.4").await.unwrap();

    let (tx, _rx) = mpsc::channel::<TaskEvent>(256);
    let token = CancellationToken::new();
    install_version(JobId(1), &paths, &mojang, tx.clone(), token.clone(), &inst.slug, &entry)
        .await
        .expect("install 1.20.4");

    assert!(
        paths.version_jar("1.20.4").exists(),
        "client.jar must exist after install"
    );

    // Cancel after 60 seconds so this test doesn't sit on Minecraft's title screen forever.
    let cancel = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        cancel.cancel();
    });

    let (launch_tx, _launch_rx) = mpsc::channel::<TaskEvent>(256);
    let result = launch_instance(
        &paths,
        &inst.slug,
        AuthContext::Offline { username: "TestUser".to_string() },
        None,
        launch_tx,
        token.clone(),
        JobId(2),
    )
    .await;

    // Valid terminal outcomes: Ok(duration_ms>0), or Err(Cancelled) from our 60s timer.
    // LaunchFailed would indicate a real startup crash.
    match &result {
        Ok(duration_ms) => {
            eprintln!("launch exited cleanly after {duration_ms} ms");
        }
        Err(AppError::Cancelled) => {
            eprintln!("launch cancelled after 60s as planned");
        }
        Err(e) => panic!("launch returned unexpected error: {e:?}"),
    }

    // Verify the log file captured Minecraft's boot output.
    let log_path: PathBuf = paths.instance_log_file(&inst.slug);
    let contents = tokio::fs::read_to_string(&log_path)
        .await
        .expect("read log file");
    assert!(!contents.is_empty(), "log file must be non-empty after launch");
    let signals = ["Setting user", "Minecraft", "Loading", "LWJGL", "Mojang"];
    let found = signals.iter().any(|s| contents.contains(s));
    assert!(
        found,
        "log file must contain a known Minecraft boot signal from {signals:?}; \
         first 500 bytes: {}",
        contents.chars().take(500).collect::<String>()
    );
}
