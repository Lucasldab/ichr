//! End-to-end integration tests for PackService + drop_pack_from_path.
//!
//! These tests build synthetic resource pack + shader pack fixtures via the
//! helpers in `tests/fixtures/packs/build.rs` and serve Modrinth API responses
//! through `httpmock`.  No real network access required.
//!
//! Run with: `cargo nextest run --test packs_integration`

use httpmock::prelude::*;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use ichr::domain::instance::InstanceManifest;
use ichr::instance::store::write_instance_manifest;
use ichr::mods::modrinth::ModrinthClient;
use ichr::packs::error::PackError;
use ichr::packs::install::drop_pack_from_path;
use ichr::packs::kind::PackKind;
use ichr::packs::service::PackService;
use ichr::persistence::paths::AppPaths;
use ichr::tasks::JobId;

// ─── Include fixture builder ──────────────────────────────────────────────────

mod fixtures {
    include!("fixtures/packs/build.rs");
}
use fixtures::{build_minimal_resource_pack, build_minimal_shader_pack, sha1_hex_of};

// ─── Shared helpers ───────────────────────────────────────────────────────────

fn make_paths(tmp: &TempDir) -> AppPaths {
    AppPaths::with_roots(
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
    )
}

fn make_service_with_mock(server: &MockServer) -> PackService {
    let client = ModrinthClient::new_with_base_url(server.base_url())
        .expect("ModrinthClient::new_with_base_url");
    PackService::with_client(client)
}

fn make_progress() -> (
    mpsc::Sender<ichr::tasks::TaskEvent>,
    mpsc::Receiver<ichr::tasks::TaskEvent>,
) {
    mpsc::channel(64)
}

async fn make_instance(paths: &AppPaths, slug: &str) {
    let manifest = InstanceManifest::new(slug.to_string(), slug.to_string(), "1.20.4".to_string());
    write_instance_manifest(paths, &manifest)
        .await
        .expect("write_instance_manifest in test setup");
}

// ─── Test 1: Drop resource pack happy path ────────────────────────────────────

/// Drop a resource pack zip into an instance via drop_pack_from_path.
/// Asserts: dest exists at .minecraft/resourcepacks/<filename>,
/// ledger has 1 row with kind=ResourcePack + source=Local.
#[tokio::test]
async fn test_drop_resource_pack_happy_path() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    let source_path = tmp.path().join("faithful.zip");
    let _bytes = build_minimal_resource_pack(&source_path, 22);

    let token = CancellationToken::new();
    let result = drop_pack_from_path(
        &paths,
        "test-instance",
        PackKind::Resource,
        &source_path,
        &token,
    )
    .await
    .expect("drop_pack_from_path must succeed");

    // File must exist at the canonical resource pack destination.
    let dest = paths.instance_pack_file("test-instance", PackKind::Resource, "faithful.zip");
    assert!(
        dest.exists(),
        "resource pack file must exist at: {}",
        dest.display()
    );
    assert_eq!(result.dest, dest);

    // Ledger must have exactly 1 row with correct kind and source.
    let ledger = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .expect("read_ledger");
    assert_eq!(ledger.mods.len(), 1, "ledger must have exactly 1 row");
    let row = &ledger.mods[0];
    assert_eq!(
        row.kind,
        ichr::mods::types::InstalledItemKind::ResourcePack
    );
    assert_eq!(row.source, ichr::mods::types::ModSource::Local);
    assert_eq!(row.file_name, "faithful.zip");
    assert!(row.enabled, "newly dropped pack must be enabled");
}

// ─── Test 2: Drop shader pack happy path ─────────────────────────────────────

