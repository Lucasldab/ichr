//! Domain tests for InstanceManifest and ModloaderKind.

use mineltui::domain::{InstanceManifest, ModloaderKind};

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
