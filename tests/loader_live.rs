//! Live Fabric + Quilt install smoke tests — gated by `#[ignore]`.
//! Run with: `cargo test --test loader_live -- --ignored --nocapture`
//!
//! Hits the real meta APIs and downloads real loader libraries. Requires
//! internet access. Each test takes ~10-30 seconds depending on bandwidth.

use mineltui::domain::InstanceManifest;
use mineltui::loader::maven::maven_coord_to_path;
use mineltui::loader::service::LoaderService;
use mineltui::loader::types::LoaderType;
use mineltui::persistence::paths::AppPaths;
use mineltui::tasks::JobId;
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

async fn write_vanilla_manifest(paths: &AppPaths, slug: &str, mc: &str) {
    let m = InstanceManifest::new(slug.into(), slug.into(), mc.into());
    mineltui::instance::store::write_instance_manifest(paths, &m)
        .await
        .unwrap();
}

fn make_progress_drain() -> mpsc::Sender<mineltui::tasks::TaskEvent> {
    let (tx, mut rx) = mpsc::channel::<mineltui::tasks::TaskEvent>(64);
    tokio::spawn(async move {
        while let Some(evt) = rx.recv().await {
            if let mineltui::tasks::TaskEvent::Progress { pct, msg, .. } = evt {
                println!("[loader_live] progress: {pct:>3}%  {msg}");
            }
        }
    });
    tx
}

#[tokio::test]
#[ignore = "requires internet access — see module docs"]
async fn live_fabric_install_1_21_4() {
    let td = TempDir::new().unwrap();
    let paths = make_paths(&td);
    let slug = "fabric-live";
    let mc = "1.21.4";
    let loader_version = "0.16.9";

    write_vanilla_manifest(&paths, slug, mc).await;
    let svc = LoaderService::new().expect("LoaderService::new");
    let progress = make_progress_drain();
    let token = CancellationToken::new();

    println!("[loader_live] starting Fabric install: mc={mc}, loader={loader_version}");
    // JobId(0) — JobId is a tuple struct with no Default impl; 0 is the sentinel for ad-hoc callers.
    let job_id = JobId(0);
    svc.install_loader(
        &paths,
        slug,
        mc,
        LoaderType::Fabric,
        loader_version,
        progress,
        token,
        job_id,
    )
    .await
    .expect("install_loader Fabric should succeed");

    let expected_id = format!("fabric-loader-{loader_version}-{mc}");
    let vjson = paths.version_json(&expected_id);
    assert!(vjson.is_file(), "version JSON missing at {}", vjson.display());

    // Manifest now has loader set with the profile.id verbatim (Pitfall 7)
    let m = mineltui::instance::store::read_instance_manifest(&paths, slug)
        .await
        .unwrap();
    let loader = m.loader.expect("manifest.loader should be set");
    assert_eq!(
        loader.version_id, expected_id,
        "version_id must match profile.id verbatim (Pitfall 7)"
    );
    assert_eq!(loader.version, loader_version);

    // Primary loader JAR must exist in libraries/
    let primary_path =
        maven_coord_to_path(&format!("net.fabricmc:fabric-loader:{loader_version}")).unwrap();
    let lib = paths.library_path(&primary_path);
    assert!(
        lib.is_file(),
        "primary fabric-loader jar missing at {}",
        lib.display()
    );

    println!("[loader_live] Fabric SUCCESS — version_id={expected_id}");
}

#[tokio::test]
#[ignore = "requires internet access — see module docs"]
async fn live_quilt_install_1_21_4() {
    let td = TempDir::new().unwrap();
    let paths = make_paths(&td);
    let slug = "quilt-live";
    let mc = "1.21.4";

    write_vanilla_manifest(&paths, slug, mc).await;
    let svc = LoaderService::new().expect("LoaderService::new");

    // Pick the newest Quilt version dynamically — Quilt's beta release churns.
    // Fetching the list avoids pinning a specific version that may be removed upstream.
    let versions = svc
        .list_loader_versions(LoaderType::Quilt, mc)
        .await
        .expect("list_loader_versions Quilt");
    assert!(!versions.is_empty(), "Quilt loader list should not be empty");
    let loader_version = versions[0].version.clone();
    println!("[loader_live] picked Quilt loader version: {loader_version}");

    let progress = make_progress_drain();
    let token = CancellationToken::new();

    // JobId(0) — JobId is a tuple struct with no Default impl; 0 is the sentinel for ad-hoc callers.
    let job_id = JobId(0);
    svc.install_loader(
        &paths,
        slug,
        mc,
        LoaderType::Quilt,
        &loader_version,
        progress,
        token,
        job_id,
    )
    .await
    .expect("install_loader Quilt should succeed");

    let m = mineltui::instance::store::read_instance_manifest(&paths, slug)
        .await
        .unwrap();
    let loader = m.loader.expect("manifest.loader should be set");
    assert!(
        loader.version_id.starts_with("quilt-loader-"),
        "version_id should be quilt-loader-*: {}",
        loader.version_id
    );
    assert_eq!(loader.version, loader_version);

    // The version JSON file must exist at the directory matching version_id verbatim (Pitfall 7)
    let vjson = paths.version_json(&loader.version_id);
    assert!(
        vjson.is_file(),
        "Quilt version JSON missing at {}",
        vjson.display()
    );

    // The primary loader JAR must exist in libraries/
    let primary_path =
        maven_coord_to_path(&format!("org.quiltmc:quilt-loader:{loader_version}")).unwrap();
    let lib = paths.library_path(&primary_path);
    assert!(
        lib.is_file(),
        "primary quilt-loader jar missing at {}",
        lib.display()
    );

    println!(
        "[loader_live] Quilt SUCCESS — version_id={}",
        loader.version_id
    );
}