/// Drop a shader pack zip into an instance via drop_pack_from_path.
/// Asserts: dest exists at .minecraft/shaderpacks/<filename>,
/// ledger has 1 row with kind=Shader + source=Local.
#[tokio::test]
async fn test_drop_shader_pack_happy_path() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    let source_path = tmp.path().join("complementary.zip");
    let _bytes = build_minimal_shader_pack(&source_path);

    let token = CancellationToken::new();
    let result = drop_pack_from_path(
        &paths,
        "test-instance",
        PackKind::Shader,
        &source_path,
        &token,
    )
    .await
    .expect("drop_pack_from_path for shader must succeed");

    // File must exist at the canonical shader pack destination.
    let dest = paths.instance_pack_file("test-instance", PackKind::Shader, "complementary.zip");
    assert!(
        dest.exists(),
        "shader pack file must exist at: {}",
        dest.display()
    );
    assert_eq!(result.dest, dest);

    // Ledger must have exactly 1 row with kind=Shader.
    let ledger = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .expect("read_ledger");
    assert_eq!(ledger.mods.len(), 1, "ledger must have exactly 1 row");
    let row = &ledger.mods[0];
    assert_eq!(row.kind, ichr::mods::types::InstalledItemKind::Shader);
    assert_eq!(row.source, ichr::mods::types::ModSource::Local);
    assert_eq!(row.file_name, "complementary.zip");
}

// ─── Test 3: Drop collision returns FilenameCollision + ledger unchanged ──────

/// Dropping the same filename twice returns PackError::FilenameCollision on the
/// second attempt. The ledger must have exactly 1 row (no double-insert).
#[tokio::test]
async fn test_drop_collision_returns_filename_collision() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    let source_path = tmp.path().join("my-pack.zip");
    build_minimal_resource_pack(&source_path, 22);

    let token = CancellationToken::new();

    // First drop -- must succeed.
    drop_pack_from_path(
        &paths,
        "test-instance",
        PackKind::Resource,
        &source_path,
        &token,
    )
    .await
    .expect("first drop must succeed");

    // Second drop with same filename -- must return FilenameCollision.
    // The source file still exists so the size / file-name validation passes.
    let result = drop_pack_from_path(
        &paths,
        "test-instance",
        PackKind::Resource,
        &source_path,
        &token,
    )
    .await;

    assert!(
        matches!(result, Err(PackError::FilenameCollision)),
        "second drop must return FilenameCollision, got: {result:?}"
    );

    // Ledger must still have exactly 1 row (no duplicate insert).
    let ledger = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .expect("read_ledger");
    assert_eq!(
        ledger.mods.len(),
        1,
        "ledger must still have exactly 1 row after collision"
    );
}

// ─── Test 4: Drop with cancel returns Cancelled + no partial file ─────────────

/// When the CancellationToken is cancelled before the drop, the call returns
/// PackError::Cancelled, no destination file exists, and the ledger is empty.
#[tokio::test]
async fn test_drop_cancel_mid_copy_cleans_partial() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    let source_path = tmp.path().join("large-pack.zip");
    build_minimal_resource_pack(&source_path, 22);

    // Pre-cancel the token before calling drop_pack_from_path.
    let token = CancellationToken::new();
    token.cancel();

    let result = drop_pack_from_path(
        &paths,
        "test-instance",
        PackKind::Resource,
        &source_path,
        &token,
    )
    .await;

    assert!(
        matches!(result, Err(PackError::Cancelled)),
        "cancelled drop must return PackError::Cancelled, got: {result:?}"
    );

    // No destination file should exist.
    let dest = paths.instance_pack_file("test-instance", PackKind::Resource, "large-pack.zip");
    assert!(
        !dest.exists(),
        "no destination file must exist after cancel: {}",
        dest.display()
    );

    // Ledger must be empty.
    let ledger = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .expect("read_ledger");
    assert!(ledger.mods.is_empty(), "ledger must be empty after cancel");
}

// ─── Test 5: Modrinth install resource pack end-to-end ───────────────────────

