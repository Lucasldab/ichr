//! Domain tests for InstanceManifest, ModloaderKind, slug functions, and store I/O.

use mineltui::domain::{InstanceManifest, ModloaderKind};
use mineltui::instance::{
    list_instance_manifests, read_instance_manifest, slugify, unique_slug,
    write_instance_manifest,
};
use mineltui::persistence::AppPaths;
use tempfile::tempdir;

fn sample_manifest() -> InstanceManifest {
    InstanceManifest {
        schema_version: 1,
        display_name: "My Instance".to_string(),
        slug: "my-instance".to_string(),
        mc_version_id: "1.21.4".to_string(),
        created_at: "2026-04-20T00:00:00Z".to_string(),
        last_played_at: Some("2026-04-21T12:00:00Z".to_string()),
        notes: Some("Test notes".to_string()),
        group: Some("survival".to_string()),
        java_override: None,
        total_play_time_ms: 3600000,
    }
}

#[test]
fn test_instance_manifest_round_trip() {
    let m = sample_manifest();
    let json = serde_json::to_string(&m).expect("serialize");
    let m2: InstanceManifest = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(m, m2);
}

#[test]
fn test_instance_manifest_omits_none_on_serialize() {
    let m = InstanceManifest {
        schema_version: 1,
        display_name: "Minimal".to_string(),
        slug: "minimal".to_string(),
        mc_version_id: "1.20.1".to_string(),
        created_at: "2026-04-20T00:00:00Z".to_string(),
        last_played_at: None,
        notes: None,
        group: None,
        java_override: None,
        total_play_time_ms: 0,
    };
    let json = serde_json::to_string(&m).expect("serialize");
    assert!(!json.contains("last_played_at"), "JSON should not contain last_played_at: {json}");
    assert!(!json.contains("notes"), "JSON should not contain notes: {json}");
    assert!(!json.contains("group"), "JSON should not contain group: {json}");
}

#[test]
fn test_instance_manifest_unknown_fields_tolerated() {
    let raw = r#"{"schema_version":1,"display_name":"Test","slug":"test","mc_version_id":"1.21.4","created_at":"2026-04-20T00:00:00Z","total_play_time_ms":0,"future_field":"ignored"}"#;
    let m: InstanceManifest = serde_json::from_str(raw).expect("should not fail on unknown fields");
    assert_eq!(m.display_name, "Test");
}

#[test]
fn test_instance_manifest_defaults_for_missing_options() {
    let raw = r#"{"schema_version":1,"display_name":"Minimal","slug":"minimal","mc_version_id":"1.21.4","created_at":"2026-04-20T00:00:00Z"}"#;
    let m: InstanceManifest = serde_json::from_str(raw).expect("deserialize with missing optional fields");
    assert_eq!(m.last_played_at, None);
    assert_eq!(m.notes, None);
    assert_eq!(m.group, None);
    assert_eq!(m.total_play_time_ms, 0);
}

#[test]
fn test_modloader_kind_vanilla_default() {
    assert_eq!(ModloaderKind::default(), ModloaderKind::Vanilla);
}

// ─── Slug tests ──────────────────────────────────────────────────────────────

#[test]
fn test_slugify_basic_lowercase_and_dashes() {
    assert_eq!(slugify("My Instance"), "my-instance");
}

#[test]
fn test_slugify_unicode_and_punctuation() {
    // é is non-ASCII → stripped; & and ! stripped; whitespace → dash; dashes collapsed
    assert_eq!(slugify("Café & Co!"), "caf-co");
}

#[test]
fn test_slugify_collapses_multiple_dashes() {
    assert_eq!(slugify("a--b---c"), "a-b-c");
}

#[test]
fn test_slugify_trims_leading_trailing_dashes() {
    assert_eq!(slugify("  --foo--  "), "foo");
}

#[test]
fn test_slugify_truncates_to_40() {
    let result = slugify(&"a".repeat(60));
    assert_eq!(result.len(), 40);
}

#[test]
fn test_slugify_empty_input_returns_instance() {
    assert_eq!(slugify(""), "instance");
}

#[test]
fn test_slugify_all_punctuation_returns_instance() {
    assert_eq!(slugify("!!!"), "instance");
}

// ─── unique_slug tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_unique_slug_no_collision() {
    let td = tempdir().unwrap();
    let result = unique_slug("foo", td.path()).await;
    assert_eq!(result, "foo");
}

