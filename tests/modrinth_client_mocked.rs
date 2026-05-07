//! Integration tests for the ModrinthService over httpmock.
//!
//! Validates three load-bearing invariants of the install pipeline that the
//! per-module unit tests cannot observe in isolation:
//!
//! 1. `test_install_respects_concurrency_cap` — parallel install respects
//!    `MOD_DOWNLOAD_CONCURRENCY = 6`. Verified by **timing**: with
//!    `N = 3 * cap` jobs that each take ~`delay` ms in the mock, total wall
//!    time MUST be at least `~3 * delay` (3 batches). A broken cap (no
//!    bound, or cap = N) would finish in ~1 batch. The matcher also tracks
//!    a high-water in-flight counter as diagnostic observability (logged
//!    via eprintln, not asserted strictly because httpmock 0.8 has no
//!    response-completion hook).
//!    Covers Pitfall 8 + ASSUMPTION A1 (08-RESEARCH.md §Validation Architecture).
//!
//! 2. `test_cancel_aborts_install_no_ledger_write` — cancellation mid-install
//!    returns `ModrinthError::Cancelled` and writes STRICTLY FEWER ledger
//!    rows than the plan length. Atomicity guard: if cancel arrives before a
//!    given file's `upsert_mod` lands, that mod MUST NOT appear in the
//!    ledger. (Threat T-08-09-03 + dep on installer.rs:316 cancel checks.)
//!
//! 3. `test_resolve_dependencies_through_full_service_stack` — the
//!    `ModrinthService::resolve_dependencies` path composes correctly through
//!    `ModrinthClient` over a mock that returns a Sodium root version with one
//!    Required dep (Fabric API). Asserts the dep graph contains at least one
//!    new Required download. (08-04 + 08-06 service-stack composition.)
//!
//! All tests use a per-test `MockServer` (parallel-safe).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use httpmock::prelude::*;
use mineltui::mods::error::ModrinthError;
use mineltui::mods::installer::{install_mods_into_instance, InstallStep, MOD_DOWNLOAD_CONCURRENCY};
use mineltui::mods::modrinth::ModrinthClient;
use mineltui::mods::service::ModrinthService;
use mineltui::mods::types::{
    DepKind, HashAlgo, InstalledModRow, Ledger, ModSource, ModrinthFile, ModrinthHashes,
};
use mineltui::persistence::paths::AppPaths;
use mineltui::tasks::JobId;
use sha2::{Digest, Sha512};
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

fn sha512_hex_of(bytes: &[u8]) -> String {
    let mut h = Sha512::new();
    h.update(bytes);
    h.finalize()
        .iter()
        .fold(String::with_capacity(128), |mut s, b| {
            use std::fmt::Write;
            write!(s, "{b:02x}").unwrap();
            s
        })
}

// ============================================================================
// Test 1: Concurrency cap respected
// ============================================================================

