//! Mojang protocol parse tests. Fixtures in `tests/fixtures/mojang/` are
//! frozen snapshots fetched live 2026-04-20 (see 02-01-...-SUMMARY.md).

use mineltui::mojang::{AssetIndexFile, VersionJson, VersionManifest};

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