#[tokio::test]
async fn test_unique_slug_single_collision() {
    let td = tempdir().unwrap();
    tokio::fs::create_dir(td.path().join("foo")).await.unwrap();
    let result = unique_slug("foo", td.path()).await;
    assert_eq!(result, "foo-2");
}

#[tokio::test]
async fn test_unique_slug_chain_collision() {
    let td = tempdir().unwrap();
    tokio::fs::create_dir(td.path().join("foo")).await.unwrap();
    tokio::fs::create_dir(td.path().join("foo-2")).await.unwrap();
    let result = unique_slug("foo", td.path()).await;
    assert_eq!(result, "foo-3");
}

// ─── Store tests ──────────────────────────────────────────────────────────────

fn make_paths(td: &tempfile::TempDir) -> AppPaths {
    AppPaths::with_roots(
        td.path().join("data"),
        td.path().join("config"),
        td.path().join("cache"),
    )
}

#[tokio::test]
async fn test_write_then_read_instance_manifest_round_trip() {
    let td = tempdir().unwrap();
    let paths = make_paths(&td);
    let m = InstanceManifest {
        schema_version: 1,
        display_name: "Round Trip".to_string(),
        slug: "round-trip".to_string(),
        mc_version_id: "1.21.4".to_string(),
        created_at: "2026-04-20T00:00:00Z".to_string(),
        last_played_at: None,
        notes: None,
        group: None,
        java_override: None,
        total_play_time_ms: 0,
    };
    write_instance_manifest(&paths, &m).await.unwrap();
    let m2 = read_instance_manifest(&paths, &m.slug).await.unwrap();
    assert_eq!(m, m2);
}

#[tokio::test]
async fn test_list_instance_manifests_skips_invalid_and_sorts_by_last_played_desc() {
    let td = tempdir().unwrap();
    let paths = make_paths(&td);

    let make = |slug: &str, display: &str, last_played: Option<&str>| InstanceManifest {
        schema_version: 1,
        display_name: display.to_string(),
        slug: slug.to_string(),
        mc_version_id: "1.21.4".to_string(),
        created_at: "2026-04-20T00:00:00Z".to_string(),
        last_played_at: last_played.map(|s| s.to_string()),
        notes: None,
        group: None,
        java_override: None,
        total_play_time_ms: 0,
    };

    let a = make("instance-a", "A", Some("2024-01-01T00:00:00Z"));
    let b = make("instance-b", "B", None);
    let c = make("instance-c", "C", Some("2024-06-01T00:00:00Z"));

    write_instance_manifest(&paths, &a).await.unwrap();
    write_instance_manifest(&paths, &b).await.unwrap();
    write_instance_manifest(&paths, &c).await.unwrap();

    // Write a bad instance.json that should be skipped
    let bad_dir = paths.instance_dir("bad-instance");
    tokio::fs::create_dir_all(&bad_dir).await.unwrap();
    tokio::fs::write(bad_dir.join("instance.json"), b"not valid json").await.unwrap();

    let list = list_instance_manifests(&paths).await.unwrap();
    // Should return 3 valid manifests (bad one skipped), sorted C, A, B
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].slug, "instance-c");
    assert_eq!(list[1].slug, "instance-a");
    assert_eq!(list[2].slug, "instance-b");
}

#[tokio::test]
async fn test_write_is_atomic_no_partial_file_visible() {
    let td = tempdir().unwrap();
    let paths = make_paths(&td);
    let m = InstanceManifest {
        schema_version: 1,
        display_name: "Atomic".to_string(),
        slug: "atomic".to_string(),
        mc_version_id: "1.21.4".to_string(),
        created_at: "2026-04-20T00:00:00Z".to_string(),
        last_played_at: None,
        notes: None,
        group: None,
        java_override: None,
        total_play_time_ms: 0,
    };
    write_instance_manifest(&paths, &m).await.unwrap();
    // No .tmp file should remain
    let instance_dir = paths.instance_dir(&m.slug);
    let tmp_path = paths.instance_manifest(&m.slug).with_extension("tmp");
    assert!(!tmp_path.exists(), ".tmp file should not exist after successful write");
    // The real file should exist
    assert!(instance_dir.join("instance.json").exists());
}
