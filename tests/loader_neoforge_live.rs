//! Live NeoForge install smoke test — `#[ignore]`-gated.
//!
//! Run with: `cargo test --test loader_neoforge_live -- --ignored --nocapture`
//!
//! Hits real endpoints:
//!   - https://piston-meta.mojang.com (Mojang version manifest, JRE manifest)
//!   - https://launcher.mojang.com (Mojang JRE binaries)
//!   - https://maven.neoforged.net (NeoForge maven-metadata.xml + installer JAR)
//!   - https://piston-data.mojang.com / libraries.minecraft.net (vanilla libs)
//!   - NeoForge libraries Maven mirrors
//!
//! Total runtime ~60-180s on a fast connection; downloads ~150MB.

use std::sync::Arc;

use mineltui::domain::instance::{InstanceManifest, ModloaderKind};
use mineltui::install::version_installer::install_version;
use mineltui::java::service::JavaService;
use mineltui::loader::service::LoaderService;
use mineltui::loader::types::LoaderType;
use mineltui::mojang::client::MojangClient;
use mineltui::persistence::paths::AppPaths;
use mineltui::tasks::{JobId, TaskEvent};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn make_paths(td: &TempDir) -> AppPaths {
    AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    )
}

fn make_progress_drain(label: &'static str) -> mpsc::Sender<TaskEvent> {
    let (tx, mut rx) = mpsc::channel::<TaskEvent>(256);
    tokio::spawn(async move {
        while let Some(evt) = rx.recv().await {
            if let TaskEvent::Progress { pct, msg, .. } = evt {
                println!("[{label}] progress: {pct:>3}%  {msg}");
            }
        }
    });
    tx
}

#[tokio::test]
#[ignore = "requires internet access — see module docs"]
async fn live_neoforge_install_1_21_4() {
    let td = TempDir::new().expect("tempdir");
    let paths = make_paths(&td);
    let slug = "neoforge-live";
    let mc = "1.21.4";

    // 1) Write vanilla InstanceManifest
    let m = InstanceManifest::new(slug.into(), slug.into(), mc.into());
    mineltui::instance::store::write_instance_manifest(&paths, &m)
        .await
        .expect("write_instance_manifest");

    // 2) Pre-fetch vanilla MC via the canonical install_version helper.
    //    Mirrors tests/version_install.rs:269 call shape exactly.
    println!("[neoforge_live] pre-fetching vanilla MC {mc}...");
    let mojang = MojangClient::new().expect("MojangClient::new");
    let manifest = mojang
        .fetch_manifest(&paths.cache_dir.join("manifest_v2.json"))
        .await
        .expect("fetch_manifest");
    let version_entry = manifest
        .versions
        .iter()
        .find(|v| v.id == mc)
        .expect("MC 1.21.4 present in version manifest")
        .clone();

    let (vt_tx, _vt_rx) = mpsc::channel::<TaskEvent>(256);
    let vanilla_token = CancellationToken::new();
    install_version(
        JobId(99),
        &paths,
        &mojang,
        vt_tx,
        vanilla_token,
        slug,
        &version_entry,
    )
    .await
    .expect("install_version vanilla MC 1.21.4");
    assert!(
        paths.version_json(mc).is_file(),
        "vanilla version JSON missing after install_version"
    );
    assert!(
        paths.version_jar(mc).is_file(),
        "vanilla client.jar missing after install_version"
    );

    // 3) Install JRE for MC 1.21.4 (Java 21 = java-runtime-delta)
    println!("[neoforge_live] installing Mojang JRE java-runtime-delta...");
    let java_svc = JavaService::new().expect("JavaService::new");
    let _ = java_svc
        .install_mojang(&paths, "java-runtime-delta")
        .await
        .expect("install_mojang java-runtime-delta");

    // 4) Resolve JRE path
    let jre_path = java_svc
        .resolve_jre_for_mc_version_install(&paths, mc)
        .await
        .expect("resolve_jre_for_mc_version_install");
    println!("[neoforge_live] using JRE: {}", jre_path.display());

    // 5) Pick NeoForge loader version dynamically
    let svc = Arc::new(LoaderService::new().expect("LoaderService::new"));
    let versions = svc
        .list_loader_versions(LoaderType::NeoForge, mc)
        .await
        .expect("list_loader_versions NeoForge");
    assert!(!versions.is_empty(), "NeoForge must publish loaders for MC {mc}");
    let loader_version = versions
        .iter()
        .find(|v| v.stable)
        .map(|v| v.version.clone())
        .unwrap_or_else(|| versions[0].version.clone());
    println!("[neoforge_live] picked NeoForge {loader_version}");

    // 6) Run install
    let progress = make_progress_drain("neoforge_live");
    let token = CancellationToken::new();
    svc.install_loader(
        &paths,
        slug,
        mc,
        LoaderType::NeoForge,
        &loader_version,
        &jre_path,
        progress,
        token,
        JobId(0),
    )
    .await
    .expect("install_loader NeoForge");

    // 7) Assertions
    let m = mineltui::instance::store::read_instance_manifest(&paths, slug)
        .await
        .expect("read manifest");
    let loader = m.loader.expect("manifest.loader must be Some");
    assert_eq!(loader.kind, ModloaderKind::NeoForge);
    assert_eq!(loader.version, loader_version);
    assert!(
        loader.version_id.starts_with("neoforge-"),
        "version_id should start with 'neoforge-' (got {})",
        loader.version_id
    );

    let vj = paths.version_json(&loader.version_id);
    assert!(vj.is_file(), "version JSON missing: {}", vj.display());

    // Staging dir cleaned
    let staging_dir = paths.data_dir.join("staging");
    if staging_dir.is_dir() {
        let mut entries = tokio::fs::read_dir(&staging_dir)
            .await
            .expect("read staging");
        assert!(
            entries.next_entry().await.expect("staging entry").is_none(),
            "staging dir not cleaned: {}",
            staging_dir.display()
        );
    }

    // At least one NeoForge library present
    let neoforge_libs_dir = paths.libraries_dir().join("net").join("neoforged");
    assert!(
        neoforge_libs_dir.is_dir(),
        "NeoForge libraries dir missing: {}",
        neoforge_libs_dir.display()
    );

    // Per-install log file present
    let instance_dir = paths.instance_dir(slug);
    let mut found_log = false;
    if let Ok(mut entries) = tokio::fs::read_dir(&instance_dir).await {
        while let Some(entry) = entries
            .next_entry()
            .await
            .expect("read instance dir")
        {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("loader-install-")
            {
                found_log = true;
                break;
            }
        }
    }
    assert!(
        found_log,
        "no loader-install-*.log file in {}",
        instance_dir.display()
    );

    println!(
        "[neoforge_live] SUCCESS — installed NeoForge {} (id={})",
        loader_version, loader.version_id
    );
}

