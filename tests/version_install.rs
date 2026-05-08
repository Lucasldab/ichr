//! Install orchestrator + natives extraction tests.

use std::io::Write;

use mineltui::domain::platform::{Arch, OsName};
use mineltui::install::natives_extract::extract_native_jar;
use mineltui::install::version_installer::{ASSET_CONCURRENCY, LIB_CONCURRENCY};
use mineltui::mojang::natives::needs_native_extraction;
use mineltui::mojang::rules::{evaluate_rules, RuleContext};
use mineltui::mojang::types::VersionJson;
use tempfile::tempdir;

fn build_test_jar(dest: &std::path::Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(dest).unwrap();
    let mut zw = zip::ZipWriter::new(file);
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        zw.start_file(*name, opts).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap();
}

#[tokio::test]
async fn test_extract_native_jar_basic() {
    let td = tempdir().unwrap();
    let jar = td.path().join("test.jar");
    let dest = td.path().join("extracted");

    let dll_bytes = b"fake dll content";
    build_test_jar(
        &jar,
        &[
            ("lwjgl.dll", dll_bytes),
            ("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\n"),
        ],
    );

    extract_native_jar(&jar, &dest, &["META-INF/".to_string()])
        .await
        .unwrap();

    // lwjgl.dll should be extracted
    assert!(
        dest.join("lwjgl.dll").exists(),
        "lwjgl.dll should be extracted"
    );
    let extracted = std::fs::read(dest.join("lwjgl.dll")).unwrap();
    assert_eq!(&extracted, dll_bytes, "extracted bytes should match");

    // META-INF entries should be excluded
    assert!(
        !dest.join("META-INF").exists(),
        "META-INF dir should NOT be created"
    );
    assert!(
        !dest.join("META-INF/MANIFEST.MF").exists(),
        "META-INF/MANIFEST.MF should NOT be extracted"
    );
}

#[tokio::test]
async fn test_extract_native_jar_rejects_path_traversal() {
    let td = tempdir().unwrap();
    let jar = td.path().join("traversal.jar");
    let dest = td.path().join("safe_dest");
    std::fs::create_dir_all(&dest).unwrap();

    // Entry that would traverse above dest
    build_test_jar(&jar, &[("../../../etc/passwd", b"root:x:0:0")]);

    // Should succeed (skips the traversal entry silently)
    extract_native_jar(&jar, &dest, &[]).await.unwrap();

    // dest dir should be empty — traversal entry was skipped
    let entries: Vec<_> = std::fs::read_dir(&dest).unwrap().collect();
    assert!(
        entries.is_empty(),
        "dest should remain clean after traversal attempt"
    );

    // The passwd file should NOT exist outside dest
    let victim = std::path::PathBuf::from("/etc/passwd_mineltui_test");
    assert!(!victim.exists());
}

#[tokio::test]
async fn test_extract_native_jar_skips_directories() {
    let td = tempdir().unwrap();
    let jar = td.path().join("dirs.jar");
    let dest = td.path().join("out");

    // Directory entry (trailing slash) plus a real file inside it
    build_test_jar(&jar, &[("subdir/", b""), ("subdir/real.so", b"elf binary")]);

    extract_native_jar(&jar, &dest, &[]).await.unwrap();

    // The file inside subdir should be extracted
    assert!(
        dest.join("subdir").join("real.so").exists(),
        "real.so should be extracted"
    );

    // The directory entry itself did not create a phantom entry
    assert!(
        dest.join("subdir").is_dir(),
        "subdir should exist as a directory"
    );
}

// ---------------------------------------------------------------------------
// Task 2-06-02 tests
// ---------------------------------------------------------------------------

#[test]
fn test_pick_libraries_filters_by_rules_on_linux_x86_64() {
    let json_str = include_str!("fixtures/mojang/version_1_21_4.json");
    let v: VersionJson = serde_json::from_str(json_str).expect("parse version_1_21_4.json");
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);

    let selected: Vec<_> = v
        .libraries
        .iter()
        .filter(|lib| evaluate_rules(&lib.rules, &ctx))
        .collect();

    // Some libraries are rule-excluded on linux/x86_64
    assert!(
        selected.len() < v.libraries.len(),
        "expected fewer selected than total: selected={} total={}",
        selected.len(),
        v.libraries.len()
    );

    // The osx-only library (ca.weblite:java-objc-bridge) should NOT appear
    let has_objc = selected
        .iter()
        .any(|lib| lib.name.contains("java-objc-bridge"));
    assert!(
        !has_objc,
        "osx-only library should be excluded on linux/x86_64"
    );
}

#[test]
fn test_pick_libraries_includes_rule_free_library() {
    let json_str = include_str!("fixtures/mojang/version_1_21_4.json");
    let v: VersionJson = serde_json::from_str(json_str).expect("parse version_1_21_4.json");
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);

    // A library with no rules must always be included.
    let rule_free: Vec<_> = v
        .libraries
        .iter()
        .filter(|lib| lib.rules.is_empty())
        .collect();
    assert!(
        !rule_free.is_empty(),
        "fixture should have at least one rule-free library"
    );

    for lib in &rule_free {
        assert!(
            evaluate_rules(&lib.rules, &ctx),
            "rule-free library '{}' should always be included",
            lib.name
        );
    }
}