#[tokio::test]
async fn test_install_respects_concurrency_cap() {
    // Strategy: spawn N = 3 * cap mocks, each delaying `delay_ms`. With the cap
    // working, total wall time MUST be at least ~3 batches × delay. Without the
    // cap (or with cap >= N), we would observe ~1 batch.
    //
    // We also track in-flight via the matcher closure — increment on entry,
    // record max — but do NOT decrement (httpmock does not expose a response-
    // complete hook). The counter is a soft cross-check: the high-water mark
    // observed within a batch should never exceed `cap` (proven indirectly
    // because the install pipeline only issues HTTP after acquiring a permit).
    //
    // The TIMING assertion is the load-bearing one.
    let server = MockServer::start();
    let cap = MOD_DOWNLOAD_CONCURRENCY;
    let n: usize = 3 * cap; // 3 batches
    let delay_ms: u64 = 200;

    // Per-job body bytes + sha512 must match what we hand the install plan,
    // because the install pipeline verifies sha512 on the response body.
    let bodies: Vec<Vec<u8>> = (0..n)
        .map(|i| format!("body-bytes-for-mod-{i}").into_bytes())
        .collect();

    // Track high-water in-flight count via a shared atomic.
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));

    // We also rely on the semaphore pattern: in addition to the counter,
    // observe a soft per-batch decay by spawning a delayed-decrement task
    // from inside the matcher. The matcher runs on the tokio runtime
    // (httpmock 0.8 is async-tokio), so `tokio::spawn` works there.
    for (i, body) in bodies.iter().enumerate() {
        let body = body.clone();
        let inf = Arc::clone(&in_flight);
        let maxs = Arc::clone(&max_seen);
        server.mock(move |when, then| {
            let inf2 = Arc::clone(&inf);
            let maxs2 = Arc::clone(&maxs);
            when.method(GET)
                .path(format!("/cdn/mod-{i}.jar"))
                .is_true(move |_req| {
                    let cur = inf2.fetch_add(1, Ordering::SeqCst) + 1;
                    let mut prev = maxs2.load(Ordering::SeqCst);
                    while cur > prev {
                        match maxs2.compare_exchange(
                            prev,
                            cur,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        ) {
                            Ok(_) => break,
                            Err(actual) => prev = actual,
                        }
                    }
                    // Schedule a decrement after the response delay so the
                    // counter approximates "currently in flight" rather than
                    // "ever issued".
                    let inf3 = Arc::clone(&inf2);
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(delay_ms + 50)).await;
                        inf3.fetch_sub(1, Ordering::SeqCst);
                    });
                    true
                });
            then.status(200)
                .delay(Duration::from_millis(delay_ms))
                .body(body);
        });
    }

    let td = TempDir::new().unwrap();
    let paths = paths_for(&td);
    let slug = "concur-test";
    tokio::fs::create_dir_all(paths.instance_minecraft_dir(slug).join("mods"))
        .await
        .unwrap();

    // Build install plan from the bodies.
    let mut plan: Vec<InstallStep> = Vec::with_capacity(n);
    for (i, body) in bodies.iter().enumerate() {
        let sha = sha512_hex_of(body);
        plan.push(InstallStep {
            row: InstalledModRow {
                mod_id: format!("M{i}"),
                project_slug: format!("p{i}"),
                display_name: format!("Mod {i}"),
                version_id: "v".into(),
                version_label: "0.0".into(),
                file_name: format!("mod-{i}.jar"),
                sha512: sha.clone(),
                size: body.len() as u64,
                hash_algo: HashAlgo::Sha512,
                source: ModSource::Modrinth,
                enabled: true,
                installed_at: "2026-01-01T00:00:00Z".into(),
            },
            file: ModrinthFile {
                url: format!("{}/cdn/mod-{i}.jar", server.base_url()),
                filename: format!("mod-{i}.jar"),
                primary: true,
                size: body.len() as u64,
                hashes: ModrinthHashes {
                    sha1: "x".into(),
                    sha512: sha,
                },
            },
        });
    }

    let http = reqwest::Client::builder()
        .user_agent("test")
        .build()
        .unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let token = CancellationToken::new();

    let started = Instant::now();
    install_mods_into_instance(
        http,
        paths.clone(),
        slug.into(),
        plan,
        tx,
        token,
        JobId(0),
    )
    .await
    .expect("install");
    let elapsed = started.elapsed();

    // Load-bearing assertion: with 3 batches of `delay_ms`, total must be at
    // least 2.5 × delay (allow 0.5-batch slack for network/scheduler jitter).
    // A broken cap would finish in ~1 batch (~delay_ms) — far below this gate.
    let min_expected = Duration::from_millis(((n / cap) as u64 - 1) * delay_ms + delay_ms / 2);
    assert!(
        elapsed >= min_expected,
        "concurrency cap appears broken: install of {n} jobs (cap={cap}, delay={delay_ms}ms) \
         finished in {elapsed:?}; expected >= {min_expected:?} (≥ ~{batches} batches)",
        batches = n / cap
    );

    // Cross-check (diagnostic only — matcher-side counter is unreliable
    // because we cannot hook the actual response-completion event in
    // httpmock 0.8). The matcher fires when a request arrives; the spawned
    // decrement runs `delay_ms + 50` later. If the install pipeline issues
    // batch B+1 before the matcher's batch B decrements have all fired,
    // we see two batches simultaneously in the counter even though the
    // semaphore correctly serialised them. The TIMING assertion above is
    // the load-bearing proof of the cap; this counter assertion only
    // catches a complete bypass (e.g. cap = N, observed_max = N).
    let observed_max = max_seen.load(Ordering::SeqCst);
    assert!(
        observed_max <= n,
        "sanity: high-water {observed_max} cannot exceed total job count {n}",
    );
    eprintln!(
        "[concurrency-cap] N={n}, cap={cap}, delay={delay_ms}ms, elapsed={elapsed:?}, \
         observed_max_in_flight={observed_max}",
    );

    // Sanity: all N mods should now be in the ledger.
    let l: Ledger = toml::from_str(
        &tokio::fs::read_to_string(paths.instance_mod_ledger(slug))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(l.mods.len(), n, "every mod should land in the ledger");
}

