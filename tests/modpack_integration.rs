//! End-to-end integration tests for `ModpackService::import_mrpack`.
//!
//! These tests build synthetic `.mrpack` fixtures via the helpers in
//! `tests/fixtures/modpacks/build.rs` and serve mod downloads through
//! `httpmock`.  No real network access required.
//!
//! Run with: `cargo nextest run --test modpack_integration`

use httpmock::prelude::*;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use ichr::modpack::service::ModpackService;
use ichr::modpack::ModpackError;
use ichr::mojang::client::MojangClient;
use ichr::persistence::paths::AppPaths;
use ichr::tasks::JobId;

// ─── Include fixture builder ───────────────────────────────────────────────────

mod fixtures {
    include!("fixtures/modpacks/build.rs");
}
use fixtures::{build_minimal_mrpack, build_mrpack_with_path_traversal, ModEntry};

// ─── Shared helpers ────────────────────────────────────────────────────────────

fn make_paths(tmp: &TempDir) -> AppPaths {
    AppPaths::with_roots(
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
    )
}

fn make_svc() -> ModpackService {
    let http = reqwest::Client::builder().build().expect("reqwest client");
    ModpackService::with_client(http)
}

async fn make_services(
    paths: &AppPaths,
) -> (
    ichr::loader::service::LoaderService,
    ichr::java::service::JavaService,
    MojangClient,
) {
    let loader_svc = ichr::loader::service::LoaderService::new().expect("LoaderService::new");
    let java_svc = ichr::java::service::JavaService::new().expect("JavaService::new");
    let mojang_svc = MojangClient::new().expect("MojangClient::new");
    // GAP-10-A fix: pre-touch vanilla version JSON so Step 2.5's existence check
    // skips the live Mojang manifest fetch (test fixtures use mc_version "1.20.4").
    pre_install_vanilla_version_json(paths, "1.20.4").await;
    (loader_svc, java_svc, mojang_svc)
}

/// GAP-10-A test helper: pre-touch the vanilla MC version JSON so that
/// `ModpackService::import_mrpack` Step 2.5 (added by Phase 10.1) skips the
/// live Mojang manifest fetch. Without this, integration tests would attempt
/// network access and fail offline.
async fn pre_install_vanilla_version_json(paths: &AppPaths, mc_version: &str) {
    let json_path = paths.version_json(mc_version);
    if let Some(parent) = json_path.parent() {
        tokio::fs::create_dir_all(parent).await.unwrap();
    }
    tokio::fs::write(&json_path, b"{}").await.unwrap();
}

// ─── Test 1: end_to_end_import_minimal_pack ───────────────────────────────────

/// Happy path: a vanilla pack with one required mod, override files, and
/// client-override files.  Validates that after `import_mrpack`:
/// - the returned manifest has the expected slug
/// - `instance.json` exists (atomicity gate was written)
/// - the mod jar exists at `<instance>/.minecraft/mods/<filename>`
/// - `overrides/config/test.txt` → `.minecraft/config/test.txt` ("from overrides")
/// - `client-overrides/options.txt` → `.minecraft/options.txt` ("from client-overrides")
#[tokio::test]
async fn end_to_end_import_minimal_pack() {
    let server = MockServer::start();
    let mod_body = b"fake-mod-bytes-required";

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/minimal-mod.jar");
        then.status(200).body(&mod_body[..]);
    });

    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    let mrpack = tmp.path().join("minimal.mrpack");

    build_minimal_mrpack(
        &mrpack,
        "Minimal Test Pack",
        "1.20.4",
        None, // vanilla -- no modloader
        &[ModEntry {
            path: "mods/minimal-mod.jar",
            payload: mod_body,
            download_url: &server.url("/minimal-mod.jar"),
            env_client: "required",
            env_server: "unsupported",
        }],
        true, // include overrides + client-overrides
    );

    let svc = make_svc();
    let (loader_svc, java_svc, mojang_svc) = make_services(&paths).await;
    let (tx, _rx) = mpsc::channel(64);
    let token = CancellationToken::new();

    let result = svc
        .import_mrpack(
            &paths,
            &mrpack,
            &mojang_svc,
            &loader_svc,
            &java_svc,
            tx,
            token,
            JobId(1),
        )
        .await;

    let manifest = result.expect("end-to-end import must succeed");

    // Slug is derived from "Minimal Test Pack" → "minimal-test-pack"
    assert_eq!(manifest.slug, "minimal-test-pack");

    // instance.json must exist (atomicity gate written last)
    assert!(
        paths.instance_manifest(&manifest.slug).exists(),
        "instance.json must exist after successful import"
    );

    // Mod jar must exist
    let mod_jar = paths.instance_mod_file(&manifest.slug, "minimal-mod.jar");
    assert!(
        mod_jar.exists(),
        "mod jar must exist: {}",
        mod_jar.display()
    );
    let mod_bytes = std::fs::read(&mod_jar).unwrap();
    assert_eq!(
        mod_bytes, mod_body,
        "mod jar bytes must match fixture payload"
    );

    // overrides/config/test.txt → .minecraft/config/test.txt
    let config_txt = paths
        .instance_minecraft_dir(&manifest.slug)
        .join("config/test.txt");
    assert!(config_txt.exists(), "override config/test.txt must exist");
    let config_content = std::fs::read_to_string(&config_txt).unwrap();
    assert_eq!(config_content, "from overrides");

    // client-overrides/options.txt → .minecraft/options.txt
    let options_txt = paths
        .instance_minecraft_dir(&manifest.slug)
        .join("options.txt");
    assert!(
        options_txt.exists(),
        "client-override options.txt must exist"
    );
    let options_content = std::fs::read_to_string(&options_txt).unwrap();
    assert_eq!(options_content, "from client-overrides");
}

