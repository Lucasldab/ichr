//! Live integration test for Modpack Import. `#[ignore]`-gated; requires internet.
//!
//! Pin: Adrenaline (modrinth.com/modpack/adrenaline)
//!   project_id : BYN9yKrV
//!   version_id : 5Po6DBDW  (1.32.0+1.21.4.fabric, pinned 2026-05-08)
//!   mc_version : 1.21.4
//!   loader     : fabric 0.17.3
//!   client_mods: 14 (of 19 total; 5 server-only filtered out)
//!
//! Run with:
//!   cargo nextest run --test modpack_live --run-ignored only
//! or:
//!   cargo test --test modpack_live -- --ignored --nocapture
//!
//! Refresh policy: re-pin if `cargo nextest run --test modpack_live --run-ignored only`
//! fails with 404 or schema drift. See 10-VALIDATION.md §Live-Test Pin for procedure.
//!
//! Pre-requisites: the test pre-fetches the MC 1.21.4 version JSON from Mojang
//! (one small HTTP call) and writes it to the TempDir so that `LoaderService`
//! can resolve the JRE for the Fabric install step without requiring a full MC
//! installation.  The JRE itself is auto-downloaded by `JavaService` using the
//! Mojang JRE manifest -- an additional ~300 MB on first run (cached in TempDir).

use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use ichr::modpack::service::ModpackService;
use ichr::persistence::paths::AppPaths;
use ichr::tasks::JobId;

// ─── Pin constants ─────────────────────────────────────────────────────────────

const PROJECT_ID: &str = "BYN9yKrV";
const VERSION_ID: &str = "5Po6DBDW";
const FILENAME: &str = "Adrenaline-1.32.0%2B1.21.4.fabric.mrpack";
const MC_VERSION: &str = "1.21.4";
const EXPECTED_PACK_NAME_SLUG: &str = "adrenaline";
const EXPECTED_MIN_MODS: usize = 1; // conservative floor; real pack has ~14 client mods

/// Mojang version JSON URL for MC 1.21.4 (pre-fetched for the JRE resolve step).
const MC_VERSION_JSON_URL: &str =
    "https://piston-meta.mojang.com/v1/packages/84f8dc068cffaff81793f49075bea51374c6a8ea/1.21.4.json";

