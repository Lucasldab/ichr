//! Mojang protocol parse tests. Fixtures in `tests/fixtures/mojang/` are
//! frozen snapshots fetched live 2026-04-20 (see 02-01-...-SUMMARY.md).

use std::collections::HashMap;

use mineltui::mojang::{AssetIndexFile, Library, Rule, VersionJson, VersionManifest};
use mineltui::mojang::rules::RuleContext;
use mineltui::mojang::rules::evaluate_rules;
use mineltui::mojang::natives::needs_native_extraction;
use mineltui::mojang::inherits::resolve_inherits;
use mineltui::error::AppError;
use mineltui::domain::platform::{Arch, OsName};

// ---------------------------------------------------------------------------
// Task 2-02-01: Parse tests (tests 1–7)
// ---------------------------------------------------------------------------

#[test]
fn test_parse_manifest() {
    let raw = include_str!("./fixtures/mojang/version_manifest_v2_sample.json");
    let manifest: VersionManifest = serde_json::from_str(raw).expect("manifest must parse");
    assert!(!manifest.latest.release.is_empty(), "latest.release must be non-empty");
    assert!(manifest.versions.len() >= 10, "versions list must have >= 10 entries");
    assert_eq!(
        manifest.versions[0].sha1.len(),
        40,
        "first version sha1 must be 40 chars"
    );
    assert!(
        manifest.versions.iter().any(|v| v.version_type == "snapshot"),
        "manifest sample must contain at least one snapshot entry for VERS-02 regression"
    );
}

#[test]
fn test_parse_modern_version_json() {
    let raw = include_str!("./fixtures/mojang/version_1_21_4.json");
    let v: VersionJson = serde_json::from_str(raw).expect("1.21.4 must parse");
    assert_eq!(v.id, "1.21.4");
    assert_eq!(v.version_type, "release");
    assert_eq!(v.main_class, "net.minecraft.client.main.Main");
    assert!(v.arguments.is_some(), "1.21.4 must have structured arguments");
    assert!(v.minecraft_arguments.is_none(), "1.21.4 must not have legacy minecraftArguments");
    assert_eq!(
        v.java_version.as_ref().expect("javaVersion must be present").major_version,
        21
    );
    assert_eq!(v.asset_index.id, v.assets, "assetIndex.id must equal assets field");
}

#[test]
fn test_parse_legacy_version_json() {
    let raw = include_str!("./fixtures/mojang/version_1_12_2.json");
    let v: VersionJson = serde_json::from_str(raw).expect("1.12.2 must parse");
    assert_eq!(v.id, "1.12.2");
    assert_eq!(v.version_type, "release");
    assert!(v.arguments.is_none(), "1.12.2 must not have structured arguments");
    assert!(v.minecraft_arguments.is_some(), "1.12.2 must have legacy minecraftArguments");
    assert!(
        v.minecraft_arguments
            .as_ref()
            .unwrap()
            .contains("${auth_player_name}"),
        "minecraftArguments must contain the auth_player_name placeholder"
    );
}

#[test]
fn test_parse_asset_index_virtual() {
    let raw = include_str!("./fixtures/mojang/asset_index_1_6_4.json");
    let idx: AssetIndexFile = serde_json::from_str(raw).expect("asset_index_1_6_4 must parse");
    assert_eq!(idx.virtual_, Some(true), "1.6.4 asset index must have virtual == true");
    assert!(idx.objects.len() > 100, "1.6.4 asset index must have many objects");
}

#[test]
fn test_parse_asset_index_modern() {
    let raw = include_str!("./fixtures/mojang/asset_index_1_7_10.json");
    let idx: AssetIndexFile = serde_json::from_str(raw).expect("asset_index_1_7_10 must parse");
    assert!(idx.virtual_ != Some(true), "1.7.10 asset index must not have virtual=true");
    assert!(!idx.objects.is_empty(), "1.7.10 asset index must have objects");
}

