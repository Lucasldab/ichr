//! Loader domain types -- pure data, no I/O.

use serde::{Deserialize, Serialize};

use crate::domain::instance::ModloaderKind;

/// Modloader family for which the loader install pipeline runs.
/// Distinct from `ModloaderKind` (which also has Vanilla/Forge/NeoForge);
/// `LoaderType` enumerates all active loader pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderType {
    Fabric,
    Quilt,
    Forge,
    /// `rename_all = "snake_case"` would produce `"neo_forge"`; the explicit rename
    /// aligns this with `ModloaderKind::NeoForge` at the wire level (PATTERNS.md gotcha #1).
    #[serde(rename = "neoforge")]
    NeoForge,
}

/// One row in the loader version picker.
/// `build` carries Quilt's integer ordering field; Fabric does not use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoaderVersionEntry {
    pub version: String,
    pub stable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<u32>,
}

/// One library entry from a loader profile JSON.
/// Canonical shared shape used by BOTH Fabric and Quilt clients --
/// fabric.rs and quilt.rs import this type directly (no per-loader
/// duplication). Fabric profiles populate `sha1`/`sha256`/`size`;
/// Quilt profiles always parse with all hash fields as `None`.
///
/// `url` is `Option<String>` because some Fabric/Quilt profile entries
/// omit the field (the loader's own JAR is sometimes published without
/// an explicit `url`, falling back to the loader's default Maven repo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoaderLibrary {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Active modloader recorded on `InstanceManifest.loader`.
/// Written last by `LoaderService::install_loader` (atomicity invariant).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoaderInfo {
    pub kind: ModloaderKind,
    pub version: String,
    pub version_id: String,
}

/// MC version compatibility for Forge (07-CONTEXT.md D-05: 1.13+ only).
///
/// Modern Forge installer architecture is post-1.13; pre-1.13 Forge uses
/// a fundamentally different installer flow that is out of scope for v1.
/// Uses string-prefix matching -- Mojang version IDs are not strict semver.
pub fn forge_supported_for_mc(mc: &str) -> bool {
    const SUPPORTED_PREFIXES: &[&str] = &[
        "1.13", "1.14", "1.15", "1.16", "1.17", "1.18", "1.19", "1.20", "1.21", "1.22", "1.23",
        "1.24", "1.25",
    ];
    SUPPORTED_PREFIXES.iter().any(|p| mc.starts_with(p))
}