/// End-to-end live import of a real Modrinth modpack.
///
/// Steps:
/// 1. Pre-fetch the Mojang version JSON for MC 1.21.4 and write it to the
///    TempDir so `LoaderService` can resolve the JRE without a full MC install.
/// 2. Download the `.mrpack` file from cdn.modrinth.com into the TempDir.
/// 3. Call `ModpackService::import_mrpack` -- this downloads 14 client mod JARs,
///    applies any overrides, and installs Fabric 0.17.3.
/// 4. Assert: instance manifest exists + at least one mod JAR installed.
///
/// Total download on first run: ~14 mod JARs + Fabric library JARs + JRE
/// (≈ 300 MB total; most is the JRE and Fabric API).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires internet -- downloads ~300 MB from cdn.modrinth.com, meta.fabricmc.net, and Mojang endpoints"]
async fn live_import_pinned_modpack() {
    let td = TempDir::new().expect("TempDir::new");
    let paths = AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    );

    let http = reqwest::Client::builder()
        .user_agent(ichr::mojang::client::USER_AGENT)
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("build http client");

    // ── Step 1: pre-fetch MC version JSON for JRE resolution ─────────────────
    // `LoaderService::install_loader` calls `JavaService::resolve_jre_for_mc_version_install`
    // which requires `paths.version_json("1.21.4")` to exist on disk.
    // We write just this one JSON file (≈ 50 KB) without downloading any actual
    // MC assets, so the loader install can resolve the correct JRE component name.
    {
        let json_path = paths.version_json(MC_VERSION);
        tokio::fs::create_dir_all(json_path.parent().unwrap())
            .await
            .expect("create versions dir");
        let resp = http
            .get(MC_VERSION_JSON_URL)
            .send()
            .await
            .expect("fetch MC version JSON");
        let status = resp.status();
        assert!(
            status.is_success(),
            "Mojang version JSON returned HTTP {status}; check network access"
        );
        let bytes = resp.bytes().await.expect("read version JSON body");
        tokio::fs::write(&json_path, &bytes)
            .await
            .expect("write MC version JSON");
    }

    // ── Step 2: download the .mrpack from Modrinth CDN ───────────────────────
    let mrpack_url =
        format!("https://cdn.modrinth.com/data/{PROJECT_ID}/versions/{VERSION_ID}/{FILENAME}");
    let mrpack_path = td.path().join("adrenaline.mrpack");

    let resp = http
        .get(&mrpack_url)
        .send()
        .await
        .expect("fetch .mrpack: check internet access and that the pinned URL is still valid");

    let status = resp.status();
    assert!(
        status.is_success(),
        "Modrinth CDN returned HTTP {status} for {mrpack_url}; \
         pin may have drifted -- update VERSION_ID in this test (see 10-VALIDATION.md)"
    );

    let bytes = resp.bytes().await.expect("read .mrpack body");
    tokio::fs::write(&mrpack_path, &bytes)
        .await
        .expect("write .mrpack to disk");

    // ── Step 3: import via ModpackService ────────────────────────────────────
    let svc = ModpackService::with_client(
        reqwest::Client::builder()
            .user_agent(ichr::mojang::client::USER_AGENT)
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("build svc client"),
    );

    let loader_svc = ichr::loader::service::LoaderService::new().expect("LoaderService::new");
    let java_svc = ichr::java::service::JavaService::new().expect("JavaService::new");
    let mojang_svc = ichr::mojang::client::MojangClient::new().expect("MojangClient::new");

    let (tx, mut rx) = mpsc::channel::<ichr::tasks::TaskEvent>(256);

    // Drain progress events in a background task so the channel never fills.
    let drain = tokio::spawn(async move {
        let mut labels: Vec<String> = Vec::new();
        while let Some(evt) = rx.recv().await {
            if let ichr::tasks::TaskEvent::Progress { msg, pct, .. } = evt {
                eprintln!("[live-test] {pct:3}% -- {msg}");
                labels.push(msg);
            }
        }
        labels
    });

    let token = CancellationToken::new();

    let result = svc
        .import_mrpack(
            &paths,
            &mrpack_path,
            &mojang_svc,
            &loader_svc,
            &java_svc,
            tx,
            token,
            JobId(0),
        )
        .await;

    let labels = drain.await.expect("drain task");

    let manifest = result.unwrap_or_else(|e| {
        panic!(
            "live import failed: {e:?}\n\
             Progress events: {labels:?}\n\
             Check internet access, CDN availability, and that the pin is still valid."
        )
    });

    // ── Step 4: assertions ───────────────────────────────────────────────────

    // instance.json must exist (atomicity gate written last)
    assert!(
        paths.instance_manifest(&manifest.slug).exists(),
        "instance.json must exist after successful live import"
    );

    // The slug must be derived from the pack name "Adrenaline"
    assert_eq!(
        manifest.slug, EXPECTED_PACK_NAME_SLUG,
        "slug must match expected pack name slug"
    );

    // At least one mod jar must be present in the mods directory
    let mods_dir = paths.instance_minecraft_dir(&manifest.slug).join("mods");
    let mod_count = if mods_dir.exists() {
        std::fs::read_dir(&mods_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "jar").unwrap_or(false))
            .count()
    } else {
        0
    };
    assert!(
        mod_count >= EXPECTED_MIN_MODS,
        "expected at least {EXPECTED_MIN_MODS} mod jar(s), found {mod_count} in {mods_dir:?}"
    );

    eprintln!(
        "[live-test result] import succeeded: slug={}, mods installed={}",
        manifest.slug, mod_count
    );
}