#[test]
fn test_instance_manifest_unknown_fields_tolerated() {
    let json = r#"{
        "id": "test-version",
        "type": "release",
        "mainClass": "net.minecraft.Main",
        "assetIndex": {
            "id": "x",
            "sha1": "0000000000000000000000000000000000000000",
            "size": 0,
            "totalSize": 0,
            "url": "http://example.com/"
        },
        "assets": "x",
        "downloads": {},
        "libraries": [],
        "releaseTime": "2020-01-01T00:00:00Z",
        "time": "2020-01-01T00:00:00Z",
        "unknownFieldFromFutureVersion": true,
        "anotherUnknownField": { "nested": "value" }
    }"#;
    let v: VersionJson = serde_json::from_str(json).expect("unknown fields must not cause parse failure");
    assert_eq!(v.id, "test-version");
}

#[test]
fn test_parse_snapshot_version_json() {
    let raw = include_str!("./fixtures/mojang/version_snapshot_pinned.json");
    let v: VersionJson = serde_json::from_str(raw).expect("snapshot fixture must parse");
    // VERS-02: the parser must accept a snapshot without filtering
    assert_eq!(v.version_type, "snapshot", "pinned snapshot must have type == snapshot");
    assert!(!v.main_class.is_empty(), "mainClass must be non-empty in snapshot");
    assert!(
        !v.asset_index.sha1.is_empty(),
        "assetIndex.sha1 must be populated in snapshot"
    );
    // The snapshot uses either modern or legacy arg format — accept either
    assert!(
        v.arguments.is_some() || v.minecraft_arguments.is_some(),
        "snapshot must have either arguments or minecraftArguments"
    );
}

// ---------------------------------------------------------------------------
// Task 2-02-02: Rules, natives, inheritsFrom tests (tests 8–19)
// ---------------------------------------------------------------------------

/// Build a minimal Rule from a JSON literal — avoids hand-constructing serde_json::Value.
fn rule_from_json(json: &str) -> Rule {
    serde_json::from_str(json).expect("rule JSON must parse")
}

/// Build a minimal VersionJson stub for inheritsFrom tests.
fn vjson_stub(id: &str) -> VersionJson {
    serde_json::from_str(&format!(
        r#"{{
            "id": "{id}",
            "type": "release",
            "mainClass": "net.minecraft.client.main.Main",
            "assetIndex": {{
                "id": "x",
                "sha1": "0000000000000000000000000000000000000000",
                "size": 0,
                "totalSize": 0,
                "url": "http://example.com/"
            }},
            "assets": "x",
            "downloads": {{}},
            "libraries": [],
            "releaseTime": "2020-01-01T00:00:00Z",
            "time": "2020-01-01T00:00:00Z"
        }}"#
    ))
    .unwrap()
}

// Test 8
#[test]
fn test_library_rules_empty_array() {
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    assert!(evaluate_rules(&[], &ctx), "empty rules must return true (include)");
}