/// Full Modrinth resource pack install via httpmock:
/// search → version → download → assert dest + ledger row.
#[tokio::test]
async fn test_modrinth_install_resource_pack_end_to_end() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    // Build a real resource pack zip so SHA-1 verification will pass.
    let zip_path = tmp.path().join("faithful-32x.zip");
    let pack_bytes = build_minimal_resource_pack(&zip_path, 22);
    let sha1 = sha1_hex_of(&pack_bytes);

    // Mock: GET /v2/search → 1 hit
    let _search_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/v2/search")
            .query_param("query", "faithful 32x")
            .query_param_exists("facets");
        then.status(200).json_body(json!({
            "hits": [{
                "project_id": "w0TnApzs",
                "slug": "faithful-32x",
                "title": "Faithful 32x",
                "description": "Faithful resource pack at 32x resolution",
                "downloads": 1000000,
                "project_type": "resourcepack"
            }],
            "total_hits": 1,
            "offset": 0,
            "limit": 20
        }));
    });

    // Mock: GET /v2/project/w0TnApzs/version → 1 version with download URL
    let download_url = server.url("/faithful-32x.zip");
    let _version_mock = server.mock(|when, then| {
        when.method(GET).path("/v2/project/w0TnApzs/version");
        then.status(200).json_body(json!([{
            "id": "kIpbQNcv",
            "project_id": "w0TnApzs",
            "name": "Faithful 32x for MC 1.20.4",
            "version_number": "1.20.4",
            "version_type": "release",
            "game_versions": ["1.20.4"],
            "loaders": ["vanilla"],
            "downloads": 50000,
            "date_published": "2026-01-01T00:00:00Z",
            "dependencies": [],
            "files": [{
                "url": download_url,
                "filename": "faithful-32x.zip",
                "primary": true,
                "size": pack_bytes.len(),
                "hashes": {
                    "sha1": sha1,
                    "sha512": sha1
                }
            }]
        }]));
    });

    // Mock: GET /faithful-32x.zip → pack bytes
    let _download_mock = server.mock(|when, then| {
        when.method(GET).path("/faithful-32x.zip");
        then.status(200)
            .header("Content-Type", "application/octet-stream")
            .body(pack_bytes.clone());
    });

    let svc = make_service_with_mock(&server);

    // Search to get the hit.
    let hits = svc
        .search(
            "faithful 32x",
            PackKind::Resource,
            Some("1.20.4"),
            Some(&paths),
            Some("test-instance"),
        )
        .await
        .expect("search must succeed");

    assert!(!hits.is_empty(), "search must return at least 1 hit");
    let hit = &hits[0];

    // List versions to get the ModrinthVersionEntry.
    let versions = svc
        .list_versions(&hit.project_id, Some("1.20.4"), PackKind::Resource)
        .await
        .expect("list_versions must succeed");

    assert!(!versions.is_empty(), "must have at least 1 version");
    let version_entry = &versions[0];

    // We need the full ModrinthVersion for install. Fetch via get_version.
    // The mock for /v2/version/{id} isn't set up, but list_versions already
    // returned the full wire shape. Use a second mock:
    let download_url2 = server.url("/faithful-32x.zip");
    let sha1_clone = sha1_hex_of(&pack_bytes);
    let pack_bytes_len = pack_bytes.len();
    let _get_version_mock = server.mock(|when, then| {
        when.method(GET).path("/v2/version/kIpbQNcv");
        then.status(200).json_body(json!({
            "id": "kIpbQNcv",
            "project_id": "w0TnApzs",
            "name": "Faithful 32x for MC 1.20.4",
            "version_number": "1.20.4",
            "version_type": "release",
            "game_versions": ["1.20.4"],
            "loaders": ["vanilla"],
            "downloads": 50000,
            "date_published": "2026-01-01T00:00:00Z",
            "dependencies": [],
            "files": [{
                "url": download_url2,
                "filename": "faithful-32x.zip",
                "primary": true,
                "size": pack_bytes_len,
                "hashes": {
                    "sha1": sha1_clone,
                    "sha512": sha1_clone
                }
            }]
        }));
    });

    let full_version = svc
        .get_version(&version_entry.version_id)
        .await
        .expect("get_version must succeed");

    // Install.
    let (progress_tx, _rx) = make_progress();
    let token = CancellationToken::new();
    let row = svc
        .install_modrinth(
            &paths,
            "test-instance",
            PackKind::Resource,
            &full_version,
            &hit.slug,
            &hit.project_id,
            &hit.title,
            progress_tx,
            token,
            JobId(1),
        )
        .await
        .expect("install_modrinth must succeed");

    // Dest file must exist.
    let dest = paths.instance_pack_file("test-instance", PackKind::Resource, &row.file_name);
    assert!(
        dest.exists(),
        "installed resource pack must exist at: {}",
        dest.display()
    );

    // Ledger row must have kind=ResourcePack + source=Modrinth + correct sha1.
    let ledger = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .expect("read_ledger");
    assert_eq!(ledger.mods.len(), 1);
    let r = &ledger.mods[0];
    assert_eq!(
        r.kind,
        ichr::mods::types::InstalledItemKind::ResourcePack
    );
    assert_eq!(r.source, ichr::mods::types::ModSource::Modrinth);
    assert_eq!(
        r.hash_algo,
        ichr::mods::types::HashAlgo::Sha1,
        "pack install uses SHA-1"
    );
    assert!(
        !r.sha512.is_empty(),
        "sha512 field (storing SHA-1) must be non-empty"
    );
}

