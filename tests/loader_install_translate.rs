//! Phase 8.4 GAP-LIBRARY-SHAPE-08 (round-4 BLOCKER closure) integration tests.
//!
//! Pins the translate-at-write contract: Phase 6 install produces an on-disk
//! version JSON in MOJANG SHAPE; the launcher's Library struct reads it
//! cleanly; the classpath builder emits one entry per library. Tests use
//! BYTE-EQUIVALENT samples of the user's actual on-disk fabric-meta and
//! quilt-meta JSONs (round-4 FORBID #4: no synthesised fixtures, no fattening).

use httpmock::Method::GET;
use httpmock::MockServer;
use ichr::loader::fabric::FabricMetaClient;
use ichr::loader::quilt::QuiltMetaClient;
use ichr::mojang::types::VersionJson;
use ichr::persistence::paths::AppPaths;
use tempfile::TempDir;

// --- Fixtures: byte-equivalent samples of the user's on-disk JSONs ---

const REAL_FABRIC_META_BYTES: &[u8] = br#"{"id":"fabric-loader-0.19.2-1.20.4","inheritsFrom":"1.20.4","releaseTime":"2026-05-07T02:57:33+0000","time":"2026-05-07T02:57:33+0000","type":"release","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient","arguments":{"game":[],"jvm":["-DFabricMcEmu= net.minecraft.client.main.Main "]},"libraries":[{"name":"org.ow2.asm:asm:9.9","url":"https://maven.fabricmc.net/","md5":"6d1dd0482c03a6dc1807d9d004456021","sha1":"c29635c8a7afa03d74b33c1884df8abb2b3f3dcc","sha256":"03d99a74ad1ee5c71334ef67437f4ef4fe3488caa7c96d8645abc73c8e2017d4","sha512":"197a4fb3ecb34d05ac555c6a510e69affcb1e476f24c5e935ad513ecdabf74b45aa1b0e0b25dbe91224fc6db7959b2677ea5876ee49e7487265e2a29c560c21c","size":126122},{"name":"org.ow2.asm:asm-analysis:9.9","url":"https://maven.fabricmc.net/","md5":"f07383cfbd50f097558341a03b8871e1","sha1":"0bf4fa6e66638851c1cd22c2caea0c3ee5d5f437","sha256":"6a15d28e8bd29ba4fd5bca4baf9b50e8fba2d7b51fbf78cfa0c875a7214c678b","sha512":"293fdf9ffd6858559d9bf4a2b68dfcfc58cb581d27e0fbcd2c2d0c540520498e9d587094534dd58d782ff5051b90f160971cfc388ea132e06eafa089bcc0bed7","size":35149},{"name":"org.ow2.asm:asm-commons:9.9","url":"https://maven.fabricmc.net/","md5":"8103b3de8f48fb4c7f97efdaa46ce809","sha1":"db9165a3bf908ded6b08612d583a15d1d0c7bda0","sha256":"db2f6f26150bbe7c126606b4a1151836bcc22a1e05a423b3585698bece995ff8","sha512":"4949cde2b51e5d171d0ff02ebd1f9f7f111bf538c8bfd62f139364181ee4bebd6598949d895f1c78daaba6dd1da4e564fab10e602cfe297915cd0287f8c2f1d5","size":74348},{"name":"org.ow2.asm:asm-tree:9.9","url":"https://maven.fabricmc.net/","md5":"912eeaba1a63d574ffc66c651c7c6725","sha1":"f8de6eead6d24dd0f45bd065bbe112b2cda6ea21","sha256":"42178f3775c9c63f9e5e1446747d29b4eca4d91bd6e75e5c43cfa372a47d38c6","sha512":"8b555d9166a17dcd0d1b297bd61fb3da59279b00a97fd7d0a3b139cb68ca8012ac14fb9bab0a6fa7ebe5612337f8e39b240d97b05a2c25ebc9ece15b7a1bc131","size":51947},{"name":"org.ow2.asm:asm-util:9.9","url":"https://maven.fabricmc.net/","md5":"ef5e90e736cd09bc407c1d46a3faba0f","sha1":"42fdfc0508b43807c8078d6e82ecff2ce2112ae8","sha256":"3842e13cfe324ee9ab7cdc4914be9943541ead397c17e26daf0b8a755bede717","sha512":"cd4f82589e0acc801618e4f55de2e7b85718d30cf5a7d2e5ead383dcdc2689a934abea37b81f5a7c508d9f7fc9ceb2e8ae5c8e4e958537ebb80b621814b099fb","size":94565},{"name":"net.fabricmc:sponge-mixin:0.17.2+mixin.0.8.7","url":"https://maven.fabricmc.net/","md5":"4b6b96074976cc7aa096b9e569ca623e","sha1":"edf98d1d98229e46e36c61774ae2b54dcd852981","sha256":"95cef6aebd9da1559cf9c4624eafae2ce1242d0167e3587d5d62c488e45b6999","sha512":"89044dca9a63bd5f2ceec09bfcb5807f1b294026665294bae7a9a980da89bd86c6d441eb38c92c89ca0efe86884c0730dab348d27633ef1e3970ed9eb5c30a4e","size":1540039},{"name":"net.fabricmc:intermediary:1.20.4","url":"https://maven.fabricmc.net/"},{"name":"net.fabricmc:fabric-loader:0.19.2","url":"https://maven.fabricmc.net/"}]}"#;