/// MC version compatibility for NeoForge (07-CONTEXT.md D-05: 1.20.1+ only).
///
/// NeoForge forked Forge at MC 1.20.1; pre-1.20.1 is Forge-only.
/// Uses string-prefix matching -- Mojang version IDs are not strict semver.
pub fn neoforge_supported_for_mc(mc: &str) -> bool {
    // Explicit prefix list from 1.20.1 onward. Uses prefix matches to handle
    // patches and pre-releases (e.g., "1.21-pre3" matches "1.21").
    const SUPPORTED_PREFIXES: &[&str] = &[
        "1.20.1", "1.20.2", "1.20.3", "1.20.4", "1.20.5", "1.20.6", "1.21", "1.22", "1.23", "1.24",
        "1.25",
    ];
    SUPPORTED_PREFIXES.iter().any(|p| mc.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loader_type_serde_roundtrip() {
        let f = LoaderType::Fabric;
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "\"fabric\"");
        let parsed: LoaderType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, LoaderType::Fabric);

        let q = LoaderType::Quilt;
        let json = serde_json::to_string(&q).unwrap();
        assert_eq!(json, "\"quilt\"");
        let parsed: LoaderType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, LoaderType::Quilt);
    }

    #[test]
    fn test_loader_version_entry_roundtrip() {
        let e = LoaderVersionEntry {
            version: "0.16.9".into(),
            stable: true,
            build: Some(509),
        };
        let json = serde_json::to_string(&e).unwrap();
        let parsed: LoaderVersionEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, e);
    }

    #[test]
    fn test_loader_version_entry_omits_none_build() {
        let e = LoaderVersionEntry {
            version: "0.16.9".into(),
            stable: true,
            build: None,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(
            !json.contains("build"),
            "None build should be omitted: {json}"
        );
    }

    #[test]
    fn test_loader_info_roundtrip() {
        let li = LoaderInfo {
            kind: ModloaderKind::Fabric,
            version: "0.16.9".into(),
            version_id: "fabric-loader-0.16.9-1.21.4".into(),
        };
        let json = serde_json::to_string(&li).unwrap();
        let parsed: LoaderInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, li);
        assert!(
            json.contains("\"kind\":\"fabric\""),
            "kind should serialize snake_case: {json}"
        );
    }

    #[test]
    fn test_loader_library_roundtrip_fabric_with_hashes() {
        let lib = LoaderLibrary {
            name: "org.ow2.asm:asm:9.7.1".into(),
            url: Some("https://maven.fabricmc.net/".into()),
            sha1: Some("f0ed132a49244b042cd0e15702ab9f2ce3cc8436".into()),
            sha256: Some("aa".into()),
            sha512: None,
            md5: None,
            size: Some(65000),
        };
        let json = serde_json::to_string(&lib).unwrap();
        let parsed: LoaderLibrary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, lib);
    }

    #[test]
    fn test_loader_library_roundtrip_quilt_no_hashes() {
        // Quilt profiles populate name + url only; all hash fields parse as None.
        let json = r#"{"name":"org.quiltmc:quilt-loader:0.30.0-beta.7","url":"https://maven.quiltmc.org/"}"#;
        let parsed: LoaderLibrary = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.name, "org.quiltmc:quilt-loader:0.30.0-beta.7");
        assert_eq!(parsed.url.as_deref(), Some("https://maven.quiltmc.org/"));
        assert!(parsed.sha1.is_none(), "Quilt has no sha1");
        assert!(parsed.sha256.is_none(), "Quilt has no sha256");
        assert!(parsed.sha512.is_none(), "Quilt has no sha512");
        assert!(parsed.md5.is_none(), "Quilt has no md5");
        assert!(parsed.size.is_none(), "Quilt has no size");
    }

    #[test]
    fn test_loader_library_omits_none_fields() {
        let lib = LoaderLibrary {
            name: "g:a:1".into(),
            url: None,
            sha1: None,
            sha256: None,
            sha512: None,
            md5: None,
            size: None,
        };
        let json = serde_json::to_string(&lib).unwrap();
        assert!(!json.contains("url"), "None url omitted: {json}");
        assert!(!json.contains("sha1"), "None sha1 omitted: {json}");
    }

    #[test]
    fn test_loader_type_serde_roundtrip_forge() {
        let f = LoaderType::Forge;
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "\"forge\"");
        let parsed: LoaderType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, LoaderType::Forge);
    }

    #[test]
    fn test_loader_type_serde_roundtrip_neoforge_uses_neoforge_not_neo_underscore_forge() {
        let nf = LoaderType::NeoForge;
        let json = serde_json::to_string(&nf).unwrap();
        assert_eq!(
            json, "\"neoforge\"",
            "must serialize as 'neoforge', NOT 'neo_forge'"
        );
        let parsed: LoaderType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, LoaderType::NeoForge);
    }

    #[test]
    fn test_loader_info_neoforge_kind_serializes_as_neoforge() {
        let li = LoaderInfo {
            kind: ModloaderKind::NeoForge,
            version: "21.1.228".into(),
            version_id: "neoforge-21.1.228".into(),
        };
        let json = serde_json::to_string(&li).unwrap();
        assert!(
            json.contains("\"kind\":\"neoforge\""),
            "ModloaderKind::NeoForge must serialize as 'neoforge', got: {json}"
        );
        let parsed: LoaderInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, li);
    }

    #[test]
    fn test_loader_info_forge_kind_serializes_as_forge() {
        let li = LoaderInfo {
            kind: ModloaderKind::Forge,
            version: "1.20.1-47.4.20".into(),
            version_id: "1.20.1-forge-47.4.20".into(),
        };
        let json = serde_json::to_string(&li).unwrap();
        assert!(
            json.contains("\"kind\":\"forge\""),
            "ModloaderKind::Forge must serialize as 'forge', got: {json}"
        );
        let parsed: LoaderInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, li);
    }

    // ── MC version compatibility helpers (07-05) ──────────────────────────────

    #[test]
    fn test_forge_supported_rejects_pre_113() {
        assert!(!forge_supported_for_mc("1.12.2"));
        assert!(!forge_supported_for_mc("1.7.10"));
        assert!(!forge_supported_for_mc("1.0"));
    }

    #[test]
    fn test_forge_supported_accepts_113_plus() {
        for mc in ["1.13", "1.16.5", "1.20.1", "1.21.4", "1.21.8"] {
            assert!(forge_supported_for_mc(mc), "should accept {mc}");
        }
    }

    #[test]
    fn test_neoforge_supported_rejects_pre_1201() {
        for mc in ["1.20", "1.19.4", "1.16.5", "1.12.2"] {
            assert!(!neoforge_supported_for_mc(mc), "should reject {mc}");
        }
    }

    #[test]
    fn test_neoforge_supported_accepts_1201_plus() {
        for mc in ["1.20.1", "1.20.4", "1.21", "1.21.4", "1.21.8"] {
            assert!(neoforge_supported_for_mc(mc), "should accept {mc}");
        }
    }
}
