//! Java runtime identifier variants. Used as the serde tag inside
//! `InstanceManifest.java_override`.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Identifies which Java runtime to use for a specific instance.
///
/// Serde-tagged with `"type"` field for forward-compatible JSON storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JavaRuntimeId {
    /// Mojang-managed JRE — `variant` is the `component` string from
    /// `VersionJson.java_version.component` (e.g. "java-runtime-delta").
    /// The variant list grows over time — never hardcode it.
    Mojang { variant: String },
    /// Adoptium JRE keyed on Java major version (8, 17, 21, 25, ...).
    Adoptium { major: u32 },
    /// User-selected system Java at an absolute path.
    /// `major_version` is captured at detection time.
    System { path: PathBuf, major_version: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mojang_variant_serde() {
        let id = JavaRuntimeId::Mojang {
            variant: "java-runtime-delta".into(),
        };
        let json = serde_json::to_string(&id).unwrap();
        assert!(
            json.contains("\"type\":\"mojang\""),
            "missing type tag in: {json}"
        );
        assert!(
            json.contains("\"variant\":\"java-runtime-delta\""),
            "missing variant in: {json}"
        );
        let roundtrip: JavaRuntimeId = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, id);
    }

    #[test]
    fn test_adoptium_roundtrip() {
        let id = JavaRuntimeId::Adoptium { major: 21 };
        let json = serde_json::to_string(&id).unwrap();
        let roundtrip: JavaRuntimeId = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, JavaRuntimeId::Adoptium { major: 21 });
    }

    #[test]
    fn test_system_roundtrip() {
        let id = JavaRuntimeId::System {
            path: PathBuf::from("/usr/bin/java"),
            major_version: 17,
        };
        let json = serde_json::to_string(&id).unwrap();
        let roundtrip: JavaRuntimeId = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, id);
    }
}
