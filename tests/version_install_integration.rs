//! Live integration test — gated by #[ignore]. Run manually:
//!
//!     cargo test --test version_install_integration -- --ignored
//!
//! Downloads ~50-100 MB from piston-meta.mojang.com + resources.download.minecraft.net.

use mineltui::install::install_version;
use mineltui::mojang::client::MojangClient;
use mineltui::persistence::AppPaths;
use mineltui::tasks::{JobId, TaskEvent};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_install_1_20_4_live() {
    let td = tempdir().expect("tempdir");
    let paths = AppPaths::with_roots(
        td.path().join("data"),
        td.path().join("config"),
        td.path().join("cache"),
    );

    let client = MojangClient::new().expect("MojangClient::new");
    let manifest_cache = paths.cache_dir.join("manifest_v2.json");
    let manifest = client
        .fetch_manifest(&manifest_cache)
        .await
        .expect("fetch manifest");

    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == "1.20.4")
        .expect("1.20.4 in manifest")
        .clone();

    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let token = CancellationToken::new();

    install_version(JobId(1), &paths, &client, tx, token, "live-test", &entry)
        .await
        .expect("install_version ok");

    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    let progress_count = events
        .iter()
        .filter(|e| matches!(e, TaskEvent::Progress { .. }))
        .count();
    assert!(
        progress_count >= 4,
        "expected >= 4 progress events, got {progress_count}"
    );

    let jar = paths.version_jar("1.20.4");
    assert!(jar.exists(), "client.jar missing at {jar:?}");

    let json = paths.version_json("1.20.4");
    assert!(json.exists(), "version JSON missing at {json:?}");

    let objects_dir = paths.assets_dir().join("objects");
    let mut object_count = 0usize;
    let mut stack = vec![objects_dir];
    while let Some(dir) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&dir).await.unwrap();
        while let Some(e) = rd.next_entry().await.unwrap() {
            if e.file_type().await.unwrap().is_dir() {
                stack.push(e.path());
            } else {
                object_count += 1;
            }
        }
    }
    assert!(object_count >= 100, "too few asset objects: {object_count}");

    let libs_dir = paths.libraries_dir();
    let mut lib_count = 0usize;
    let mut stack = vec![libs_dir];
    while let Some(dir) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&dir).await.unwrap();
        while let Some(e) = rd.next_entry().await.unwrap() {
            if e.file_type().await.unwrap().is_dir() {
                stack.push(e.path());
            } else {
                lib_count += 1;
            }
        }
    }
    assert!(lib_count >= 10, "too few libraries: {lib_count}");
}
