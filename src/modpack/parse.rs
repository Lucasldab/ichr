//! `.mrpack` v1 manifest parsing: serde types, validation, and loader detection.
//!
//! All functions are synchronous (no I/O, no network). Validation errors surface
//! as typed `ModpackError` variants — no panics, no silent fallbacks.
//!
//! # Trust boundary
//! All public functions in this module accept untrusted data from the
//! `modrinth.index.json` inside a user-supplied `.mrpack` archive.
//! Input validation happens in `parse_index` (format version, game, missing
//! minecraft dep). Path-traversal protection for `MrpackFile::path` values
//! happens in `src/util/safe_zip.rs` (Plan 10-04).

use std::collections::HashMap;

use serde::Deserialize;

use crate::loader::types::LoaderType;
use crate::modpack::error::ModpackError;

// ============================================================================
// === Wire types                                                            ===
// ============================================================================

/// The top-level `modrinth.index.json` manifest inside a `.mrpack` archive.
///
/// `formatVersion` MUST equal `1`. `game` MUST equal `"minecraft"`.
/// `dependencies` MUST contain the key `"minecraft"`.
///
/// Unknown JSON fields are silently ignored (forward-compat — v1 minor
/// extensions may add fields; `#[serde(deny_unknown_fields)]` is forbidden
/// per the plan's `<forbids>` section).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackIndex {
    pub format_version: u32,
    pub game: String,
    pub version_id: String,
    pub name: String,
    pub summary: Option<String>,
    pub dependencies: HashMap<String, String>,
    pub files: Vec<MrpackFile>,
}

/// One entry in the `files[]` array — a mod (or other file) to download.
///
/// `env` is `Option<MrpackEnv>` with `#[serde(default)]` because many packs
/// in the wild omit the field entirely. Absent `env` means the file is
/// universal (treat as `env.client = Required`). See Pitfall 3 in RESEARCH.md.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackFile {
    pub path: String,
    pub hashes: MrpackHashes,
    #[serde(default)]
    pub env: Option<MrpackEnv>,
    pub downloads: Vec<String>,
    #[serde(default)]
    pub file_size: u64,
}

/// Hash pair recorded for every file entry.
///
/// SHA-512 is the PRIMARY verification hash. SHA-1 is recorded for the
/// per-instance ledger (compatibility) but is not used as the primary
/// integrity check.
#[derive(Debug, Clone, Deserialize)]
pub struct MrpackHashes {
    pub sha1: String,
    pub sha512: String,
}

/// Per-side environment requirement for a file entry.
///
/// Absent `env` field on the parent `MrpackFile` → `None` → treat as Required
/// on both sides (Pitfall 3).
#[derive(Debug, Clone, Deserialize)]
pub struct MrpackEnv {
    pub client: EnvRequirement,
    pub server: EnvRequirement,
}

/// Whether a file is required, optional, or unsupported for a given side.
///
/// `#[default]` is `Required` — when the `env` object is present but a field
/// is somehow absent (malformed pack), defaulting to Required is safer than
/// silently skipping the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EnvRequirement {
    #[default]
    Required,
    Optional,
    Unsupported,
}

// ============================================================================
// === Public API                                                            ===
// ============================================================================

/// Parse and validate a `modrinth.index.json` JSON string.
///
/// # Errors
///
/// - [`ModpackError::ManifestParse`] — invalid JSON or schema mismatch
/// - [`ModpackError::UnsupportedFormat`] — `formatVersion` != 1
/// - [`ModpackError::UnsupportedGame`] — `game` != `"minecraft"`
/// - [`ModpackError::MissingMinecraftDependency`] — no `"minecraft"` key in `dependencies`
pub fn parse_index(json: &str) -> Result<MrpackIndex, ModpackError> {
    let idx: MrpackIndex = serde_json::from_str(json).map_err(ModpackError::ManifestParse)?;

    if idx.format_version != 1 {
        return Err(ModpackError::UnsupportedFormat {
            version: idx.format_version,
        });
    }

    if idx.game != "minecraft" {
        return Err(ModpackError::UnsupportedGame {
            game: idx.game.clone(),
        });
    }

    if !idx.dependencies.contains_key("minecraft") {
        return Err(ModpackError::MissingMinecraftDependency);
    }

    Ok(idx)
}

/// Map the manifest's `dependencies` map to a recognised `LoaderType`.
///
/// Priority order: Fabric > Quilt > Forge > NeoForge (Pitfall 8). When more
/// than one recognised loader key is present (malformed pack), the first-
/// priority match is returned and a `tracing::warn!` is emitted.
///
/// Returns `None` for vanilla packs (no recognised loader key). The caller
/// treats `None` as "no loader install step".
pub fn detect_loader(deps: &HashMap<String, String>) -> Option<(LoaderType, String)> {
    const LOADERS: &[(&str, LoaderType)] = &[
        ("fabric-loader", LoaderType::Fabric),
        ("quilt-loader", LoaderType::Quilt),
        ("forge", LoaderType::Forge),
        ("neoforge", LoaderType::NeoForge),
    ];

    let mut found: Vec<(&str, LoaderType, &str)> = Vec::new();
    for &(key, loader) in LOADERS {
        if let Some(version) = deps.get(key) {
            found.push((key, loader, version.as_str()));
        }
    }

    match found.len() {
        0 => None,
        1 => {
            let (_, loader, version) = found[0];
            Some((loader, version.to_owned()))
        }
        _ => {
            let keys: Vec<&str> = found.iter().map(|(k, _, _)| *k).collect();
            let (chosen_key, chosen_loader, chosen_version) = found[0];
            tracing::warn!(
                keys = ?keys,
                chosen = chosen_key,
                "multiple loader keys in dependencies — using first-priority match"
            );
            Some((chosen_loader, chosen_version.to_owned()))
        }
    }
}