// ─── Test 2: import_atomicity_failure_cleans_dir ──────────────────────────────

/// When the mod download returns HTTP 500, `import_mrpack` must return `Err`
/// and the instance directory must not exist (atomicity invariant).
#[tokio::test]
async fn import_atomicity_failure_cleans_dir() {
    let server = MockServer::start();

    // 500 → download fails → inner block errors → cleanup fires
    let _mock = server.mock(|when, then| {
        when.method(GET).path("/fail-mod.jar");
        then.status(500).body(b"server error");
    });

    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    let mrpack = tmp.path().join("fail.mrpack");

    build_minimal_mrpack(
        &mrpack,
        "Fail Pack",
        "1.20.4",
        None,
        &[ModEntry {
            path: "mods/fail-mod.jar",
            payload: b"some bytes",
            download_url: &server.url("/fail-mod.jar"),
            env_client: "required",
            env_server: "unsupported",
        }],
        false,
    );

    let svc = make_svc();
    let (loader_svc, java_svc, mojang_svc) = make_services(&paths).await;
    let (tx, _rx) = mpsc::channel(64);
    let token = CancellationToken::new();

    let result = svc
        .import_mrpack(
            &paths,
            &mrpack,
            &mojang_svc,
            &loader_svc,
            &java_svc,
            tx,
            token,
            JobId(2),
        )
        .await;

    assert!(
        result.is_err(),
        "failed download must return Err: {result:?}"
    );

    // Atomicity: instance dir must not exist
    let slug = "fail-pack";
    assert!(
        !paths.instance_dir(slug).exists(),
        "instance dir must be removed after failure: {}",
        paths.instance_dir(slug).display()
    );
    assert!(
        !paths.instance_manifest(slug).exists(),
        "instance.json must not exist after cleanup"
    );
}

// ─── Test 3: import_pre_cancel_cleans_state ───────────────────────────────────

/// A pre-cancelled token causes `import_mrpack` to return `Err(Cancelled)` at
/// Step 1 (before creating the instance directory).  No instance dir must exist.
#[tokio::test]
async fn import_pre_cancel_cleans_state() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    let mrpack = tmp.path().join("cancel.mrpack");

    // Fixture uses a valid CDN URL but the token is pre-cancelled so no HTTP call
    // is ever made.
    build_minimal_mrpack(
        &mrpack,
        "Cancel Pack",
        "1.20.4",
        None,
        &[ModEntry {
            path: "mods/cancel-mod.jar",
            payload: b"x",
            download_url: "https://cdn.modrinth.com/fake-cancel.jar",
            env_client: "required",
            env_server: "unsupported",
        }],
        false,
    );

    let svc = make_svc();
    let (loader_svc, java_svc, mojang_svc) = make_services(&paths).await;
    let (tx, _rx) = mpsc::channel(64);

    // Cancel BEFORE calling import_mrpack
    let token = CancellationToken::new();
    token.cancel();

    let result = svc
        .import_mrpack(
            &paths,
            &mrpack,
            &mojang_svc,
            &loader_svc,
            &java_svc,
            tx,
            token,
            JobId(3),
        )
        .await;

    assert!(
        matches!(result, Err(ModpackError::Cancelled)),
        "pre-cancel must return Err(Cancelled): {result:?}"
    );

    // No instance directory should have been created
    let slug = "cancel-pack";
    assert!(
        !paths.instance_dir(slug).exists(),
        "pre-cancel must not create any instance dir"
    );
}

// ─── Test 4: import_with_unsupported_client_skips_files ──────────────────────