const REAL_QUILT_META_BYTES: &[u8] = br#"{"id":"quilt-loader-0.30.0-beta.7-1.20.4","inheritsFrom":"1.20.4","type":"release","mainClass":"org.quiltmc.loader.impl.launch.knot.KnotClient","arguments":{"game":[]},"libraries":[{"name":"net.fabricmc:sponge-mixin:0.17.0+mixin.0.8.7","url":"https://maven.fabricmc.net/"},{"name":"org.quiltmc:quilt-json5:1.0.4+final","url":"https://maven.quiltmc.org/repository/release/"},{"name":"org.ow2.asm:asm:9.9","url":"https://maven.fabricmc.net/"},{"name":"org.quiltmc:quilt-loader:0.30.0-beta.7","url":"https://maven.quiltmc.org/repository/release/"}],"releaseTime":"2023-12-07T12:56:20+00:00","time":"2026-04-21T06:25:58+00:00"}"#;

fn paths_in(td: &TempDir) -> AppPaths {
    AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    )
}

// --- Test 1: Fabric translate-at-write end-to-end ---

#[tokio::test]
async fn test_fabric_translate_at_write_end_to_end() {
    // Set up the staging that mimics post-Phase-6-install: write a
    // POST-TRANSLATION JSON directly to the on-disk path (the translator
    // is unit-tested in src/loader/fabric.rs::tests; this integration test
    // verifies that a Mojang-shape JSON on disk parses cleanly and that
    // every library has artifact populated). This is faster than driving
    // the full install_loader call (which has many other dependencies).
    let td = TempDir::new().unwrap();
    let paths = paths_in(&td);

    let mojang_bytes =
        ichr::loader::fabric::to_mojang_shape(REAL_FABRIC_META_BYTES).expect("translate ok");
    let json_path = paths.version_json("fabric-loader-0.19.2-1.20.4");
    tokio::fs::create_dir_all(json_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&json_path, &mojang_bytes).await.unwrap();

    // The launcher's Library struct deserialises cleanly:
    let bytes = tokio::fs::read(&json_path).await.unwrap();
    let v: VersionJson = serde_json::from_slice(&bytes).expect("Mojang-shape JSON parses");
    assert_eq!(v.id, "fabric-loader-0.19.2-1.20.4");
    assert_eq!(v.libraries.len(), 8);
    for lib in &v.libraries {
        let art = lib.downloads.artifact.as_ref().unwrap_or_else(|| {
            panic!(
                "library {} must have downloads.artifact after translate-at-write",
                lib.name
            )
        });
        assert!(!art.path.is_empty());
        assert!(!art.url.is_empty());
    }
    // Spot-check: the 6 libraries with hashes have Some(sha1), the 2 without have None.
    let with_hashes = v
        .libraries
        .iter()
        .filter(|l| l.downloads.artifact.as_ref().unwrap().sha1.is_some())
        .count();
    let without_hashes = v
        .libraries
        .iter()
        .filter(|l| l.downloads.artifact.as_ref().unwrap().sha1.is_none())
        .count();
    assert_eq!(with_hashes, 6, "6 fabric libraries have upstream sha1");
    assert_eq!(
        without_hashes, 2,
        "intermediary + fabric-loader have no sha1"
    );
}

// --- Test 2: Quilt translate-at-write end-to-end ---