/// Return `true` if this file should be downloaded for a client install.
///
/// - `None` → absent `env` field → universal file → always download (Pitfall 3)
/// - `Some(env)` → download unless `env.client == Unsupported`
pub fn should_download_for_client(env: Option<&MrpackEnv>) -> bool {
    match env {
        None => true,
        Some(e) => e.client != EnvRequirement::Unsupported,
    }
}

/// Strip a leading `"./"` from a path string.
///
/// Some packs in the wild prefix `files[].path` values with `"./"` (Open
/// Question §1 in RESEARCH.md). Stripping is safe: removing `"./"` from
/// `"./../../etc/passwd"` produces `"../../etc/passwd"` which the path-
/// traversal guard in `src/util/safe_zip.rs` still rejects.
pub fn strip_leading_dot_slash(path: &str) -> &str {
    path.strip_prefix("./").unwrap_or(path)
}

// ============================================================================
// === Tests                                                                 ===
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal valid JSON fixture shared across multiple tests.
    const MINIMAL_JSON: &str = r#"{
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "0.1.0",
        "name": "Minimal Test Pack",
        "summary": "Minimal fixture for unit tests",
        "dependencies": { "minecraft": "1.20.4", "fabric-loader": "0.16.9" },
        "files": [
            {
                "path": "mods/required-mod.jar",
                "hashes": { "sha1": "aabbccdd", "sha512": "eeff0011" },
                "env": { "client": "required", "server": "unsupported" },
                "downloads": ["https://cdn.modrinth.com/data/abc/required-mod.jar"],
                "fileSize": 10
            },
            {
                "path": "mods/optional-mod.jar",
                "hashes": { "sha1": "11223344", "sha512": "55667788" },
                "env": { "client": "optional", "server": "unsupported" },
                "downloads": ["https://cdn.modrinth.com/data/def/optional-mod.jar"],
                "fileSize": 10
            },
            {
                "path": "mods/server-only.jar",
                "hashes": { "sha1": "0", "sha512": "0" },
                "env": { "client": "unsupported", "server": "required" },
                "downloads": ["https://cdn.modrinth.com/data/ghi/server-only.jar"],
                "fileSize": 10
            },
            {
                "path": "mods/no-env-field.jar",
                "hashes": { "sha1": "aabb", "sha512": "ccdd" },
                "downloads": ["https://cdn.modrinth.com/data/jkl/no-env-field.jar"],
                "fileSize": 10
            }
        ]
    }"#;

    // 1. Happy path ─────────────────────────────────────────────────────────

    #[test]
    fn test_parse_minimal_index_happy_path() {
        let idx = parse_index(MINIMAL_JSON).expect("should parse");
        assert_eq!(idx.format_version, 1);
        assert_eq!(idx.game, "minecraft");
        assert_eq!(idx.version_id, "0.1.0");
        assert_eq!(idx.name, "Minimal Test Pack");
        assert_eq!(
            idx.summary.as_deref(),
            Some("Minimal fixture for unit tests")
        );
        assert_eq!(
            idx.dependencies.get("minecraft").map(String::as_str),
            Some("1.20.4")
        );
        assert_eq!(
            idx.dependencies.get("fabric-loader").map(String::as_str),
            Some("0.16.9")
        );
        assert_eq!(idx.files.len(), 4);

        // First file: required client, unsupported server
        let f0 = &idx.files[0];
        assert_eq!(f0.path, "mods/required-mod.jar");
        assert_eq!(f0.hashes.sha1, "aabbccdd");
        assert_eq!(f0.file_size, 10);
        assert!(f0.env.is_some());
        assert_eq!(f0.env.as_ref().unwrap().client, EnvRequirement::Required);

        // Fourth file: no env field at all
        let f3 = &idx.files[3];
        assert!(
            f3.env.is_none(),
            "absent env field should deserialize as None"
        );
    }

    // 2. Validation rejections ───────────────────────────────────────────────

    #[test]
    fn test_rejects_format_version_2() {
        let json = r#"{
            "formatVersion": 2, "game": "minecraft", "versionId": "1.0",
            "name": "Bad", "dependencies": { "minecraft": "1.20.4" }, "files": []
        }"#;
        match parse_index(json) {
            Err(ModpackError::UnsupportedFormat { version: 2 }) => {}
            other => panic!("expected UnsupportedFormat(2), got {other:?}"),
        }
    }

    #[test]
    fn test_rejects_non_minecraft_game() {
        let json = r#"{
            "formatVersion": 1, "game": "terraria", "versionId": "1.0",
            "name": "Bad", "dependencies": { "minecraft": "1.20.4" }, "files": []
        }"#;
        match parse_index(json) {
            Err(ModpackError::UnsupportedGame { game }) => {
                assert_eq!(game, "terraria");
            }
            other => panic!("expected UnsupportedGame, got {other:?}"),
        }
    }

    #[test]
    fn test_rejects_missing_minecraft_dependency() {
        let json = r#"{
            "formatVersion": 1, "game": "minecraft", "versionId": "1.0",
            "name": "Bad", "dependencies": { "fabric-loader": "0.16.9" }, "files": []
        }"#;
        match parse_index(json) {
            Err(ModpackError::MissingMinecraftDependency) => {}
            other => panic!("expected MissingMinecraftDependency, got {other:?}"),
        }
    }

    // 3. Env field edge cases (Pitfall 3) ───────────────────────────────────

    #[test]
    fn test_env_default_when_absent() {
        let idx = parse_index(MINIMAL_JSON).expect("should parse");
        // File at index 3 has no env field
        let f = &idx.files[3];
        assert!(f.env.is_none(), "env should be None when absent from JSON");
        assert!(
            should_download_for_client(f.env.as_ref()),
            "absent env → universal → should download"
        );
    }

    #[test]
    fn test_env_unsupported_filtered() {
        let idx = parse_index(MINIMAL_JSON).expect("should parse");
        // File at index 2 has env.client = unsupported
        let f = &idx.files[2];
        let env = f.env.as_ref().expect("env should be present");
        assert_eq!(env.client, EnvRequirement::Unsupported);
        assert!(
            !should_download_for_client(f.env.as_ref()),
            "env.client=unsupported → should NOT download"
        );
    }

    #[test]
    fn test_env_optional_downloaded() {
        let idx = parse_index(MINIMAL_JSON).expect("should parse");
        // File at index 1 has env.client = optional
        let f = &idx.files[1];
        let env = f.env.as_ref().expect("env should be present");
        assert_eq!(env.client, EnvRequirement::Optional);
        assert!(
            should_download_for_client(f.env.as_ref()),
            "env.client=optional → should download (v1 installs all optional)"
        );
    }

    // 4. detect_loader ────────────────────────────────────────────────────────

    fn deps(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_detect_loader_fabric() {
        let d = deps(&[("minecraft", "1.20.4"), ("fabric-loader", "0.16.9")]);
        let result = detect_loader(&d);
        assert_eq!(result, Some((LoaderType::Fabric, "0.16.9".to_owned())));
    }

    #[test]
    fn test_detect_loader_quilt() {
        let d = deps(&[("minecraft", "1.20.4"), ("quilt-loader", "0.26.0")]);
        let result = detect_loader(&d);
        assert_eq!(result, Some((LoaderType::Quilt, "0.26.0".to_owned())));
    }

    #[test]
    fn test_detect_loader_forge() {
        let d = deps(&[("minecraft", "1.20.1"), ("forge", "47.4.10")]);
        let result = detect_loader(&d);
        assert_eq!(result, Some((LoaderType::Forge, "47.4.10".to_owned())));
    }

    #[test]
    fn test_detect_loader_neoforge() {
        let d = deps(&[("minecraft", "1.21.4"), ("neoforge", "21.4.121")]);
        let result = detect_loader(&d);
        assert_eq!(result, Some((LoaderType::NeoForge, "21.4.121".to_owned())));
    }

    #[test]
    fn test_detect_loader_priority_fabric_wins() {
        // Pitfall 8: multiple loaders — Fabric must win over Forge
        let d = deps(&[
            ("minecraft", "1.20.4"),
            ("fabric-loader", "0.16.9"),
            ("forge", "47.4.10"),
        ]);
        let result = detect_loader(&d);
        assert_eq!(
            result,
            Some((LoaderType::Fabric, "0.16.9".to_owned())),
            "Fabric has higher priority than Forge"
        );
    }

    #[test]
    fn test_detect_loader_vanilla_returns_none() {
        let d = deps(&[("minecraft", "1.20.4")]);
        let result = detect_loader(&d);
        assert!(result.is_none(), "vanilla pack — no loader install step");
    }

    // 5. strip_leading_dot_slash ──────────────────────────────────────────────

    #[test]
    fn test_strip_leading_dot_slash() {
        assert_eq!(strip_leading_dot_slash("./mods/foo.jar"), "mods/foo.jar");
        assert_eq!(strip_leading_dot_slash("mods/foo.jar"), "mods/foo.jar");
        // Traversal attempt — strip only removes the leading "./"; the result
        // still contains ".." which the path-guard will reject separately.
        assert_eq!(
            strip_leading_dot_slash("./../../etc/passwd"),
            "../../etc/passwd"
        );
        // Double dot-slash should only strip ONE prefix.
        assert_eq!(strip_leading_dot_slash(".././foo.jar"), ".././foo.jar");
    }
}