// ─── Test 6: Modrinth install shader pack end-to-end ─────────────────────────

/// Full Modrinth shader pack install via httpmock.
#[tokio::test]
async fn test_modrinth_install_shader_pack_end_to_end() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    let zip_path = tmp.path().join("complementary.zip");
    let pack_bytes = build_minimal_shader_pack(&zip_path);
    let sha1 = sha1_hex_of(&pack_bytes);

    let download_url = server.url("/complementary.zip");
    let sha1_clone = sha1.clone();
    let pack_bytes_len = pack_bytes.len();

    // Mock: GET /v2/version/836bPNGo → shader version JSON
    let _get_version_mock = server.mock(|when, then| {
        when.method(GET).path("/v2/version/836bPNGo");
        then.status(200).json_body(json!({
            "id": "836bPNGo",
            "project_id": "HVnmMxH1",
            "name": "Complementary Reimagined",
            "version_number": "r5.3",
            "version_type": "release",
            "game_versions": ["1.20.4"],
            "loaders": ["iris", "optifine"],
            "downloads": 200000,
            "date_published": "2026-01-01T00:00:00Z",
            "dependencies": [],
            "files": [{
                "url": download_url,
                "filename": "complementary.zip",
                "primary": true,
                "size": pack_bytes_len,
                "hashes": {
                    "sha1": sha1_clone,
                    "sha512": sha1_clone
                }
            }]
        }));
    });

    // Mock: GET /complementary.zip → shader bytes
    let _download_mock = server.mock(|when, then| {
        when.method(GET).path("/complementary.zip");
        then.status(200)
            .header("Content-Type", "application/octet-stream")
            .body(pack_bytes);
    });

    let svc = make_service_with_mock(&server);
    let full_version = svc
        .get_version("836bPNGo")
        .await
        .expect("get_version must succeed");

    let (progress_tx, _rx) = make_progress();
    let token = CancellationToken::new();
    let row = svc
        .install_modrinth(
            &paths,
            "test-instance",
            PackKind::Shader,
            &full_version,
            "complementary-reimagined",
            "HVnmMxH1",
            "Complementary Reimagined",
            progress_tx,
            token,
            JobId(2),
        )
        .await
        .expect("install_modrinth shader must succeed");

    // Dest must be in shaderpacks/.
    let dest = paths.instance_pack_file("test-instance", PackKind::Shader, &row.file_name);
    assert!(
        dest.exists(),
        "shader pack must exist at: {}",
        dest.display()
    );

    let ledger = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .expect("read_ledger");
    assert_eq!(ledger.mods.len(), 1);
    assert_eq!(
        ledger.mods[0].kind,
        ichr::mods::types::InstalledItemKind::Shader
    );
    assert_eq!(
        ledger.mods[0].source,
        ichr::mods::types::ModSource::Modrinth
    );
}

// ─── Test 7: Toggle resource pack enabled -- extension rename round-trip ───────

