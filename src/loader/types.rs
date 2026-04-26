//! Loader domain types — pure data, no I/O.

use serde::{Deserialize, Serialize};

use crate::domain::instance::ModloaderKind;

/// Modloader family for which the loader install pipeline runs.
/// Distinct from `ModloaderKind` (which also has Vanilla/Forge/NeoForge);
/// LoaderType only enumerates the two loaders Phase 6 implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderType {
    Fabric,
    Quilt,
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
/// Canonical shared shape used by BOTH Fabric and Quilt clients —
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
        assert!(!json.contains("build"), "None build should be omitted: {json}");
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
        assert!(json.contains("\"kind\":\"fabric\""), "kind should serialize snake_case: {json}");
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
            url: None, sha1: None, sha256: None, sha512: None, md5: None, size: None,
        };
        let json = serde_json::to_string(&lib).unwrap();
        assert!(!json.contains("url"), "None url omitted: {json}");
        assert!(!json.contains("sha1"), "None sha1 omitted: {json}");
    }
}