// Test 9
#[test]
fn test_library_rules_allow_linux_only_on_linux() {
    let rules = vec![rule_from_json(r#"{"action":"allow","os":{"name":"linux"}}"#)];
    let linux_ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    let windows_ctx = RuleContext::for_os_arch(OsName::Windows, Arch::X86_64);
    assert!(evaluate_rules(&rules, &linux_ctx), "linux-only rule must be true on linux");
    assert!(!evaluate_rules(&rules, &windows_ctx), "linux-only rule must be false on windows");
}

// Test 10
#[test]
fn test_library_rules_disallow_on_osx_means_include_on_linux() {
    let rules = vec![
        rule_from_json(r#"{"action":"allow"}"#),
        rule_from_json(r#"{"action":"disallow","os":{"name":"osx"}}"#),
    ];
    let linux_ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    // allow all, then disallow osx — on linux the disallow doesn't match, so result is allow
    assert!(
        evaluate_rules(&rules, &linux_ctx),
        "allow-all + disallow-osx must be true on linux"
    );
}

// Test 11
#[test]
fn test_library_rules_arch_x86_excludes_x86_64_system() {
    // Rule requires arch == "x86" (32-bit). Our system is x86_64, so it must NOT match.
    let rules = vec![rule_from_json(r#"{"action":"allow","os":{"arch":"x86"}}"#)];
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    assert!(
        !evaluate_rules(&rules, &ctx),
        "x86 rule must not match an x86_64 system"
    );
}

// Test 12
#[test]
fn test_needs_extraction_legacy_library() {
    let lib_json = r#"{
        "name": "org.lwjgl.lwjgl:lwjgl-platform:2.9.4",
        "downloads": {
            "classifiers": {
                "natives-linux": {
                    "path": "org/lwjgl/lwjgl/lwjgl-platform/2.9.4/lwjgl-platform-2.9.4-natives-linux.jar",
                    "sha1": "0000000000000000000000000000000000000000",
                    "size": 1234,
                    "url": "http://example.com/natives-linux.jar"
                }
            }
        },
        "rules": [],
        "natives": { "linux": "natives-linux", "windows": "natives-windows" }
    }"#;
    let lib: Library = serde_json::from_str(lib_json).expect("legacy library must parse");
    assert!(needs_native_extraction(&lib), "pre-1.19 library with natives+classifiers must need extraction");
}

// Test 13
#[test]
fn test_needs_extraction_embedded_native_library() {
    // 1.19+ embedded native: separate library entry with classifier-style name, no natives key
    let lib_json = r#"{
        "name": "org.lwjgl:lwjgl-glfw:3.3.1:natives-linux",
        "downloads": {
            "artifact": {
                "path": "org/lwjgl/lwjgl-glfw/3.3.1/lwjgl-glfw-3.3.1-natives-linux.jar",
                "sha1": "0000000000000000000000000000000000000000",
                "size": 5678,
                "url": "http://example.com/lwjgl-glfw-natives-linux.jar"
            }
        },
        "rules": [{"action":"allow","os":{"name":"linux"}}]
    }"#;
    let lib: Library = serde_json::from_str(lib_json).expect("embedded native library must parse");
    assert!(
        !needs_native_extraction(&lib),
        "1.19+ embedded-native library must NOT need extraction"
    );
}

// Test 14
#[test]
fn test_inherits_from_two_level_merge() {
    let parent_lib_json = r#"{
        "name": "com.example:parent-lib:1.0",
        "downloads": {},
        "rules": []
    }"#;
    let child_lib_json = r#"{
        "name": "com.example:child-lib:1.0",
        "downloads": {},
        "rules": []
    }"#;

    let mut parent = vjson_stub("parent-id");
    parent.libraries = vec![serde_json::from_str(parent_lib_json).unwrap()];
    parent.main_class = "A".to_string();
    parent.arguments = Some(serde_json::from_str(r#"{"game":["-g0"],"jvm":[]}"#).unwrap());

    let mut child = vjson_stub("child-id");
    child.inherits_from = Some("parent-id".to_string());
    child.main_class = "B".to_string();
    child.libraries = vec![serde_json::from_str(child_lib_json).unwrap()];
    child.arguments = Some(serde_json::from_str(r#"{"game":["-g1"],"jvm":[]}"#).unwrap());

    let mut parents = HashMap::new();
    parents.insert("parent-id".to_string(), parent);

    let merged = resolve_inherits(&child, &parents).expect("two-level merge must succeed");
    assert_eq!(merged.main_class, "B", "child mainClass must win");
    assert_eq!(merged.libraries.len(), 2, "merged libraries must have both entries");
    let game_args: Vec<String> = merged
        .arguments
        .unwrap()
        .game
        .iter()
        .filter_map(|e| {
            if let mineltui::mojang::ArgumentEntry::Plain(s) = e {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(game_args, vec!["-g0", "-g1"], "game args must be parent-first then child");
}

// Test 15
#[test]
fn test_inherits_from_library_dedup_by_group_artifact() {
    let parent_lib_json = r#"{"name":"org.example:lib:1.0","downloads":{},"rules":[]}"#;
    let child_lib_json = r#"{"name":"org.example:lib:2.0","downloads":{},"rules":[]}"#;

    let mut parent = vjson_stub("parent");
    parent.libraries = vec![serde_json::from_str(parent_lib_json).unwrap()];

    let mut child = vjson_stub("child");
    child.inherits_from = Some("parent".to_string());
    child.libraries = vec![serde_json::from_str(child_lib_json).unwrap()];

    let mut parents = HashMap::new();
    parents.insert("parent".to_string(), parent);

    let merged = resolve_inherits(&child, &parents).expect("dedup merge must succeed");
    assert_eq!(merged.libraries.len(), 1, "dedup by group:artifact must yield exactly 1 library");
    assert!(
        merged.libraries[0].name.contains("2.0"),
        "child version (2.0) must survive dedup"
    );
}

// Test 16
#[test]
fn test_inherits_from_cycle_detection() {
    let mut a = vjson_stub("A");
    a.inherits_from = Some("B".to_string());

    let mut b = vjson_stub("B");
    b.inherits_from = Some("A".to_string());

    let mut parents = HashMap::new();
    parents.insert("B".to_string(), b);
    // A is the child, but also put it in the map since B will try to look up A
    parents.insert("A".to_string(), a.clone());

    let result = resolve_inherits(&a, &parents);
    assert!(
        matches!(result, Err(AppError::InheritsFromCycle(_))),
        "cycle must yield InheritsFromCycle error, got: {result:?}"
    );
}

// Test 17
#[test]
fn test_inherits_from_depth_cap() {
    // Chain: A -> B -> C -> D (4 levels, exceeds MAX_DEPTH=3)
    let mut a = vjson_stub("A");
    a.inherits_from = Some("B".to_string());

    let mut b = vjson_stub("B");
    b.inherits_from = Some("C".to_string());

    let mut c = vjson_stub("C");
    c.inherits_from = Some("D".to_string());

    let d = vjson_stub("D");

    let mut parents = HashMap::new();
    parents.insert("B".to_string(), b);
    parents.insert("C".to_string(), c);
    parents.insert("D".to_string(), d);

    let result = resolve_inherits(&a, &parents);
    assert!(
        matches!(result, Err(AppError::InheritsFromDepthExceeded { .. })),
        "depth > 3 must yield InheritsFromDepthExceeded, got: {result:?}"
    );
}

// Test 18
#[test]
fn test_inherits_from_parent_missing_errors() {
    let mut child = vjson_stub("child");
    child.inherits_from = Some("ghost".to_string());

    let result = resolve_inherits(&child, &HashMap::new());
    assert!(
        matches!(result, Err(AppError::InheritsFromParentMissing(ref id)) if id == "ghost"),
        "missing parent must yield InheritsFromParentMissing(ghost), got: {result:?}"
    );
}

// Test 19
#[test]
fn test_inherits_from_no_parent_is_identity() {
    let child = vjson_stub("standalone");
    let result = resolve_inherits(&child, &HashMap::new()).expect("no inheritsFrom must succeed");
    assert_eq!(result.id, "standalone");
    assert_eq!(result.main_class, child.main_class);
}

// ---------------------------------------------------------------------------
// Task 2-02-03: Argument resolver tests (tests 20–26)
// ---------------------------------------------------------------------------

use mineltui::mojang::args::{resolve_game_args, resolve_jvm_args};

// Test 20
#[test]
fn test_resolve_game_args_1_21_4_flattens_structured_args() {
    let raw = include_str!("./fixtures/mojang/version_1_21_4.json");
    let v: VersionJson = serde_json::from_str(raw).unwrap();
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    let result = resolve_game_args(&v, &ctx);
    assert!(!result.is_empty(), "1.21.4 game args must be non-empty");
    assert!(
        result.contains(&"--username".to_string()),
        "result must contain --username; got: {result:?}"
    );
    assert!(result.contains(&"--version".to_string()), "result must contain --version");
}

// Test 21
#[test]
fn test_resolve_game_args_1_12_2_splits_minecraft_arguments_on_whitespace() {
    let raw = include_str!("./fixtures/mojang/version_1_12_2.json");
    let v: VersionJson = serde_json::from_str(raw).unwrap();
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    let result = resolve_game_args(&v, &ctx);
    assert!(result.len() > 10, "1.12.2 split args must have many tokens; got {}", result.len());
    assert!(
        result.iter().any(|s| s == "--username"),
        "result must contain --username; got: {result:?}"
    );
    assert!(
        result.iter().any(|s| s == "${auth_player_name}"),
        "result must contain auth_player_name placeholder"
    );
}

// Test 22
#[test]
fn test_resolve_jvm_args_1_21_4_os_linux_x86_64_contains_classpath_placeholder() {
    let raw = include_str!("./fixtures/mojang/version_1_21_4.json");
    let v: VersionJson = serde_json::from_str(raw).unwrap();
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    let result = resolve_jvm_args(&v, &ctx);
    assert!(
        result.contains(&"${classpath}".to_string()),
        "JVM args for linux/x86_64 must contain classpath placeholder; got: {result:?}"
    );
    assert!(
        !result.contains(&"-XstartOnFirstThread".to_string()),
        "JVM args for linux must NOT contain osx-only -XstartOnFirstThread"
    );
}

// Test 23
#[test]
fn test_resolve_jvm_args_1_21_4_unique_token_count_under_80() {
    let raw = include_str!("./fixtures/mojang/version_1_21_4.json");
    let v: VersionJson = serde_json::from_str(raw).unwrap();
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    let result = resolve_jvm_args(&v, &ctx);
    assert!(
        result.len() <= 20,
        "rule filtering must produce a reasonable JVM arg count; got {}",
        result.len()
    );
}

// Test 24
#[test]
fn test_resolve_arguments_unknown_feature_flag_defaults_disallow() {
    use std::collections::HashSet;
    let args_json = r#"{
        "game": [{
            "rules": [{"action": "allow", "features": {"is_demo_user": true}}],
            "value": "--demo"
        }],
        "jvm": []
    }"#;
    let arguments: mineltui::mojang::Arguments = serde_json::from_str(args_json).unwrap();
    let mut v = vjson_stub("synthetic");
    v.arguments = Some(arguments);
    let ctx = RuleContext {
        os: OsName::Linux,
        arch: Arch::X86_64,
        features: HashSet::new(),
    };
    let result = resolve_game_args(&v, &ctx);
    assert!(
        !result.contains(&"--demo".to_string()),
        "unknown feature flag must NOT produce --demo; got: {result:?}"
    );
}

// Test 25
#[test]
fn test_resolve_arguments_prefers_arguments_struct_over_legacy_string() {
    let args_json = r#"{"game":["--structured-arg"],"jvm":[]}"#;
    let arguments: mineltui::mojang::Arguments = serde_json::from_str(args_json).unwrap();
    let mut v = vjson_stub("synthetic");
    v.arguments = Some(arguments);
    v.minecraft_arguments = Some("--legacy-arg".to_string());
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    let result = resolve_game_args(&v, &ctx);
    assert!(
        result.contains(&"--structured-arg".to_string()),
        "structured arguments must be returned when both formats present"
    );
    assert!(
        !result.contains(&"--legacy-arg".to_string()),
        "legacy minecraftArguments must NOT be used when arguments struct is present"
    );
}

// Test 26
#[test]
fn test_resolve_arguments_missing_both_returns_empty() {
    let mut v = vjson_stub("empty");
    v.arguments = None;
    v.minecraft_arguments = None;
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    let game = resolve_game_args(&v, &ctx);
    let jvm = resolve_jvm_args(&v, &ctx);
    assert!(game.is_empty(), "game args must be empty when both None");
    assert!(jvm.is_empty(), "jvm args must be empty when both None");
}