/// Install via drop; toggle disabled (.zip → .zip.disabled); toggle back (.zip).
/// Verifies ledger enabled flag flips and file is renamed in resourcepacks/.
#[tokio::test]
async fn test_toggle_resource_pack_enabled_flips_extension() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    let source_path = tmp.path().join("my-resource.zip");
    build_minimal_resource_pack(&source_path, 22);

    let token = CancellationToken::new();
    let outcome = drop_pack_from_path(
        &paths,
        "test-instance",
        PackKind::Resource,
        &source_path,
        &token,
    )
    .await
    .expect("drop must succeed");

    let mod_id = outcome.row.mod_id.clone();

    // Verify the file exists and is enabled.
    let enabled_path =
        paths.instance_pack_file("test-instance", PackKind::Resource, "my-resource.zip");
    assert!(enabled_path.exists(), "pack must exist after install");

    // Build a service to call toggle_pack_enabled.
    let server = MockServer::start(); // no HTTP calls expected for toggle
    let svc = make_service_with_mock(&server);

    // First toggle: enabled → disabled.
    let new_state = svc
        .toggle_pack_enabled(&paths, "test-instance", &mod_id, PackKind::Resource)
        .await
        .expect("first toggle must succeed");

    assert!(!new_state, "after first toggle, pack must be disabled");
    let disabled_path = paths.instance_pack_file(
        "test-instance",
        PackKind::Resource,
        "my-resource.zip.disabled",
    );
    assert!(
        disabled_path.exists(),
        "disabled file must exist at: {}",
        disabled_path.display()
    );
    assert!(
        !enabled_path.exists(),
        "enabled file must NOT exist after disable"
    );

    // Verify ledger row has enabled=false.
    let ledger = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .unwrap();
    assert!(!ledger.mods[0].enabled);

    // Second toggle: disabled → enabled.
    let new_state2 = svc
        .toggle_pack_enabled(&paths, "test-instance", &mod_id, PackKind::Resource)
        .await
        .expect("second toggle must succeed");

    assert!(new_state2, "after second toggle, pack must be re-enabled");
    assert!(
        enabled_path.exists(),
        "re-enabled file must be back at: {}",
        enabled_path.display()
    );
    assert!(
        !disabled_path.exists(),
        "disabled file must NOT exist after re-enable"
    );

    // Verify ledger row has enabled=true again.
    let ledger2 = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .unwrap();
    assert!(ledger2.mods[0].enabled);
}

// ─── Test 8: Uninstall pack removes file + ledger row ────────────────────────

/// Install via drop, then uninstall. Asserts: file gone, ledger empty.
#[tokio::test]
async fn test_uninstall_pack_removes_file_and_ledger() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    let source_path = tmp.path().join("uninstall-me.zip");
    build_minimal_resource_pack(&source_path, 22);

    let token = CancellationToken::new();
    let outcome = drop_pack_from_path(
        &paths,
        "test-instance",
        PackKind::Resource,
        &source_path,
        &token,
    )
    .await
    .expect("drop must succeed");

    let mod_id = outcome.row.mod_id.clone();
    let dest = paths.instance_pack_file("test-instance", PackKind::Resource, "uninstall-me.zip");
    assert!(dest.exists(), "pack must exist before uninstall");

    let server = MockServer::start(); // no HTTP calls expected
    let svc = make_service_with_mock(&server);

    svc.uninstall_pack(&paths, "test-instance", &mod_id, PackKind::Resource)
        .await
        .expect("uninstall must succeed");

    assert!(
        !dest.exists(),
        "pack file must be gone after uninstall: {}",
        dest.display()
    );

    let ledger = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .unwrap();
    assert!(
        ledger.mods.is_empty(),
        "ledger must be empty after uninstall"
    );
}

// ─── Test 9 (bonus): Toggle shader pack returns ShaderToggleNotSupported ─────

/// D-LOCK: toggle_pack_enabled on a shader pack must return
/// PackError::ShaderToggleNotSupported. The file on disk is unchanged.
#[tokio::test]
async fn test_toggle_shader_pack_returns_shader_toggle_not_supported() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    let source_path = tmp.path().join("shader.zip");
    build_minimal_shader_pack(&source_path);

    let token = CancellationToken::new();
    let outcome = drop_pack_from_path(
        &paths,
        "test-instance",
        PackKind::Shader,
        &source_path,
        &token,
    )
    .await
    .expect("drop shader must succeed");

    let mod_id = outcome.row.mod_id.clone();

    let server = MockServer::start();
    let svc = make_service_with_mock(&server);

    let result = svc
        .toggle_pack_enabled(&paths, "test-instance", &mod_id, PackKind::Shader)
        .await;

    assert!(
        matches!(result, Err(PackError::ShaderToggleNotSupported { .. })),
        "toggle shader must return ShaderToggleNotSupported, got: {result:?}"
    );

    // File on disk must be unchanged (still the .zip, not .zip.disabled).
    let dest = paths.instance_pack_file("test-instance", PackKind::Shader, "shader.zip");
    assert!(
        dest.exists(),
        "shader file must remain unchanged after failed toggle"
    );
}

// ─── Test 10 (bonus): list_installed filters by kind in mixed ledger ─────────