/// GAP-7-B canary — exercises the LIVE JSON endpoint (no env override).
///
/// Capture command (re-run if upstream shape drifts):
///     curl -s 'https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge'
///
/// This test asserts SHAPE invariants only — never specific version counts
/// (NeoForge ships new builds weekly). If this test fails:
///   1. Re-run the capture command above and inspect the response shape.
///   2. If `isSnapshot` is missing or renamed, update
///      `src/loader/neoforge_meta.rs::list_loader_versions::VersionsResponse`.
///   3. Re-capture `tests/fixtures/neoforge_meta_versions.json` (Phase 7.1-01
///      docs the exact `python3 -c` trim command).
#[tokio::test]
#[ignore = "requires internet access — see module docs"]
async fn live_neoforge_meta_lists_versions() {
    use mineltui::loader::neoforge_meta::NeoForgeMetaClient;

    let client = NeoForgeMetaClient::new()
        .expect("NeoForgeMetaClient::new (no env override; production endpoints)");

    // The 1.21.4 prefix is a current MC version — production must list ≥1.
    let versions = client
        .list_loader_versions("1.21.4")
        .await
        .expect("live JSON endpoint must respond 200 with parseable body");

    assert!(
        !versions.is_empty(),
        "live endpoint returned 0 versions for MC 1.21.4 — endpoint shape may have drifted; \
         re-capture tests/fixtures/neoforge_meta_versions.json and re-verify the parser"
    );
    assert!(
        versions.iter().all(|v| v.version.starts_with("21.4.")),
        "filter post-condition broken: {:?}",
        versions.iter().map(|v| &v.version).collect::<Vec<_>>()
    );
    assert!(
        versions.iter().all(|v| !v.version.is_empty()),
        "no empty-string versions: {:?}",
        versions.iter().map(|v| &v.version).collect::<Vec<_>>()
    );

    println!("[neoforge_meta_live] {} versions for MC 1.21.4", versions.len());
    for v in versions.iter().take(5) {
        println!("[neoforge_meta_live]   {} (stable={})", v.version, v.stable);
    }
}