/// A pack with two files:
/// - file 1: `env.client = "required"` → must be downloaded (mock hit = 1)
/// - file 2: `env.client = "unsupported"` → must be skipped (mock hit = 0)
///
/// After import, only 1 `.jar` file exists in the mods directory.
#[tokio::test]
async fn import_with_unsupported_client_skips_files() {
    let server = MockServer::start();
    let required_body = b"required-mod-bytes";

    let mock_required = server.mock(|when, then| {
        when.method(GET).path("/client-req.jar");
        then.status(200).body(&required_body[..]);
    });

    // The unsupported file must never be requested
    let mock_unsupported = server.mock(|when, then| {
        when.method(GET).path("/server-only.jar");
        then.status(200).body(b"server-only bytes");
    });

    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    let mrpack = tmp.path().join("filter.mrpack");

    build_minimal_mrpack(
        &mrpack,
        "Filter Test Pack",
        "1.20.4",
        None,
        &[
            ModEntry {
                path: "mods/client-req.jar",
                payload: required_body,
                download_url: &server.url("/client-req.jar"),
                env_client: "required",
                env_server: "unsupported",
            },
            ModEntry {
                path: "mods/server-only.jar",
                payload: b"server-only bytes",
                download_url: &server.url("/server-only.jar"),
                env_client: "unsupported",
                env_server: "required",
            },
        ],
        false,
    );

    let svc = make_svc();
    let (loader_svc, java_svc, mojang_svc) = make_services(&paths).await;
    let (tx, _rx) = mpsc::channel(64);
    let token = CancellationToken::new();

    let result = svc
        .import_mrpack(
            &paths,
            &mrpack,
            &mojang_svc,
            &loader_svc,
            &java_svc,
            tx,
            token,
            JobId(4),
        )
        .await;

    let manifest = result.expect("filter-test import must succeed");

    // Required mod was downloaded exactly once; unsupported was never hit
    mock_required.assert_calls(1);
    mock_unsupported.assert_calls(0);

    // Only 1 .jar in the mods directory
    let mods_dir = paths.instance_minecraft_dir(&manifest.slug).join("mods");
    let jars: Vec<_> = std::fs::read_dir(&mods_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "jar").unwrap_or(false))
        .collect();
    assert_eq!(jars.len(), 1, "exactly 1 mod jar must exist; got: {jars:?}");
    assert!(
        jars[0].file_name().to_str().unwrap().contains("client-req"),
        "only client-req.jar must be present in mods/"
    );
}

// ─── Test 5: import_with_path_traversal_in_overrides_skips_safely ────────────

/// A `.mrpack` whose `overrides/` directory contains a path-traversal entry
/// (`overrides/../../etc/passwd`).  After import, the traversal victim path must
/// NOT exist anywhere outside the instance directory, proving that
/// `safe_zip::safe_extract_path` in `apply_overrides` wires through the full
/// service pipeline.
#[tokio::test]
async fn import_with_path_traversal_in_overrides_skips_safely() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    let mrpack = tmp.path().join("traversal.mrpack");

    // Build the adversarial fixture (no mods, just the malicious override entry)
    build_mrpack_with_path_traversal(&mrpack, "1.20.4");

    let svc = make_svc();
    let (loader_svc, java_svc, mojang_svc) = make_services(&paths).await;
    let (tx, _rx) = mpsc::channel(64);
    let token = CancellationToken::new();

    // Import must succeed (traversal entries are silently skipped, not errored)
    let result = svc
        .import_mrpack(
            &paths,
            &mrpack,
            &mojang_svc,
            &loader_svc,
            &java_svc,
            tx,
            token,
            JobId(5),
        )
        .await;

    let manifest = result.expect("traversal-skipping import must succeed");

    // The traversal path must NOT exist inside the instance directory
    let minecraft_dir = paths.instance_minecraft_dir(&manifest.slug);

    // Construct what an unsafe extractor would have written relative to .minecraft/
    // e.g. "<instance>/.minecraft/../../etc/passwd" → resolves up to some dir
    let traversal_target = minecraft_dir.join("../../etc/passwd");
    let traversal_canonical = std::fs::canonicalize(&traversal_target);

    // The traversal target either doesn't exist (most platforms) or exists but
    // must NOT have our test content (if the real /etc/passwd exists).
    if let Ok(canonical) = traversal_canonical {
        // The file exists (e.g. /etc/passwd on Linux) but must NOT contain our
        // test content -- proves we did not overwrite it.
        if let Ok(content) = std::fs::read_to_string(&canonical) {
            assert!(
                !content.contains("root:x:0:0:root:/root:/bin/bash"),
                "traversal entry content must not be present at {}: content = {:?}",
                canonical.display(),
                &content[..content.len().min(200)]
            );
        }
    }

    // The .minecraft/ directory must not contain any file named "passwd"
    // at the expected traversal depth.
    let inner_etc = minecraft_dir.join("etc");
    if inner_etc.exists() {
        let passwd_inside = inner_etc.join("passwd");
        assert!(
            !passwd_inside.exists(),
            "traversal must not place 'passwd' inside the instance dir at {}",
            passwd_inside.display()
        );
    }

    // The instance itself must have been created normally (no overflow into
    // temp dir root from the traversal).
    assert!(
        paths.instance_manifest(&manifest.slug).exists(),
        "instance.json must exist -- import succeeded despite traversal attempt"
    );
}