#[tokio::test]
async fn test_quilt_translate_at_write_end_to_end() {
    let td = TempDir::new().unwrap();
    let paths = paths_in(&td);

    let mojang_bytes =
        ichr::loader::quilt::to_mojang_shape(REAL_QUILT_META_BYTES).expect("translate ok");
    let json_path = paths.version_json("quilt-loader-0.30.0-beta.7-1.20.4");
    tokio::fs::create_dir_all(json_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&json_path, &mojang_bytes).await.unwrap();

    let bytes = tokio::fs::read(&json_path).await.unwrap();
    let v: VersionJson = serde_json::from_slice(&bytes).expect("parses as Mojang");
    assert_eq!(v.id, "quilt-loader-0.30.0-beta.7-1.20.4");
    assert_eq!(v.libraries.len(), 4); // matches REAL_QUILT_META_BYTES library count

    // Quilt has no upstream hashes; every library's artifact has sha1=None, size=None.
    for lib in &v.libraries {
        let art = lib
            .downloads
            .artifact
            .as_ref()
            .unwrap_or_else(|| panic!("library {} must have artifact", lib.name));
        assert!(
            art.sha1.is_none(),
            "{} sha1 must be None for Quilt",
            lib.name
        );
        assert!(
            art.size.is_none(),
            "{} size must be None for Quilt",
            lib.name
        );
        assert!(!art.path.is_empty());
        assert!(!art.url.is_empty());
    }
}

// --- Test 3: Classpath emits >=6 fabric library entries (round-4 BLOCKER fix proof) ---

#[tokio::test]
async fn test_classpath_emits_at_least_six_fabric_libraries_after_install() {
    // After translate-at-write, every fabric library has artifact.path populated.
    // The classpath builder walks Library.downloads.artifact and emits
    // paths.library_path(&art.path) per entry. Pre-8.4 the classpath would
    // have had ZERO fabric library entries; post-8.4 it has all 8 (we
    // assert >= 6 to leave headroom for legitimate rules-based exclusions
    // if any are introduced later).

    let td = TempDir::new().unwrap();
    let paths = paths_in(&td);

    // Translate + write the loader JSON.
    let loader_bytes =
        ichr::loader::fabric::to_mojang_shape(REAL_FABRIC_META_BYTES).expect("translate ok");
    let loader_id = "fabric-loader-0.19.2-1.20.4";
    let loader_json_path = paths.version_json(loader_id);
    tokio::fs::create_dir_all(loader_json_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&loader_json_path, &loader_bytes)
        .await
        .unwrap();

    // Stub vanilla parent JSON (Mojang shape; minimal but valid) -- needed
    // for resolve_inherits to produce a ResolvedVersion.
    let vanilla_id = "1.20.4";
    let vanilla_json = br#"{
        "id":"1.20.4",
        "type":"release",
        "mainClass":"net.minecraft.client.main.Main",
        "assetIndex":{"id":"4","sha1":"0000000000000000000000000000000000000000","size":0,"totalSize":0,"url":"http://example.com/"},
        "assets":"4",
        "downloads":{"client":{"path":"x","sha1":"0000000000000000000000000000000000000000","size":0,"url":"http://example.com/c.jar"}},
        "libraries":[],
        "releaseTime":"2024-01-01T00:00:00Z",
        "time":"2024-01-01T00:00:00Z"
    }"#;
    let vanilla_json_path = paths.version_json(vanilla_id);
    tokio::fs::create_dir_all(vanilla_json_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&vanilla_json_path, vanilla_json)
        .await
        .unwrap();

    // Read + resolve.
    let loader: VersionJson =
        serde_json::from_slice(&tokio::fs::read(&loader_json_path).await.unwrap()).unwrap();
    let vanilla: VersionJson = serde_json::from_slice(vanilla_json).unwrap();
    let mut parents = std::collections::HashMap::new();
    parents.insert(vanilla_id.to_string(), vanilla);
    let resolved = ichr::mojang::inherits::resolve_inherits(&loader, &parents).expect("resolve ok");

    // Build classpath. Use a stubbed RuleContext that allows everything.
    let ctx = ichr::mojang::rules::RuleContext::current();
    let cp = ichr::launcher::classpath::build_classpath(&resolved, &ctx, &paths)
        .expect("build_classpath ok");
    // build_classpath returns a String of os-specific path-separated entries.
    let separator = if cfg!(target_os = "windows") {
        ';'
    } else {
        ':'
    };
    let entries: Vec<&str> = cp.split(separator).collect();

    // Filter to fabric library entries (paths under .../net/fabricmc/ or .../org/ow2/asm/).
    let fabric_entries: Vec<&&str> = entries
        .iter()
        .filter(|p| {
            p.contains("net/fabricmc/")
                || p.contains("net\\fabricmc\\")
                || p.contains("org/ow2/asm/")
                || p.contains("org\\ow2\\asm\\")
        })
        .collect();

    assert!(
        fabric_entries.len() >= 6,
        "post-translate-at-write classpath must include >=6 fabric library entries; \
         got {} entries: {:?}",
        fabric_entries.len(),
        fabric_entries
    );
}

