//! End-to-end #[ignore]d test: install vanilla 1.20.4 + launch +
//! verify log file population. Requires network + Java + ~90 seconds.
//!
//! Run with: `cargo test --test launch_integration -- --ignored`

use std::path::PathBuf;
use std::time::Duration;

use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use ichr::auth::AuthContext;
use ichr::domain::instance::InstanceManifest;
use ichr::error::AppError;
use ichr::install::install_version;
use ichr::java::service::JavaService;
use ichr::java::types::JavaRuntimeId;
use ichr::launcher::service::launch_instance;
use ichr::mojang::client::MojangClient;
use ichr::mojang::types::VersionEntry;
use ichr::persistence::paths::AppPaths;
use ichr::services::create_instance;
use ichr::tasks::job::{JobId, TaskEvent};

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

    let inst = create_instance(&paths, "SmokeTest", "1.20.4")
        .await
        .unwrap();

    let (tx, _rx) = mpsc::channel::<TaskEvent>(256);
    let token = CancellationToken::new();
    install_version(
        JobId(1),
        &paths,
        &mojang,
        tx.clone(),
        token.clone(),
        &inst.slug,
        &entry,
    )
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

    let java_service = JavaService::new().expect("JavaService::new");

    let (launch_tx, _launch_rx) = mpsc::channel::<TaskEvent>(256);
    let result = launch_instance(
        &paths,
        &inst.slug,
        AuthContext::Offline {
            username: "TestUser".to_string(),
        },
        None,
        &java_service,
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
    assert!(
        !contents.is_empty(),
        "log file must be non-empty after launch"
    );
    let signals = ["Setting user", "Minecraft", "Loading", "LWJGL", "Mojang"];
    let found = signals.iter().any(|s| contents.contains(s));
    assert!(
        found,
        "log file must contain a known Minecraft boot signal from {signals:?}; \
         first 500 bytes: {}",
        contents.chars().take(500).collect::<String>()
    );
}

/// Regression test: launch_instance returns JavaMismatch BEFORE spawning any
/// process when the instance's java_override has a major version that does not
/// meet the version JSON's requirement.
#[tokio::test]
async fn test_launch_fails_early_on_java_mismatch() {
    use ichr::instance::store::write_instance_manifest;
    use ichr::mojang::types::{AssetIndex, JavaVersion, VersionDownloads, VersionJson};

    let td = TempDir::new().unwrap();
    let paths = paths_in(&td);

    // Create a fake java binary that is "on disk" so the System override path exists.
    let fake_java = td.path().join("java8");
    std::fs::write(&fake_java, b"#!/bin/sh\nexec true\n").unwrap();

    // Write instance manifest with System override claiming Java 8.
    let mut manifest = InstanceManifest::new("mismatch".into(), "mismatch".into(), "1.21.4".into());
    manifest.java_override = Some(JavaRuntimeId::System {
        path: fake_java.clone(),
        major_version: 8,
    });
    write_instance_manifest(&paths, &manifest).await.unwrap();

    // Create a minimal client.jar so the VersionNotInstalled check passes.
    let version_dir = paths.versions_dir().join("1.21.4");
    tokio::fs::create_dir_all(&version_dir).await.unwrap();
    tokio::fs::write(paths.version_jar("1.21.4"), b"fake")
        .await
        .unwrap();

    // Write a minimal version JSON requiring Java 21.
    // Note: VersionJson::asset_index/assets/downloads are Option<_> after
    // Phase 8.3 (loader JSONs lack them and inherit from vanilla); vanilla
    // version JSONs declare them, so we wrap in Some(...) at construction.
    let version = VersionJson {
        id: "1.21.4".into(),
        version_type: "release".into(),
        main_class: "net.minecraft.client.main.Main".into(),
        asset_index: Some(AssetIndex {
            id: "17".into(),
            sha1: "aaaa".into(),
            size: 0,
            total_size: 0,
            url: "http://example.com/assets.json".into(),
        }),
        assets: Some("17".into()),
        downloads: Some(VersionDownloads::default()),
        libraries: vec![],
        java_version: Some(JavaVersion {
            component: "java-runtime-delta".into(),
            major_version: 21,
        }),
        logging: None,
        compliance_level: None,
        minimum_launcher_version: None,
        release_time: "2024-12-03T00:00:00Z".into(),
        time: "2024-12-03T00:00:00Z".into(),
        arguments: None,
        minecraft_arguments: None,
        inherits_from: None,
    };
    let version_json_path = paths.version_json("1.21.4");
    tokio::fs::write(&version_json_path, serde_json::to_string(&version).unwrap())
        .await
        .unwrap();

    let java_service = JavaService::new().expect("JavaService::new");

    // Ensure ICHR_JAVA is not set — we want the System override path taken.
    let _prior = std::env::var("ICHR_JAVA").ok();
    std::env::remove_var("ICHR_JAVA");

    let (tx, _rx) = mpsc::channel::<TaskEvent>(16);
    let token = CancellationToken::new();
    let result = launch_instance(
        &paths,
        "mismatch",
        AuthContext::Offline {
            username: "TestUser".to_string(),
        },
        None,
        &java_service,
        tx,
        token,
        JobId(1),
    )
    .await;

    // Restore ICHR_JAVA if it was set.
    if let Some(v) = _prior {
        std::env::set_var("ICHR_JAVA", v);
    }

    assert!(
        matches!(
            result,
            Err(AppError::JavaMismatch {
                required: 21,
                found: 8,
                ..
            })
        ),
        "expected JavaMismatch{{required:21,found:8}}; got: {result:?}"
    );
}