/// A ledger with both a resource pack row and a shader pack row:
/// list_installed(Resource) returns only the resource row,
/// list_installed(Shader) returns only the shader row.
#[tokio::test]
async fn test_list_installed_filters_by_kind_in_mixed_ledger() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    // Install one resource pack.
    let rp_path = tmp.path().join("faithful.zip");
    build_minimal_resource_pack(&rp_path, 22);
    let token = CancellationToken::new();
    drop_pack_from_path(
        &paths,
        "test-instance",
        PackKind::Resource,
        &rp_path,
        &token,
    )
    .await
    .expect("resource pack drop must succeed");

    // Install one shader pack.
    let sp_path = tmp.path().join("complementary.zip");
    build_minimal_shader_pack(&sp_path);
    let token2 = CancellationToken::new();
    drop_pack_from_path(&paths, "test-instance", PackKind::Shader, &sp_path, &token2)
        .await
        .expect("shader pack drop must succeed");

    let server = MockServer::start();
    let svc = make_service_with_mock(&server);

    // list_installed(Resource) must return exactly 1 row.
    let rp_rows = svc
        .list_installed(&paths, "test-instance", PackKind::Resource)
        .await
        .expect("list_installed Resource");
    assert_eq!(rp_rows.len(), 1, "must have exactly 1 resource pack row");
    assert_eq!(
        rp_rows[0].kind,
        ichr::mods::types::InstalledItemKind::ResourcePack
    );

    // list_installed(Shader) must return exactly 1 row.
    let sp_rows = svc
        .list_installed(&paths, "test-instance", PackKind::Shader)
        .await
        .expect("list_installed Shader");
    assert_eq!(sp_rows.len(), 1, "must have exactly 1 shader pack row");
    assert_eq!(
        sp_rows[0].kind,
        ichr::mods::types::InstalledItemKind::Shader
    );
}

// ─── Test 11 (bonus): Modrinth install oversized file is rejected ─────────────

/// When the Modrinth version JSON reports file.size > MAX_PACK_FILE_BYTES,
/// install_modrinth must return Err(PackError::FileTooLarge) BEFORE sending
/// the download request. Dest does not exist; ledger empty.
#[tokio::test]
async fn test_modrinth_install_oversized_file_rejected() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    let oversized = ichr::mods::filter::MAX_PACK_FILE_BYTES + 1;

    // Mock: GET /v2/version/oversized → version with huge file.size
    let _get_version_mock = server.mock(|when, then| {
        when.method(GET).path("/v2/version/oversized");
        then.status(200).json_body(json!({
            "id": "oversized",
            "project_id": "OVPID",
            "name": "Oversized Pack",
            "version_number": "999.0",
            "version_type": "release",
            "game_versions": ["1.20.4"],
            "loaders": ["vanilla"],
            "downloads": 0,
            "date_published": "2026-01-01T00:00:00Z",
            "dependencies": [],
            "files": [{
                "url": server.url("/huge.zip"),
                "filename": "huge.zip",
                "primary": true,
                "size": oversized,
                "hashes": {
                    "sha1": "aaaa",
                    "sha512": "aaaa"
                }
            }]
        }));
    });

    // The download URL must NOT be hit.
    let _download_mock = server.mock(|when, then| {
        when.method(GET).path("/huge.zip");
        then.status(500).body(b"must not be called");
    });

    let svc = make_service_with_mock(&server);
    let full_version = svc
        .get_version("oversized")
        .await
        .expect("get_version must succeed");

    let (progress_tx, _rx) = make_progress();
    let token = CancellationToken::new();
    let result = svc
        .install_modrinth(
            &paths,
            "test-instance",
            PackKind::Resource,
            &full_version,
            "oversized-pack",
            "OVPID",
            "Oversized Pack",
            progress_tx,
            token,
            JobId(3),
        )
        .await;

    assert!(
        matches!(result, Err(PackError::FileTooLarge { .. })),
        "oversized install must return FileTooLarge, got: {result:?}"
    );

    // No destination file.
    let dest = paths.instance_pack_file("test-instance", PackKind::Resource, "huge.zip");
    assert!(!dest.exists(), "no dest file after FileTooLarge rejection");

    // Ledger empty.
    let ledger = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .unwrap();
    assert!(
        ledger.mods.is_empty(),
        "ledger must be empty after rejection"
    );

    // Download endpoint must NOT have been hit.
    _download_mock.assert_calls(0);
}