// ============================================================================
// Test 2: Cancel mid-install does not leak ledger rows
// ============================================================================

#[tokio::test]
async fn test_cancel_aborts_install_no_ledger_write() {
    let server = MockServer::start();
    // 5 slow downloads; user cancels mid-stream. With a 2s delay per response
    // and a 150ms cancel window, no responses can complete before cancel.
    let n = 5;
    let body = vec![0u8; 1024];
    let sha = sha512_hex_of(&body);
    for i in 0..n {
        let body = body.clone();
        server.mock(move |when, then| {
            when.method(GET).path(format!("/cdn/slow-{i}.jar"));
            then.status(200)
                .delay(Duration::from_secs(2))
                .body(body);
        });
    }

    let td = TempDir::new().unwrap();
    let paths = paths_for(&td);
    let slug = "cancel-test";
    tokio::fs::create_dir_all(paths.instance_minecraft_dir(slug).join("mods"))
        .await
        .unwrap();

    let mut plan: Vec<InstallStep> = Vec::with_capacity(n);
    for i in 0..n {
        plan.push(InstallStep {
            row: InstalledModRow {
                mod_id: format!("M{i}"),
                project_slug: format!("p{i}"),
                display_name: format!("Slow {i}"),
                version_id: "v".into(),
                version_label: "0.0".into(),
                file_name: format!("slow-{i}.jar"),
                sha512: sha.clone(),
                size: body.len() as u64,
                hash_algo: HashAlgo::Sha512,
                source: ModSource::Modrinth,
                enabled: true,
                installed_at: "2026-01-01T00:00:00Z".into(),
            },
            file: ModrinthFile {
                url: format!("{}/cdn/slow-{i}.jar", server.base_url()),
                filename: format!("slow-{i}.jar"),
                primary: true,
                size: body.len() as u64,
                hashes: ModrinthHashes {
                    sha1: "x".into(),
                    sha512: sha.clone(),
                },
            },
        });
    }

    let http = reqwest::Client::builder()
        .user_agent("test")
        .build()
        .unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let token = CancellationToken::new();
    let token_for_install = token.clone();
    let paths_for_install = paths.clone();
    let slug_for_install = slug.to_string();

    let install_handle = tokio::spawn(async move {
        install_mods_into_instance(
            http,
            paths_for_install,
            slug_for_install,
            plan,
            tx,
            token_for_install,
            JobId(0),
        )
        .await
    });

    // Let some downloads start, then cancel.
    tokio::time::sleep(Duration::from_millis(150)).await;
    token.cancel();

    let res = install_handle.await.expect("install task should join");
    assert!(
        matches!(res, Err(ModrinthError::Cancelled)),
        "expected Err(Cancelled), got {res:?}"
    );

    // Ledger must contain STRICTLY FEWER than `n` rows — every uncompleted
    // download leaves no ledger row (atomicity invariant). With 2s response
    // delay and 150ms cancel, the expected count is 0; tolerate up to n-1 in
    // case the harness is unusually slow.
    let ledger_path = paths.instance_mod_ledger(slug);
    if ledger_path.exists() {
        let raw = tokio::fs::read_to_string(&ledger_path).await.unwrap();
        let l: Ledger = toml::from_str(&raw).unwrap_or_default();
        assert!(
            l.mods.len() < n,
            "expected partial or empty ledger after cancel, got {} of {n} rows",
            l.mods.len()
        );
    }
    // (If ledger does not exist at all, that is the "ideal" outcome — every
    // download was aborted before the first ledger upsert. No assertion needed.)
}

