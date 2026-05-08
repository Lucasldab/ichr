//! Integration tests for the CurseForgeService over httpmock.
//!
//! Validates Phase 9-specific surfaces that the per-module unit tests cannot
//! observe in isolation, plus paranoid regression-net duplicates of the most
//! load-bearing unit-level invariants:
//!
//! 1. `test_constructor_rejects_empty_key_before_reqwest_invoked` — empty
//!    API key produces `Err` at `CurseForgeClient::new_with_base_url`
//!    construction (no reqwest call). Defense-in-depth integration test for
//!    the empty-string filter at the precedence-resolver boundary; duplicated
//!    here so a regression in 09-03's `new_with_base_url` is caught even if
//!    the unit test is removed.
//! 2. `test_x_api_key_present_at_integration_layer` — the `x-api-key` header
//!    survives the service-layer wrap (Pitfall 2). Mock requires the header
//!    on the wire; a missing header would surface as an httpmock 404 (no
//!    matching mock).
//! 3. `test_install_with_inline_url_writes_ledger_row_with_curseforge_source`
//!    — full happy install path through `CurseForgeService::with_client`.
//!    Writes ledger row with `source: ModSource::CurseForge`,
//!    `hash_algo: HashAlgo::Sha1`, sha512 field stores SHA-1 hex (historical
//!    naming carve-out per 09-RESEARCH.md §"Per-Instance Ledger Reuse"
//!    lines 297-318).
//! 4. `test_install_with_null_download_url_returns_file_not_downloadable_no_ledger_row`
//!    — `downloadUrl: null` + the dedicated `/download-url` endpoint
//!    returning 403 surfaces `CurseForgeError::FileNotDownloadable` with the
//!    canonical web URL, AND the ledger remains empty (Pitfall 8 atomicity).
//! 5. `test_cancel_aborts_install_no_ledger_row` — cancellation mid-install
//!    returns `CurseForgeError::Cancelled` AND the ledger remains empty
//!    (atomicity invariant: the ledger upsert is gated on a successful
//!    sha1-verified download; an aborted download MUST NOT leave a row).
//!
//! All tests use a per-test `MockServer` (parallel-safe).

use httpmock::prelude::*;
use mineltui::mods::curseforge::client::CurseForgeClient;
use mineltui::mods::curseforge::error::CurseForgeError;
use mineltui::mods::curseforge::service::CurseForgeService;
use mineltui::mods::curseforge::types::{
    CurseForgeAuthor, CurseForgeFileEntry, CurseForgeHash, CurseForgeLinks, CurseForgeProjectDetail,
};
use mineltui::mods::types::{HashAlgo, Ledger, ModSource};
use mineltui::persistence::paths::AppPaths;
use mineltui::tasks::JobId;
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn paths_for(td: &TempDir) -> AppPaths {
    AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    )
}

fn fx_detail(id: u64, slug: &str) -> CurseForgeProjectDetail {
    CurseForgeProjectDetail {
        id,
        slug: slug.into(),
        name: "X".into(),
        summary: String::new(),
        description: String::new(),
        download_count: 0,
        authors: vec![CurseForgeAuthor {
            id: 1,
            name: "Author".into(),
            url: String::new(),
        }],
        links: CurseForgeLinks::default(),
    }
}

fn sha1_hex(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(bytes);
    h.finalize()
        .iter()
        .fold(String::with_capacity(40), |mut s, b| {
            use std::fmt::Write;
            write!(s, "{b:02x}").unwrap();
            s
        })
}

// ============================================================================
// Test 1: Empty key rejected at constructor (defense-in-depth).
// ============================================================================

#[tokio::test]
async fn test_constructor_rejects_empty_key_before_reqwest_invoked() {
    let r = CurseForgeClient::new_with_base_url("", "http://localhost:1234");
    assert!(
        matches!(r, Err(CurseForgeError::Http(_))),
        "empty key must error at construction; got {r:?}"
    );
}

// ============================================================================
// Test 2: x-api-key invariant survives the service-layer wrap (Pitfall 2).
// ============================================================================

