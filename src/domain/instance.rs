//! Instance domain types.
//!
//! The canonical on-disk form is `InstanceManifest` serialized as JSON
//! at `{data_dir}/instances/{slug}/instance.json`. See 02-RESEARCH.md
//! §"`instance.json` Schema" for schema; PITFALLS.md pitfall 5 for
//! the forward-compatibility rule (no deny_unknown_fields).

use serde::{Deserialize, Serialize};

use crate::java::types::JavaRuntimeId;
use crate::loader::types::LoaderInfo;

/// Opaque instance identifier. Equal to the slug string for v1.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InstanceId(pub String);

impl std::fmt::Display for InstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for InstanceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Modloader variant for an instance. Vanilla is the default (no modloader).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModloaderKind {
    #[default]
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    /// `rename_all = "snake_case"` would produce `"neo_forge"`; the explicit rename
    /// aligns this with `LoaderType::NeoForge` at the wire level (PATTERNS.md gotcha #1).
    #[serde(rename = "neoforge")]
    NeoForge,
}

/// On-disk instance manifest. Version 1 schema.
///
/// Unknown fields are tolerated (never use `#[serde(deny_unknown_fields)]`).
/// `None` Options are omitted during serialization so old launchers don't
/// see fields they can't parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceManifest {
    /// Schema version. Always 1 for v1 instances; bump on breaking change.
    pub schema_version: u32,

    /// Human-readable name as entered by the user (UI surface).
    pub display_name: String,

    /// Filesystem-safe identifier. Stable after creation — rename does NOT mutate slug.
    pub slug: String,

    /// Minecraft version id, e.g. "1.21.4".
    pub mc_version_id: String,

    /// ISO 8601 UTC timestamp when the instance was created.
    pub created_at: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played_at: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,

    /// Optional tag/folder for UI grouping (INST-06).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,

    /// Per-instance Java runtime override. When `None`, the launcher uses the
    /// Mojang-recommended JRE for the Minecraft version. Set by the user via
    /// the instance settings UI (Phase 5+).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_override: Option<JavaRuntimeId>,

    /// Active modloader for this instance. `None` = vanilla.
    /// Written last by `LoaderService::install_loader` (atomicity invariant
    /// per 06-RESEARCH.md Pitfall 7) so a partial install never leaves an
    /// `instance.loader` pointing at a missing version JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<LoaderInfo>,

    #[serde(default)]
    pub total_play_time_ms: u64,
}

impl InstanceManifest {
    /// Convenience: construct a freshly-created manifest with sensible defaults.
    ///
    /// `display_name` and `slug` are caller-provided (caller runs slugify/unique_slug
    /// before calling). `created_at` is set to the current UTC time in RFC3339.
    pub fn new(display_name: String, slug: String, mc_version_id: String) -> Self {
        Self {
            schema_version: 1,
            display_name,
            slug,
            mc_version_id,
            created_at: now_iso8601_utc(),
            last_played_at: None,
            notes: None,
            group: None,
            java_override: None,
            loader: None,
            total_play_time_ms: 0,
        }
    }
}

/// RFC3339/ISO-8601 UTC "2026-04-20T09:00:00Z" — seconds precision is enough.
pub(crate) fn now_iso8601_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let dt = time::OffsetDateTime::from_unix_timestamp(secs as i64)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    dt.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| format!("{secs}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_java_override_backward_compat() {
        // Legacy instance.json without java_override must deserialize to None.
        let json = r#"{
            "schema_version": 1,
            "display_name": "Test",
            "slug": "test",
            "mc_version_id": "1.21.4",
            "created_at": "2026-01-01T00:00:00Z",
            "total_play_time_ms": 0
        }"#;
        let m: InstanceManifest = serde_json::from_str(json).unwrap();
        assert!(
            m.java_override.is_none(),
            "legacy instance should have None java_override"
        );
    }

    #[test]
    fn test_java_override_none_not_serialized() {
        let m = InstanceManifest::new("Test".into(), "test".into(), "1.21.4".into());
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            !json.contains("java_override"),
            "None override must not appear in JSON: {json}"
        );
    }

    #[test]
    fn test_java_override_roundtrip_mojang() {
        let mut m = InstanceManifest::new("Test".into(), "test".into(), "1.21.4".into());
        m.java_override = Some(JavaRuntimeId::Mojang {
            variant: "java-runtime-delta".into(),
        });
        let json = serde_json::to_string(&m).unwrap();
        let parsed: InstanceManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.java_override,
            Some(JavaRuntimeId::Mojang {
                variant: "java-runtime-delta".into()
            })
        );
    }

    #[test]
    fn test_loader_field_backward_compat() {
        // Legacy instance.json without `loader` must deserialize to None.
        let json = r#"{
            "schema_version": 1,
            "display_name": "Test",
            "slug": "test",
            "mc_version_id": "1.21.4",
            "created_at": "2026-01-01T00:00:00Z",
            "total_play_time_ms": 0
        }"#;
        let m: InstanceManifest = serde_json::from_str(json).unwrap();
        assert!(
            m.loader.is_none(),
            "legacy instance should have None loader"
        );
    }

    #[test]
    fn test_loader_none_not_serialized() {
        let m = InstanceManifest::new("Test".into(), "test".into(), "1.21.4".into());
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            !json.contains("loader"),
            "None loader must not appear in JSON: {json}"
        );
    }

    #[test]
    fn test_loader_field_roundtrip_fabric() {
        let mut m = InstanceManifest::new("Test".into(), "test".into(), "1.21.4".into());
        m.loader = Some(LoaderInfo {
            kind: ModloaderKind::Fabric,
            version: "0.16.9".into(),
            version_id: "fabric-loader-0.16.9-1.21.4".into(),
        });
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            json.contains("\"kind\":\"fabric\""),
            "fabric kind should serialize snake_case: {json}"
        );
        assert!(
            json.contains("\"version_id\":\"fabric-loader-0.16.9-1.21.4\""),
            "version_id missing: {json}"
        );
        let parsed: InstanceManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.loader, m.loader);
    }

    #[test]
    fn test_loader_field_roundtrip_quilt() {
        let mut m = InstanceManifest::new("Test".into(), "test".into(), "1.21.4".into());
        m.loader = Some(LoaderInfo {
            kind: ModloaderKind::Quilt,
            version: "0.30.0-beta.7".into(),
            version_id: "quilt-loader-0.30.0-beta.7-1.21.4".into(),
        });
        let json = serde_json::to_string(&m).unwrap();
        let parsed: InstanceManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.loader, m.loader);
        assert!(
            json.contains("\"kind\":\"quilt\""),
            "quilt kind missing: {json}"
        );
    }
}