#[test]
fn test_needs_extraction_on_legacy_version() {
    // 1.12.2 should have at least one classifier-style native that needs extraction.
    let json_str_112 = include_str!("fixtures/mojang/version_1_12_2.json");
    let v112: VersionJson = serde_json::from_str(json_str_112).expect("parse version_1_12_2.json");

    let legacy_native = v112
        .libraries
        .iter()
        .find(|lib| needs_native_extraction(lib));
    assert!(
        legacy_native.is_some(),
        "version 1.12.2 should have at least one library needing native extraction"
    );
    let legacy = legacy_native.unwrap();
    assert!(
        legacy.name.contains("lwjgl-platform") || needs_native_extraction(legacy),
        "legacy native lib should require extraction: {}",
        legacy.name
    );

    // 1.19.4 LWJGL embedded natives must NOT require extraction.
    let json_str_194 = include_str!("fixtures/mojang/version_1_19_4.json");
    let v194: VersionJson = serde_json::from_str(json_str_194).expect("parse version_1_19_4.json");

    let lwjgl_glfw = v194
        .libraries
        .iter()
        .find(|lib| lib.name.contains("lwjgl-glfw:3.3.1:natives-linux"));
    assert!(
        lwjgl_glfw.is_some(),
        "1.19.4 fixture should have org.lwjgl:lwjgl-glfw:3.3.1:natives-linux"
    );
    assert!(
        !needs_native_extraction(lwjgl_glfw.unwrap()),
        "1.19.4 embedded native should NOT require extraction"
    );
}

#[test]
fn test_concurrency_bounds_are_distinct_constants() {
    assert_eq!(LIB_CONCURRENCY, 8, "LIB_CONCURRENCY must be 8");
    assert_eq!(ASSET_CONCURRENCY, 16, "ASSET_CONCURRENCY must be 16");
    assert_ne!(
        LIB_CONCURRENCY, ASSET_CONCURRENCY,
        "LIB_CONCURRENCY and ASSET_CONCURRENCY must be distinct"
    );
}

#[test]
fn test_resolve_inherits_with_no_parent_is_noop() {
    use mineltui::mojang::inherits::resolve_inherits;
    use mineltui::mojang::types::{AssetIndex, VersionDownloads};
    use std::collections::HashMap;

    // Synthesize a VersionJson with no inheritsFrom.
    let version = VersionJson {
        id: "1.21.4".into(),
        version_type: "release".into(),
        main_class: "net.minecraft.client.main.Main".into(),
        asset_index: Some(AssetIndex {
            id: "21".into(),
            sha1: "abc".into(),
            size: 0,
            total_size: 0,
            url: "https://example.com/assets.json".into(),
        }),
        assets: Some("21".into()),
        downloads: Some(VersionDownloads::default()),
        libraries: vec![],
        java_version: None,
        logging: None,
        compliance_level: None,
        minimum_launcher_version: None,
        release_time: "2024-01-01T00:00:00+00:00".into(),
        time: "2024-01-01T00:00:00+00:00".into(),
        arguments: None,
        minecraft_arguments: None,
        inherits_from: None,
    };

    let parents: HashMap<String, VersionJson> = HashMap::new();
    let resolved = resolve_inherits(&version, &parents).unwrap();

    // When there is no parent, resolve_inherits returns the version unchanged
    // (post-merge ResolvedVersion has no `inherits_from` field — the chain is
    // resolved). For a vanilla input, root_id == id (the chain has length 1).
    assert_eq!(resolved.id, "1.21.4");
    assert_eq!(
        resolved.root_id, "1.21.4",
        "vanilla input has root_id == id (no inheritsFrom chain to walk)"
    );
    assert_eq!(resolved.main_class, "net.minecraft.client.main.Main");
    assert_eq!(
        resolved.asset_index.id, "21",
        "asset_index hydrated from input (no parent merge needed)"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_install_version_live_1_20_4() {
    use mineltui::install::install_version;
    use mineltui::mojang::client::MojangClient;
    use mineltui::persistence::paths::AppPaths;
    use mineltui::tasks::job::{JobId, TaskEvent};
    use tokio_util::sync::CancellationToken;

    let td = tempdir().unwrap();
    let paths = AppPaths::with_roots(
        td.path().join("data"),
        td.path().join("config"),
        td.path().join("cache"),
    );
    let client = MojangClient::new().unwrap();
    let manifest = client
        .fetch_manifest(&paths.cache_dir.join("manifest_v2.json"))
        .await
        .unwrap();
    let v = manifest
        .versions
        .iter()
        .find(|v| v.id == "1.20.4")
        .unwrap()
        .clone();

    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let token = CancellationToken::new();
    install_version(JobId(1), &paths, &client, tx, token, "test-slug", &v)
        .await
        .unwrap();

    // Drain events
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
    assert!(
        paths.version_jar("1.20.4").exists(),
        "client.jar should exist"
    );
    assert!(
        paths.version_json("1.20.4").exists(),
        "version.json should exist"
    );
}