#[tokio::test]
async fn test_x_api_key_present_at_integration_layer() {
    let server = MockServer::start();
    let m = server.mock(|when, then| {
        when.method(GET)
            .path("/v1/mods/search")
            .header("x-api-key", "test-key");
        then.status(200).body(r#"{"data":[]}"#);
    });
    let client = CurseForgeClient::new_with_base_url("test-key", server.base_url()).unwrap();
    let svc = CurseForgeService::with_client(client);
    // Search must succeed only because the mock matched on the header; if the
    // header was dropped at the service-layer wrap, the mock would not match
    // and httpmock would return 404, surfacing as Err.
    let _ = svc.search("x", None, None, None, None).await.unwrap();
    m.assert();
}

// ============================================================================
// Test 3: Happy install — ledger row written with source=CurseForge, hash_algo=Sha1.
// ============================================================================

#[tokio::test]
async fn test_install_with_inline_url_writes_ledger_row_with_curseforge_source() {
    let server = MockServer::start();
    let body = b"fake-jar-body".to_vec();
    let sha1 = sha1_hex(&body);

    // Mock the CDN for the mod file.
    server.mock(|when, then| {
        when.method(GET).path("/cdn/sodium.jar");
        then.status(200).body(body.clone());
    });

    let client = CurseForgeClient::new_with_base_url("test-key", server.base_url()).unwrap();
    let svc = CurseForgeService::with_client(client);

    let td = TempDir::new().unwrap();
    let paths = paths_for(&td);
    let slug = "happy-install";

    // Pre-create the .minecraft/mods directory (the installer expects it).
    tokio::fs::create_dir_all(paths.instance_minecraft_dir(slug).join("mods"))
        .await
        .unwrap();

    let detail = fx_detail(443959, "sodium");
    let file = CurseForgeFileEntry {
        id: 4567890,
        display_name: "Sodium 0.5.8".into(),
        file_name: "sodium-fabric.jar".into(),
        release_type: 1,
        file_status: 4,
        hashes: vec![CurseForgeHash {
            value: sha1.clone(),
            algo: 1,
        }],
        file_date: "2026-01-01T00:00:00Z".into(),
        file_length: body.len() as u64,
        download_count: 100,
        download_url: Some(format!("{}/cdn/sodium.jar", server.base_url())),
        game_versions: vec!["1.20.4".into()],
        dependencies: vec![],
        is_available: true,
    };

    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let token = CancellationToken::new();

    svc.install_mod_into_instance(&paths, slug, &detail, &file, tx, token, JobId(0))
        .await
        .expect("install");

    // Ledger assertions.
    let ledger_raw = tokio::fs::read_to_string(paths.instance_mod_ledger(slug))
        .await
        .unwrap();
    let ledger: Ledger = toml::from_str(&ledger_raw).unwrap();
    assert_eq!(ledger.mods.len(), 1, "expected exactly 1 ledger row");
    let row = &ledger.mods[0];
    assert_eq!(
        row.source,
        ModSource::CurseForge,
        "source must be CurseForge"
    );
    assert_eq!(row.hash_algo, HashAlgo::Sha1, "hash_algo must be Sha1");
    assert_eq!(row.mod_id, "443959");
    assert_eq!(row.version_id, "4567890");
    assert_eq!(
        row.sha512, sha1,
        "sha512 field stores SHA-1 hex (historical naming carve-out)"
    );

    // File on disk.
    let f = paths.instance_mod_file(slug, &row.file_name);
    assert!(f.is_file(), "mod file missing: {}", f.display());
}

// ============================================================================
// Test 4: downloadUrl null + 403 fallback → FileNotDownloadable + no ledger.
// ============================================================================

#[tokio::test]
async fn test_install_with_null_download_url_returns_file_not_downloadable_no_ledger_row() {
    let server = MockServer::start();
    // The /download-url fallback endpoint must return 403 (restricted).
    server.mock(|when, then| {
        when.method(GET)
            .path("/v1/mods/443959/files/4567890/download-url");
        then.status(403).body(r#"{"error":"restricted"}"#);
    });

    let client = CurseForgeClient::new_with_base_url("test-key", server.base_url()).unwrap();
    let svc = CurseForgeService::with_client(client);

    let td = TempDir::new().unwrap();
    let paths = paths_for(&td);
    let slug = "restricted-test";
    tokio::fs::create_dir_all(paths.instance_minecraft_dir(slug).join("mods"))
        .await
        .unwrap();

    let detail = fx_detail(443959, "wonderful-world-mod");
    let file = CurseForgeFileEntry {
        id: 4567890,
        display_name: "Wonderful World 1.5.0".into(),
        file_name: "wwm.jar".into(),
        release_type: 1,
        file_status: 4,
        hashes: vec![CurseForgeHash {
            value: "abc".into(),
            algo: 1,
        }],
        file_date: "2026-01-01T00:00:00Z".into(),
        file_length: 1024,
        download_count: 100,
        download_url: None, // CRITICAL: triggers the null fallback chain
        game_versions: vec!["1.20.4".into()],
        dependencies: vec![],
        is_available: true,
    };

    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let token = CancellationToken::new();

    let res = svc
        .install_mod_into_instance(&paths, slug, &detail, &file, tx, token, JobId(0))
        .await;
    match res {
        Err(CurseForgeError::FileNotDownloadable {
            web_url,
            mod_slug,
            file_id,
        }) => {
            assert_eq!(
                web_url,
                "https://www.curseforge.com/minecraft/mc-mods/wonderful-world-mod/files/4567890",
                "web_url must be canonical CurseForge URL"
            );
            assert_eq!(mod_slug, "wonderful-world-mod");
            assert_eq!(file_id, 4567890);
        }
        other => panic!("expected FileNotDownloadable, got {other:?}"),
    }

    // Atomicity: NO ledger row written.
    let ledger_path = paths.instance_mod_ledger(slug);
    if ledger_path.exists() {
        let raw = tokio::fs::read_to_string(&ledger_path).await.unwrap();
        let ledger: Ledger = toml::from_str(&raw).unwrap_or_default();
        assert!(
            ledger.mods.is_empty(),
            "no ledger row should exist after FileNotDownloadable"
        );
    }
}

// ============================================================================
// Test 5: Cancel mid-install → Cancelled + no ledger row (atomicity).
// ============================================================================

#[tokio::test]
async fn test_cancel_aborts_install_no_ledger_row() {
    let server = MockServer::start();
    let body = vec![0u8; 1024];
    let sha1 = sha1_hex(&body);
    // Slow CDN — 2-second delay so we can cancel mid-stream.
    server.mock(|when, then| {
        when.method(GET).path("/cdn/slow.jar");
        then.status(200)
            .delay(std::time::Duration::from_secs(2))
            .body(body.clone());
    });

    let client = CurseForgeClient::new_with_base_url("test-key", server.base_url()).unwrap();
    let svc = std::sync::Arc::new(CurseForgeService::with_client(client));

    let td = TempDir::new().unwrap();
    let paths = paths_for(&td);
    let slug = "cancel-test";
    tokio::fs::create_dir_all(paths.instance_minecraft_dir(slug).join("mods"))
        .await
        .unwrap();

    let detail = fx_detail(1, "slow");
    let file = CurseForgeFileEntry {
        id: 1,
        display_name: "Slow".into(),
        file_name: "slow.jar".into(),
        release_type: 1,
        file_status: 4,
        hashes: vec![CurseForgeHash {
            value: sha1,
            algo: 1,
        }],
        file_date: "x".into(),
        file_length: 1024,
        download_count: 1,
        download_url: Some(format!("{}/cdn/slow.jar", server.base_url())),
        game_versions: vec![],
        dependencies: vec![],
        is_available: true,
    };

    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let token = CancellationToken::new();
    let token_clone = token.clone();

    let svc_clone = svc.clone();
    let paths_clone = paths.clone();
    let slug_str = slug.to_string();
    let install_handle = tokio::spawn(async move {
        svc_clone
            .install_mod_into_instance(
                &paths_clone,
                &slug_str,
                &detail,
                &file,
                tx,
                token_clone,
                JobId(0),
            )
            .await
    });

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    token.cancel();

    let res = install_handle.await.expect("join");
    assert!(
        matches!(res, Err(CurseForgeError::Cancelled)),
        "expected Cancelled, got {res:?}"
    );

    // Atomicity: ledger MUST be empty.
    let ledger_path = paths.instance_mod_ledger(slug);
    if ledger_path.exists() {
        let raw = tokio::fs::read_to_string(&ledger_path).await.unwrap();
        let ledger: Ledger = toml::from_str(&raw).unwrap_or_default();
        assert!(ledger.mods.is_empty(), "ledger MUST be empty after cancel");
    }
}