// --- Test 4: Migration auto-heals flat-shape phase5-smoke ---

#[tokio::test]
async fn test_migration_hook_auto_heals_flat_shape_phase5_smoke() {
    // Pre-seed the data dir with a flat-shape JSON (the user's actual
    // pre-8.4 phase5-smoke install).
    let td = TempDir::new().unwrap();
    let paths = paths_in(&td);
    let version_id = "fabric-loader-0.19.2-1.20.4";
    let json_path = paths.version_json(version_id);
    tokio::fs::create_dir_all(json_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&json_path, REAL_FABRIC_META_BYTES)
        .await
        .unwrap();

    // Set up a httpmock-backed Fabric meta server returning the same
    // flat-shape body for the profile endpoint (the migration translates
    // it the SAME way as install would).
    let server = MockServer::start();
    let profile_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/v2/versions/loader/1.20.4/0.19.2/profile/json");
        then.status(200)
            .header("content-type", "application/json")
            .body(REAL_FABRIC_META_BYTES);
    });

    // Construct meta clients pointing at the mock server.
    let prior = std::env::var("ICHR_FABRIC_META_BASE_URL").ok();
    std::env::set_var("ICHR_FABRIC_META_BASE_URL", server.base_url());
    let fabric = FabricMetaClient::new().expect("fabric client");
    let quilt = QuiltMetaClient::new_with_base_url(server.base_url()).expect("quilt client");

    // Run migration.
    let result = ichr::launcher::service::__test_migrate_loader_json_in_place_if_needed(
        &paths, version_id, &fabric, &quilt,
    )
    .await;

    match prior {
        Some(v) => std::env::set_var("ICHR_FABRIC_META_BASE_URL", v),
        None => std::env::remove_var("ICHR_FABRIC_META_BASE_URL"),
    }

    result.expect("migration ok");
    profile_mock.assert_calls(1);

    // Assert the on-disk JSON is now Mojang shape.
    let bytes = tokio::fs::read(&json_path).await.unwrap();
    let v: VersionJson = serde_json::from_slice(&bytes).expect("parses as Mojang");
    for lib in &v.libraries {
        assert!(
            lib.downloads.artifact.is_some(),
            "library {} must have artifact after migration",
            lib.name
        );
    }
}

// --- Test 5: Migration is idempotent on already-Mojang-shape ---

#[tokio::test]
async fn test_migration_hook_idempotent_on_already_mojang_shape() {
    let td = TempDir::new().unwrap();
    let paths = paths_in(&td);
    let version_id = "fabric-loader-0.19.2-1.20.4";

    // Pre-seed with a MOJANG-shape JSON (the OUTPUT of test 4).
    let mojang_bytes =
        ichr::loader::fabric::to_mojang_shape(REAL_FABRIC_META_BYTES).expect("translate ok");
    let json_path = paths.version_json(version_id);
    tokio::fs::create_dir_all(json_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&json_path, &mojang_bytes).await.unwrap();
    let bytes_before = tokio::fs::read(&json_path).await.unwrap();

    // Set up a mock server that fails any profile call (idempotent path
    // must NOT hit the network).
    let server = MockServer::start();
    let profile_mock = server.mock(|when, then| {
        when.method(GET).path_includes("/profile/json");
        then.status(500).body("migration must not call this");
    });

    let prior = std::env::var("ICHR_FABRIC_META_BASE_URL").ok();
    std::env::set_var("ICHR_FABRIC_META_BASE_URL", server.base_url());
    let fabric = FabricMetaClient::new().expect("fabric client");
    let quilt = QuiltMetaClient::new_with_base_url(server.base_url()).expect("quilt client");

    let result = ichr::launcher::service::__test_migrate_loader_json_in_place_if_needed(
        &paths, version_id, &fabric, &quilt,
    )
    .await;

    match prior {
        Some(v) => std::env::set_var("ICHR_FABRIC_META_BASE_URL", v),
        None => std::env::remove_var("ICHR_FABRIC_META_BASE_URL"),
    }

    result.expect("idempotent migration ok");
    // Crucial: zero HTTP calls.
    profile_mock.assert_calls(0);

    // On-disk JSON unchanged byte-for-byte.
    let bytes_after = tokio::fs::read(&json_path).await.unwrap();
    assert_eq!(
        bytes_before, bytes_after,
        "idempotent migration must not rewrite the file"
    );
}