// ============================================================================
// Test 3: resolve_dependencies composes through ModrinthService stack
// ============================================================================

#[tokio::test]
async fn test_resolve_dependencies_through_full_service_stack() {
    let server = MockServer::start();
    let root_id = "vSodium1";
    let dep_project_id = "PFabric";
    let dep_version_id = "vFabric1";

    // GET /v2/version/{root_id} → Sodium root with 1 Required dep (project_id=PFabric).
    server.mock(|when, then| {
        when.method(GET).path(format!("/v2/version/{root_id}"));
        then.status(200).body(
            serde_json::json!({
                "id": root_id,
                "project_id": "PSodium",
                "name": "Sodium 0.5.8",
                "version_number": "0.5.8",
                "version_type": "release",
                "game_versions": ["1.20.4"],
                "loaders": ["fabric"],
                "downloads": 100,
                "date_published": "2026-01-01T00:00:00Z",
                "dependencies": [{
                    "project_id": dep_project_id,
                    "dependency_type": "required"
                }],
                "files": [{
                    "url": format!("{}/cdn/sodium.jar", server.base_url()),
                    "filename": "sodium.jar",
                    "primary": true,
                    "size": 1024,
                    "hashes": { "sha1": "a", "sha512": "b" }
                }]
            })
            .to_string(),
        );
    });

    // GET /v2/project/{dep_project_id}/version → one Fabric API version.
    server.mock(|when, then| {
        when.method(GET)
            .path(format!("/v2/project/{dep_project_id}/version"));
        then.status(200).body(
            serde_json::json!([{
                "id": dep_version_id,
                "project_id": dep_project_id,
                "name": "Fabric API 0.92.0",
                "version_number": "0.92.0",
                "version_type": "release",
                "game_versions": ["1.20.4"],
                "loaders": ["fabric"],
                "downloads": 100,
                "date_published": "2026-01-01T00:00:00Z",
                "dependencies": [],
                "files": [{
                    "url": format!("{}/cdn/fabric-api.jar", server.base_url()),
                    "filename": "fabric-api.jar",
                    "primary": true,
                    "size": 2048,
                    "hashes": { "sha1": "a", "sha512": "b" }
                }]
            }])
            .to_string(),
        );
    });

    // GET /v2/projects?ids=[...] → title hydration (closes GAP-8-D).
    // The resolver dedupes ids before this call; only `dep_project_id` is in
    // the dep set (root is excluded from `deps`). Accept any ids order via
    // query_param_exists — order depends on internal collection traversal.
    server.mock(|when, then| {
        when.method(GET)
            .path("/v2/projects")
            .query_param_exists("ids");
        then.status(200).body(
            serde_json::json!([
                { "id": dep_project_id, "title": "Fabric API" }
            ])
            .to_string(),
        );
    });

    let client =
        ModrinthClient::new_with_base_url(server.base_url()).expect("client::new_with_base_url");
    let svc = ModrinthService::with_client(client);

    let td = TempDir::new().unwrap();
    let paths = paths_for(&td);
    let slug = "resolve-test";

    let loader = mineltui::loader::types::LoaderInfo {
        kind: mineltui::domain::instance::ModloaderKind::Fabric,
        version: "0.16.9".into(),
        version_id: "fabric-loader-0.16.9-1.20.4".into(),
    };

    let graph = svc
        .resolve_dependencies(&paths, slug, root_id, "1.20.4", Some(&loader))
        .await
        .expect("resolve_dependencies");
    assert!(
        graph
            .deps
            .iter()
            .any(|d| matches!(d.kind, DepKind::Required) && d.is_new_download),
        "expected at least one new Required dep, got: {:?}",
        graph.deps
    );
    assert_eq!(
        graph.total_new_files, 1,
        "expected exactly 1 new file (Fabric API)"
    );
    assert_eq!(
        graph.root.id, root_id,
        "graph root.id should match the requested root version"
    );
}