// ─── Test 12: Browser Enter-key install chain -- auto-pick latest stable ──────

/// GAP-11-A regression test: pin the auto-pick behaviour of the
/// `Effect::FetchPackVersions` task chain that backs the pack browser
/// Enter key.
///
/// The chain in `src/tui/run.rs` is:
///   `pack_service.list_versions(project_id, mc, kind)`
///   -> pick first `is_latest_stable=true` (fallback `versions.first()`)
///   -> `pack_service.get_version(version_id)`
///   -> dispatch the existing `Effect::InstallPackFromModrinth`
///
/// This test mocks Modrinth with 3 versions in date-descending order:
///   1. `older_release`  -- `version_type: "release"`, `date_published: 2026-02-01`
///   2. `newer_release`  -- `version_type: "release"`, `date_published: 2026-03-01`
///   3. `unstable_beta`  -- `version_type: "beta"`,    `date_published: 2026-04-01`
///
/// `PackService::list_versions` sorts by `date_published` DESC then marks
/// the FIRST encountered `release` as `is_latest_stable=true`. So the
/// auto-pick MUST land on `newer_release` (NOT the beta even though it is
/// chronologically newest, NOT the older_release even though they share a
/// channel).
///
/// Asserts the installed file matches `newer_release`'s filename.
#[tokio::test]
async fn test_browser_enter_install_chain_picks_latest_stable() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(&tmp);
    make_instance(&paths, "test-instance").await;

    // Build the file body that the *winning* version will reference.
    let zip_path = tmp.path().join("newer-release.zip");
    let pack_bytes = build_minimal_resource_pack(&zip_path, 22);
    let sha1 = sha1_hex_of(&pack_bytes);
    let download_url = server.url("/newer-release.zip");

    // GET /v2/project/PID/version → 3 entries (response order does NOT
    // matter; PackService::list_versions sorts by date_published DESC).
    let _list_mock = server.mock(|when, then| {
        when.method(GET).path("/v2/project/PIDxxxx/version");
        then.status(200).json_body(json!([
            {
                "id": "older_release",
                "project_id": "PIDxxxx",
                "name": "Older Release",
                "version_number": "1.0",
                "version_type": "release",
                "game_versions": ["1.20.4"],
                "loaders": ["vanilla"],
                "downloads": 100,
                "date_published": "2026-02-01T00:00:00Z",
                "dependencies": [],
                "files": [{
                    "url": server.url("/older-release.zip"),
                    "filename": "older-release.zip",
                    "primary": true,
                    "size": 999,
                    "hashes": { "sha1": "deadbeef", "sha512": "deadbeef" }
                }]
            },
            {
                "id": "newer_release",
                "project_id": "PIDxxxx",
                "name": "Newer Release",
                "version_number": "1.1",
                "version_type": "release",
                "game_versions": ["1.20.4"],
                "loaders": ["vanilla"],
                "downloads": 200,
                "date_published": "2026-03-01T00:00:00Z",
                "dependencies": [],
                "files": [{
                    "url": download_url.clone(),
                    "filename": "newer-release.zip",
                    "primary": true,
                    "size": pack_bytes.len(),
                    "hashes": { "sha1": sha1.clone(), "sha512": sha1.clone() }
                }]
            },
            {
                "id": "unstable_beta",
                "project_id": "PIDxxxx",
                "name": "Unstable Beta",
                "version_number": "2.0-beta",
                "version_type": "beta",
                "game_versions": ["1.20.4"],
                "loaders": ["vanilla"],
                "downloads": 50,
                "date_published": "2026-04-01T00:00:00Z",
                "dependencies": [],
                "files": [{
                    "url": server.url("/unstable-beta.zip"),
                    "filename": "unstable-beta.zip",
                    "primary": true,
                    "size": 999,
                    "hashes": { "sha1": "cafebabe", "sha512": "cafebabe" }
                }]
            }
        ]));
    });

    // GET /v2/version/newer_release → full body for the winning entry.
    // The losing entries (older_release, unstable_beta) MUST NOT be fetched
    // by version_id, since the auto-pick step happens BEFORE get_version.
    let sha1_for_get = sha1.clone();
    let url_for_get = download_url.clone();
    let _get_winner_mock = server.mock(|when, then| {
        when.method(GET).path("/v2/version/newer_release");
        then.status(200).json_body(json!({
            "id": "newer_release",
            "project_id": "PIDxxxx",
            "name": "Newer Release",
            "version_number": "1.1",
            "version_type": "release",
            "game_versions": ["1.20.4"],
            "loaders": ["vanilla"],
            "downloads": 200,
            "date_published": "2026-03-01T00:00:00Z",
            "dependencies": [],
            "files": [{
                "url": url_for_get,
                "filename": "newer-release.zip",
                "primary": true,
                "size": pack_bytes.len(),
                "hashes": { "sha1": sha1_for_get.clone(), "sha512": sha1_for_get }
            }]
        }));
    });

    // Negative-assert mocks: if the chain wrongly picks a loser, these will fire.
    let loser_get_older = server.mock(|when, then| {
        when.method(GET).path("/v2/version/older_release");
        then.status(500)
            .body(b"loser_older fetched -- chain picked wrong");
    });
    let loser_get_beta = server.mock(|when, then| {
        when.method(GET).path("/v2/version/unstable_beta");
        then.status(500)
            .body(b"loser_beta fetched -- chain picked wrong");
    });

    // Download endpoint for the winner.
    let _download_mock = server.mock(|when, then| {
        when.method(GET).path("/newer-release.zip");
        then.status(200)
            .header("Content-Type", "application/octet-stream")
            .body(pack_bytes.clone());
    });

    let svc = make_service_with_mock(&server);

    // ── Walk the chain that the Effect::FetchPackVersions task performs in
    //    src/tui/run.rs. This is the *exact* logic the run.rs handler executes.

    // Step 1: list_versions
    let entries = svc
        .list_versions("PIDxxxx", Some("1.20.4"), PackKind::Resource)
        .await
        .expect("list_versions must succeed");
    assert_eq!(entries.len(), 3, "all 3 entries must come back");

    // Step 2: pick first is_latest_stable, fall back to versions.first()
    //         (mirrors the run.rs handler).
    let entry = entries
        .iter()
        .find(|v| v.is_latest_stable)
        .cloned()
        .or_else(|| entries.first().cloned())
        .expect("at least one entry");
    assert_eq!(
        entry.version_id, "newer_release",
        "auto-pick must land on the newest *release* (date_published 2026-03-01), \
         NOT the chronologically newest beta and NOT the older release"
    );
    assert!(entry.is_latest_stable, "marker must be true on the winner");

    // Step 3: get_version
    let full_version = svc
        .get_version(&entry.version_id)
        .await
        .expect("get_version must succeed");

    // Step 4: install_modrinth (the existing Effect::InstallPackFromModrinth path)
    let (progress_tx, _rx) = make_progress();
    let token = CancellationToken::new();
    let row = svc
        .install_modrinth(
            &paths,
            "test-instance",
            PackKind::Resource,
            &full_version,
            "test-pack",
            "PIDxxxx",
            "Test Pack",
            progress_tx,
            token,
            JobId(99),
        )
        .await
        .expect("install_modrinth must succeed for the auto-picked winner");

    // The installed filename must match the *winner's* file, not a loser.
    assert_eq!(row.file_name, "newer-release.zip");
    let dest = paths.instance_pack_file("test-instance", PackKind::Resource, &row.file_name);
    assert!(
        dest.exists(),
        "winner file must exist on disk: {}",
        dest.display()
    );

    // Ledger has exactly one row, sourced from Modrinth.
    let ledger = ichr::mods::ledger::read_ledger(&paths, "test-instance")
        .await
        .expect("read_ledger");
    assert_eq!(ledger.mods.len(), 1);
    assert_eq!(
        ledger.mods[0].source,
        ichr::mods::types::ModSource::Modrinth
    );
    assert_eq!(
        ledger.mods[0].mod_id, "PIDxxxx",
        "ledger row must be keyed on the winner's project_id"
    );

    // Loser version endpoints must NOT have been hit by the chain.
    loser_get_older.assert_calls(0);
    loser_get_beta.assert_calls(0);
}
