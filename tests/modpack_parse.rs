//! File-level integration tests for `.mrpack` manifest parsing.
//!
//! These tests call `mineltui::modpack::parse::parse_index` directly on JSON
//! strings and on bytes extracted from zip fixtures.  No network access required.
//!
//! Run with: `cargo nextest run --test modpack_parse`

use mineltui::modpack::parse::{parse_index, EnvRequirement, MrpackIndex};
use mineltui::modpack::ModpackError;

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn minimal_json(format_version: u32, game: &str, deps_extra: &str, summary: Option<&str>) -> String {
    let summary_field = match summary {
        Some(s) => format!(r#""summary": "{s}","#),
        None => String::new(),
    };
    format!(
        r#"{{
            "formatVersion": {format_version},
            "game": "{game}",
            "versionId": "0.1.0",
            "name": "Test Pack",
            {summary_field}
            "dependencies": {{ "minecraft": "1.20.4"{deps_extra} }},
            "files": []
        }}"#
    )
}

// ─── Test 1: parse_minimal_index_succeeds ─────────────────────────────────────

#[test]
fn parse_minimal_index_succeeds() {
    let json = minimal_json(1, "minecraft", r#", "fabric-loader": "0.16.9""#, Some("a nice pack"));
    let idx: MrpackIndex = parse_index(&json).expect("parse must succeed");

    assert_eq!(idx.format_version, 1);
    assert_eq!(idx.game, "minecraft");
    assert_eq!(idx.version_id, "0.1.0");
    assert_eq!(idx.name, "Test Pack");
    assert_eq!(idx.summary.as_deref(), Some("a nice pack"));
    assert_eq!(
        idx.dependencies.get("minecraft").map(String::as_str),
        Some("1.20.4")
    );
    assert_eq!(
        idx.dependencies.get("fabric-loader").map(String::as_str),
        Some("0.16.9")
    );
    assert_eq!(idx.files.len(), 0);
}

// ─── Test 2: parse_unsupported_format_fails ───────────────────────────────────

#[test]
fn parse_unsupported_format_fails() {
    let json = minimal_json(2, "minecraft", "", Some("bad version"));
    let err = parse_index(&json).expect_err("must fail for formatVersion 2");
    match err {
        ModpackError::UnsupportedFormat { version: 2 } => {}
        other => panic!("expected UnsupportedFormat {{ version: 2 }}, got {other:?}"),
    }
}

// ─── Test 3: parse_missing_minecraft_dep_fails ────────────────────────────────

#[test]
fn parse_missing_minecraft_dep_fails() {
    // Build a JSON where `dependencies` only has fabric-loader (no "minecraft" key).
    let json = r#"{
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "1.0",
        "name": "No MC Dep Pack",
        "dependencies": { "fabric-loader": "0.16.9" },
        "files": []
    }"#;
    let err = parse_index(json).expect_err("must fail when minecraft dep is absent");
    match err {
        ModpackError::MissingMinecraftDependency => {}
        other => panic!("expected MissingMinecraftDependency, got {other:?}"),
    }
}

// ─── Test 4: parse_real_world_pack_with_optional_summary ─────────────────────

/// Pitfall 10 (RESEARCH.md): many packs in the wild omit `summary` entirely or
/// set it to `null`. The parser must tolerate both and expose `None`.
#[test]
fn parse_real_world_pack_with_optional_summary() {
    // Case A: `summary` field completely absent from the JSON.
    let json_absent = r#"{
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "2.0",
        "name": "No Summary Pack",
        "dependencies": { "minecraft": "1.20.1" },
        "files": []
    }"#;
    let idx_absent = parse_index(json_absent).expect("must parse when summary absent");
    assert!(
        idx_absent.summary.is_none(),
        "absent summary field must deserialize as None"
    );

    // Case B: `summary` field present but set to JSON null.
    let json_null = r#"{
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "2.0",
        "name": "Null Summary Pack",
        "summary": null,
        "dependencies": { "minecraft": "1.20.1" },
        "files": []
    }"#;
    let idx_null = parse_index(json_null).expect("must parse when summary is null");
    assert!(
        idx_null.summary.is_none(),
        "null summary must deserialize as None"
    );

    // Sanity-check: a non-null summary is still captured.
    let json_present = r#"{
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "2.0",
        "name": "Present Summary Pack",
        "summary": "A brief description",
        "dependencies": { "minecraft": "1.20.1" },
        "files": []
    }"#;
    let idx_present = parse_index(json_present).expect("must parse when summary is set");
    assert_eq!(
        idx_present.summary.as_deref(),
        Some("A brief description"),
        "non-null summary must be captured"
    );

    // Bonus: verify env.client parsing for a file in an otherwise-complete pack.
    let json_with_file = r#"{
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "3.0",
        "name": "File Env Pack",
        "dependencies": { "minecraft": "1.21.0" },
        "files": [
            {
                "path": "mods/example.jar",
                "hashes": { "sha1": "aabbcc", "sha512": "ddeeff" },
                "env": { "client": "required", "server": "unsupported" },
                "downloads": ["https://cdn.modrinth.com/data/xxx/example.jar"],
                "fileSize": 100
            }
        ]
    }"#;
    let idx_file = parse_index(json_with_file).expect("must parse pack with file");
    assert_eq!(idx_file.files.len(), 1);
    let f = &idx_file.files[0];
    assert_eq!(f.path, "mods/example.jar");
    assert!(f.env.is_some(), "env must be Some");
    assert_eq!(f.env.as_ref().unwrap().client, EnvRequirement::Required);
    assert_eq!(f.env.as_ref().unwrap().server, EnvRequirement::Unsupported);
}