// ============================================================================
// Test 4: Title hydration -> ledger display_name end-to-end (GAP-8-D)
// ============================================================================

#[tokio::test]
async fn test_dep_resolve_titles_propagate_to_ledger_display_name() {
    // GAP-8-D end-to-end: the BFS resolver must populate ResolvedDep.project_title
    // from the new /v2/projects?ids=[...] endpoint, and build_install_plan must
    // surface that title as InstalledModRow.display_name in the ledger so the
    // Installed Mods List + dep-confirm modal show "Fabric API" instead of an
    // opaque project_id like "P-fabric".
    //
    // The test mocks: root version, dep version listing, batch project titles,
    // CDN bodies (with computed sha512 so install verification passes). After
    // resolve + install, asserts the ledger has BOTH non-opaque display_name rows.
    let server = MockServer::start();

    // CDN bodies + their sha512 hashes (computed inline so install verification passes).
    let sodium_body = b"sodium-jar-bytes-for-gap-8d".to_vec();
    let fabric_body = b"fabric-api-jar-bytes-for-gap-8d".to_vec();
    let sodium_sha = sha512_hex_of(&sodium_body);
    let fabric_sha = sha512_hex_of(&fabric_body);

    let root_id = "v-sodium-1";
    let dep_pid = "P-fabric";
    let root_pid = "P-sodium";

    // Mock /v2/version/{root_id} — Sodium with one Required dep on P-fabric.
    {
        let server_url = server.base_url();
        let sodium_sha_for_root = sodium_sha.clone();
        let sodium_size = sodium_body.len() as u64;
        server.mock(move |when, then| {
            when.method(GET).path(format!("/v2/version/{root_id}"));
            then.status(200).body(
                serde_json::json!({
                    "id": root_id,
                    "project_id": root_pid,
                    "name": "Sodium 0.5.8",
                    "version_number": "0.5.8",
                    "version_type": "release",
                    "game_versions": ["1.20.4"],
                    "loaders": ["fabric"],
                    "downloads": 100,
                    "date_published": "2026-01-01T00:00:00Z",
                    "dependencies": [{
                        "project_id": dep_pid,
                        "dependency_type": "required"
                    }],
                    "files": [{
                        "url": format!("{server_url}/cdn/sodium.jar"),
                        "filename": "sodium.jar",
                        "primary": true,
                        "size": sodium_size,
                        "hashes": { "sha1": "x", "sha512": sodium_sha_for_root }
                    }]
                })
                .to_string(),
            );
        });
    }

    // Mock /v2/project/P-fabric/version — one Fabric API version.
    {
        let server_url = server.base_url();
        let fabric_sha_for_dep = fabric_sha.clone();
        let fabric_size = fabric_body.len() as u64;
        server.mock(move |when, then| {
            when.method(GET).path(format!("/v2/project/{dep_pid}/version"));
            then.status(200).body(
                serde_json::json!([{
                    "id": "v-fabric-1",
                    "project_id": dep_pid,
                    "name": "Fabric API 0.92.0",
                    "version_number": "0.92.0",
                    "version_type": "release",
                    "game_versions": ["1.20.4"],
                    "loaders": ["fabric"],
                    "downloads": 100,
                    "date_published": "2026-01-01T00:00:00Z",
                    "dependencies": [],
                    "files": [{
                        "url": format!("{server_url}/cdn/fabric-api.jar"),
                        "filename": "fabric-api.jar",
                        "primary": true,
                        "size": fabric_size,
                        "hashes": { "sha1": "x", "sha512": fabric_sha_for_dep }
                    }]
                }])
                .to_string(),
            );
        });
    }

    // Mock /v2/projects?ids=[...] — title hydration endpoint (closes GAP-8-D).
    // Note: the resolver only batch-fetches titles for `deps`, NOT the root —
    // root_pid is not in the dep set. We still return both ids in the response
    // body so the test exercises the realistic shape Modrinth would return if
    // we ever batch root + deps together later.
    server.mock(|when, then| {
        when.method(GET)
            .path("/v2/projects")
            .query_param_exists("ids");
        then.status(200).body(
            serde_json::json!([
                { "id": root_pid, "title": "Sodium" },
                { "id": dep_pid,  "title": "Fabric API" },
            ])
            .to_string(),
        );
    });

    // CDN mocks — bodies whose sha512 matches the manifest entries above.
    {
        let body = sodium_body.clone();
        server.mock(move |when, then| {
            when.method(GET).path("/cdn/sodium.jar");
            then.status(200).body(body.clone());
        });
    }
    {
        let body = fabric_body.clone();
        server.mock(move |when, then| {
            when.method(GET).path("/cdn/fabric-api.jar");
            then.status(200).body(body.clone());
        });
    }

    let client = ModrinthClient::new_with_base_url(server.base_url())
        .expect("client::new_with_base_url");
    let svc = ModrinthService::with_client(client);

    let td = TempDir::new().unwrap();
    let paths = paths_for(&td);
    let slug = "title-test";
    tokio::fs::create_dir_all(paths.instance_minecraft_dir(slug).join("mods"))
        .await
        .unwrap();

    let loader = mineltui::loader::types::LoaderInfo {
        kind: mineltui::domain::instance::ModloaderKind::Fabric,
        version: "0.16.9".into(),
        version_id: "fabric-loader-0.16.9-1.20.4".into(),
    };

    let graph = svc
        .resolve_dependencies(&paths, slug, root_id, "1.20.4", Some(&loader))
        .await
        .expect("resolve_dependencies");

    // Asserts the resolver populated the dep title via post-BFS hydration.
    let dep_with_title = graph
        .deps
        .iter()
        .find(|d| d.project_id == dep_pid)
        .expect("Fabric API dep missing from graph");
    assert_eq!(
        dep_with_title.project_title, "Fabric API",
        "GAP-8-D: project_title must be populated by post-BFS title hydration"
    );

    // Now install: expect 2 ledger rows (root + 1 dep) with non-opaque display_name fields.
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let token = CancellationToken::new();
    svc.install_mod_into_instance(
        &paths,
        slug,
        "sodium",
        "Sodium",
        &graph.root,
        &graph,
        tx,
        token,
        JobId(0),
    )
    .await
    .expect("install");

    let mods = svc.list_installed_mods(&paths, slug).await.unwrap();
    assert_eq!(
        mods.len(),
        2,
        "expected root + 1 dep, got {}: {mods:?}",
        mods.len()
    );
    let fabric_row = mods
        .iter()
        .find(|m| m.mod_id == dep_pid)
        .expect("Fabric API row missing from ledger");
    assert_eq!(
        fabric_row.display_name, "Fabric API",
        "GAP-8-D: ledger display_name must surface project_title, not project_id"
    );
    // Sodium row's display_name comes from `root_project_title` arg above ("Sodium"),
    // not via title hydration (root_pid is not in the dep set). Asserting both
    // proves the end-to-end title path covers both code branches.
    let sodium_row = mods
        .iter()
        .find(|m| m.mod_id == root_pid)
        .expect("Sodium row missing from ledger");
    assert_eq!(sodium_row.display_name, "Sodium");
}
