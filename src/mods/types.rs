//! Modrinth domain types: wire shapes (Modrinth API v2), UI state shapes
//! (per 08-UI-SPEC.md §State Machine), and the per-instance ledger schema
//! (per 08-RESEARCH.md §Pattern 3).
//!
//! All types are pure data — no I/O, no async. Serde-derive on every type.

use serde::{Deserialize, Serialize};

// ============================================================================
// === Modrinth wire types (08-RESEARCH.md §Endpoint Reference)            ===
// ============================================================================

/// Modrinth dependency kind. Tag-format matches Modrinth's `dependency_type`
/// field: `"required"` | `"optional"` | `"incompatible"` | `"embedded"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepKind {
    Required,
    Optional,
    Incompatible,
    Embedded,
}

/// One dependency record on a Modrinth Version.
/// `project_id` is canonical; `version_id` pins a specific version (rare);
/// `file_name` is informational. Per 08-RESEARCH.md §Endpoint #4.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthDep {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    pub dependency_type: DepKind,
}

/// `hashes` block on a Modrinth file entry. sha512 is canonical; sha1 is legacy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModrinthHashes {
    pub sha1: String,
    pub sha512: String,
}

/// One file entry on a Modrinth Version. `primary == true` is the JAR to install.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthFile {
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: u64,
    pub hashes: ModrinthHashes,
}

/// Full Modrinth Version object as returned by /v2/version/{id} and
/// /v2/project/{id}/version (array element). All wire-shape fields per
/// 08-RESEARCH.md §Endpoint #4 lines 121-148.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthVersion {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub version_type: String, // "release" | "beta" | "alpha"
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    #[serde(default)]
    pub downloads: u64,
    pub date_published: String, // ISO 8601 — used for "latest" tiebreaker
    #[serde(default)]
    pub dependencies: Vec<ModrinthDep>,
    pub files: Vec<ModrinthFile>,
}

// ============================================================================
// === UI-SPEC types (08-UI-SPEC.md §State Machine lines 502-549)          ===
// ============================================================================

/// One row in the mod browser results list (left pane).
/// Field set is a strict subset of the wire-level search hit (we do not
/// need every Modrinth field; UI only renders these).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModrinthSearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String, // single-line summary
    pub downloads: u64,
    pub already_installed: bool, // computed against current instance ledger
}

/// Detail-pane view of a single project (right pane).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModrinthProjectDetail {
    pub project_id: String,
    pub title: String,
    pub author: String,
    pub body: String, // full description, may contain newlines
    pub downloads: u64,
    pub latest_version_label: String,
    pub latest_version_channel: String, // "release" | "beta" | "alpha"
    pub license_id: String,
    pub categories: Vec<String>,
}

/// Lightweight projection of `/v2/projects?ids=[...]` for title hydration.
///
/// We only need (id, title) pairs to populate `ResolvedDep.project_title`;
/// the full `ModrinthProjectDetail` (with body, license, etc.) is fetched
/// on demand from the detail-pane code path. Used by the dep-resolver to
/// close GAP-8-D — without this, the dep-confirm modal and Installed Mods
/// List surface opaque project_ids instead of human-readable titles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectIdTitle {
    pub id: String,
    pub title: String,
}

/// One row in the version-picker modal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModrinthVersionEntry {
    pub version_id: String,
    pub version_label: String, // "0.92.0+1.20.4"
    pub channel: String, // "release" | "beta" | "alpha"
    pub is_latest_stable: bool,
}

/// One resolved dep row in the dep-confirm modal.
/// Algorithm output of `mods::dep_resolve::resolve` (08-04).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedDep {
    pub kind: DepKind,
    pub project_id: String,
    pub project_title: String,
    /// None for Optional / Incompatible / Embedded variants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<ModrinthVersion>,
    pub already_satisfied: bool,
    pub is_new_download: bool, // false if already_satisfied / embedded / incompatible
}

/// Mod-browser fetch state machine. Pure UI state — no serde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModBrowserFetchState {
    Loading,
    Ready,
    Error(String),
}

// ============================================================================
// === Per-instance ledger (08-RESEARCH.md §Pattern 3 lines 504-553)       ===
// ============================================================================

/// Source of an installed mod — supports forward-compat with Phase 9 (CurseForge),
/// Phase 10 (modpack), Phase 11 (local file drop), and any future "manual drop" detection.
///
/// Canonical wire format per 08-RESEARCH.md line 517 + 08-UI-SPEC.md line 668:
/// `"modrinth"` | `"curseforge"` | `"manual"` | `"modpack"` | `"local"` (single-word, no
/// underscore in `curseforge` — `serde(rename_all = "snake_case")` would emit
/// `curse_forge`, so the `CurseForge` variant carries an explicit `#[serde(rename)]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModSource {
    Modrinth,
    #[serde(rename = "curseforge")]
    CurseForge,
    Manual,
    Modpack,
    /// Drop-from-path install (Phase 11). Wire form: `"local"`.
    Local,
}

/// Discriminates mods from resource/shader packs in the shared ledger.
/// Default == Mod so pre-Phase-11 rows (no `kind` field) deserialize
/// transparently — mirrors HashAlgo::Sha512 default at types.rs:184.
/// Per 11-RESEARCH.md §"Adding `kind: PackKind`" + Phase 9 precedent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InstalledItemKind {
    #[default]
    Mod,
    ResourcePack,
    Shader,
}

/// Hash algorithm discriminator for `InstalledModRow.hash_algo`.
///
/// Phase 8 (Modrinth) uses Sha512 canonically; Phase 9 (CurseForge) uses
/// Sha1 by default (with Sha256 as a rare fallback). Default == Sha512 so
/// pre-Phase-9 ledger files (no `hash_algo` field) deserialize transparently
/// — see `tests::test_pre_phase9_ledger_loads_with_default_hash_algo`.
///
/// Per 09-RESEARCH.md §"Per-Instance Ledger Reuse" lines 297-318.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HashAlgo {
    /// Modrinth canonical (Phase 8). Default — pre-Phase-9 ledger rows
    /// without a `hash_algo` field deserialize as `Sha512`.
    #[default]
    Sha512,
    /// CurseForge default. The hex value is stored in the existing
    /// `InstalledModRow.sha512` field — the field name is a Phase 8
    /// historical-naming carve-out, the discriminator is `hash_algo`.
    Sha1,
    /// Rare — for future use (CurseForge files occasionally carry SHA-256).
    Sha256,
}

/// One row in `instance_dir/installed-mods.toml`. Carries full provenance
/// (project_id, version_id, sha512) so we can detect "already installed"
/// without re-hashing every file in `.minecraft/mods/`.
/// Full shape per 08-RESEARCH.md §Pattern 3 lines 535-549.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledModRow {
    pub mod_id: String,        // Modrinth project_id (or "manual:{sha512_short}")
    pub project_slug: String,
    pub display_name: String,
    pub version_id: String,    // Modrinth version_id
    pub version_label: String, // Modrinth version_number verbatim
    pub file_name: String,     // e.g. "sodium-fabric-0.5.8+mc1.20.4.jar"
    pub sha512: String,        // 128 hex chars, lowercase
    pub size: u64,
    /// Hash algorithm discriminator. NEW in Phase 9: `Sha512` (Modrinth)
    /// or `Sha1` (CurseForge). The `sha512` field name is kept stable for
    /// back-compat — for CurseForge rows, the SHA-1 hex value is stored
    /// in `sha512` with `hash_algo == HashAlgo::Sha1` as the discriminator.
    /// Documented historical-naming carve-out.
    /// `#[serde(default)]` lets pre-Phase-9 ledger files deserialize with
    /// `hash_algo == HashAlgo::Sha512` (the Modrinth canonical).
    /// Per 09-RESEARCH.md §"Per-Instance Ledger Reuse" lines 297-318.
    #[serde(default)]
    pub hash_algo: HashAlgo,
    /// Item kind discriminator. NEW in Phase 11. `#[serde(default)]` lets
    /// pre-Phase-11 ledger rows (no `kind` field) deserialize as `Mod`
    /// (mirrors `hash_algo` Phase 9 migration precedent above).
    #[serde(default)]
    pub kind: InstalledItemKind,
    pub source: ModSource,
    pub enabled: bool,         // false → file is "{file_name}.disabled"
    pub installed_at: String,  // RFC3339
}

/// Schema for `instance_dir/installed-mods.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub schema_version: u32,
    #[serde(default)]
    pub mods: Vec<InstalledModRow>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self { schema_version: 1, mods: Vec::new() }
    }
}

// ============================================================================
// === Tests                                                               ===
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dep_kind_serde_snake_case() {
        assert_eq!(serde_json::to_string(&DepKind::Required).unwrap(), "\"required\"");
        assert_eq!(serde_json::to_string(&DepKind::Optional).unwrap(), "\"optional\"");
        assert_eq!(serde_json::to_string(&DepKind::Incompatible).unwrap(), "\"incompatible\"");
        assert_eq!(serde_json::to_string(&DepKind::Embedded).unwrap(), "\"embedded\"");
        // Roundtrip
        let parsed: DepKind = serde_json::from_str("\"required\"").unwrap();
        assert_eq!(parsed, DepKind::Required);
    }

    #[test]
    fn test_mod_source_serde_snake_case() {
        assert_eq!(serde_json::to_string(&ModSource::Modrinth).unwrap(), "\"modrinth\"");
        assert_eq!(serde_json::to_string(&ModSource::CurseForge).unwrap(), "\"curseforge\"");
        assert_eq!(serde_json::to_string(&ModSource::Manual).unwrap(), "\"manual\"");
        assert_eq!(serde_json::to_string(&ModSource::Modpack).unwrap(), "\"modpack\"");
        let parsed: ModSource = serde_json::from_str("\"modrinth\"").unwrap();
        assert_eq!(parsed, ModSource::Modrinth);
    }

    #[test]
    fn test_modrinth_version_parse_minimal_shape() {
        // Minimum-fields parse — exercises serde defaults on dependencies.
        let json = r#"{
            "id":"abc","project_id":"def","name":"Sodium 0.5.8","version_number":"0.5.8",
            "version_type":"release","game_versions":["1.20.4"],"loaders":["fabric"],
            "date_published":"2026-01-01T00:00:00Z",
            "files":[{"url":"https://cdn.modrinth.com/x.jar","filename":"x.jar","primary":true,"size":1024,
                      "hashes":{"sha1":"abc","sha512":"def"}}]
        }"#;
        let v: ModrinthVersion = serde_json::from_str(json).unwrap();
        assert_eq!(v.id, "abc");
        assert_eq!(v.dependencies.len(), 0);
        assert_eq!(v.files.len(), 1);
        assert!(v.files[0].primary);
    }

    #[test]
    fn test_modrinth_dep_parse_with_only_project_id() {
        let json = r#"{"project_id":"AANobbMI","dependency_type":"required"}"#;
        let d: ModrinthDep = serde_json::from_str(json).unwrap();
        assert_eq!(d.project_id.as_deref(), Some("AANobbMI"));
        assert!(d.version_id.is_none());
        assert!(d.file_name.is_none());
        assert_eq!(d.dependency_type, DepKind::Required);
    }

    #[test]
    fn test_modrinth_dep_parse_with_only_version_id() {
        // Q2 from 08-RESEARCH.md — spec-rare case where only version_id is set.
        let json = r#"{"version_id":"Yp8wLY1P","dependency_type":"optional"}"#;
        let d: ModrinthDep = serde_json::from_str(json).unwrap();
        assert!(d.project_id.is_none());
        assert_eq!(d.version_id.as_deref(), Some("Yp8wLY1P"));
        assert_eq!(d.dependency_type, DepKind::Optional);
    }

    #[test]
    fn test_installed_mod_row_toml_roundtrip() {
        let row = InstalledModRow {
            mod_id: "AANobbMI".into(),
            project_slug: "sodium".into(),
            display_name: "Sodium".into(),
            version_id: "Yp8wLY1P".into(),
            version_label: "0.5.8".into(),
            file_name: "sodium-fabric-0.5.8+mc1.20.4.jar".into(),
            sha512: "a3f0c91a".into(),
            size: 1567890,
            hash_algo: HashAlgo::Sha512,
            kind: InstalledItemKind::Mod,
            source: ModSource::Modrinth,
            enabled: true,
            installed_at: "2026-05-05T12:34:56Z".into(),
        };
        let s = toml::to_string_pretty(&row).unwrap();
        let parsed: InstalledModRow = toml::from_str(&s).unwrap();
        assert_eq!(parsed, row);
        // ModSource serializes as snake_case in TOML too.
        assert!(s.contains("source = \"modrinth\""), "snake_case ModSource: {s}");
    }

    #[test]
    fn test_ledger_toml_roundtrip_with_two_mods() {
        let l = Ledger {
            schema_version: 1,
            mods: vec![
                InstalledModRow {
                    mod_id: "AANobbMI".into(),
                    project_slug: "sodium".into(),
                    display_name: "Sodium".into(),
                    version_id: "Yp8wLY1P".into(),
                    version_label: "0.5.8".into(),
                    file_name: "sodium.jar".into(),
                    sha512: "deadbeef".into(),
                    size: 1024,
                    hash_algo: HashAlgo::Sha512,
                    kind: InstalledItemKind::Mod,
                    source: ModSource::Modrinth,
                    enabled: true,
                    installed_at: "2026-01-01T00:00:00Z".into(),
                },
                InstalledModRow {
                    mod_id: "P7dR8mSH".into(),
                    project_slug: "fabric-api".into(),
                    display_name: "Fabric API".into(),
                    version_id: "ZZZZ".into(),
                    version_label: "0.92.0".into(),
                    file_name: "fabric-api.jar".into(),
                    sha512: "cafebabe".into(),
                    size: 2048,
                    hash_algo: HashAlgo::Sha512,
                    kind: InstalledItemKind::Mod,
                    source: ModSource::Modrinth,
                    enabled: false,
                    installed_at: "2026-01-02T00:00:00Z".into(),
                },
            ],
        };
        let s = toml::to_string_pretty(&l).unwrap();
        let parsed: Ledger = toml::from_str(&s).unwrap();
        assert_eq!(parsed, l);
        assert_eq!(parsed.mods.len(), 2);
        assert!(!parsed.mods[1].enabled);
    }

    #[test]
    fn test_pre_phase9_ledger_loads_with_default_hash_algo() {
        // A Phase 8 ledger file (no `hash_algo` field) must deserialize with
        // `hash_algo == HashAlgo::Sha512` thanks to `#[serde(default)]`.
        // Per 09-RESEARCH.md §"Per-Instance Ledger Reuse" lines 297-318.
        let toml_text = r#"
mod_id = "AANobbMI"
project_slug = "sodium"
display_name = "Sodium"
version_id = "Yp8wLY1P"
version_label = "0.5.8"
file_name = "sodium.jar"
sha512 = "deadbeef"
size = 1024
source = "modrinth"
enabled = true
installed_at = "2026-01-01T00:00:00Z"
"#;
        let row: InstalledModRow = toml::from_str(toml_text).unwrap();
        assert_eq!(
            row.hash_algo,
            HashAlgo::Sha512,
            "pre-Phase-9 row must default to Sha512"
        );
        assert_eq!(row.source, ModSource::Modrinth);
        assert_eq!(row.mod_id, "AANobbMI");
        assert_eq!(row.sha512, "deadbeef");
    }

    #[test]
    fn test_hash_algo_serde_snake_case() {
        assert_eq!(serde_json::to_string(&HashAlgo::Sha512).unwrap(), "\"sha512\"");
        assert_eq!(serde_json::to_string(&HashAlgo::Sha1).unwrap(), "\"sha1\"");
        assert_eq!(serde_json::to_string(&HashAlgo::Sha256).unwrap(), "\"sha256\"");
        let parsed: HashAlgo = serde_json::from_str("\"sha1\"").unwrap();
        assert_eq!(parsed, HashAlgo::Sha1);
        assert_eq!(HashAlgo::default(), HashAlgo::Sha512);
    }

    #[test]
    fn test_ledger_default_is_empty_v1() {
        let l = Ledger::default();
        assert_eq!(l.schema_version, 1);
        assert!(l.mods.is_empty());
    }

    #[test]
    fn test_resolved_dep_serde_with_version_none() {
        let rd = ResolvedDep {
            kind: DepKind::Optional,
            project_id: "ABC".into(),
            project_title: "Mod Menu".into(),
            version: None,
            already_satisfied: false,
            is_new_download: false,
        };
        let json = serde_json::to_string(&rd).unwrap();
        assert!(!json.contains("version"), "None version should be omitted: {json}");
        let parsed: ResolvedDep = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rd);
    }

    #[test]
    fn test_modrinth_search_hit_roundtrip() {
        let h = ModrinthSearchHit {
            project_id: "AANobbMI".into(),
            slug: "sodium".into(),
            title: "Sodium".into(),
            description: "Modern rendering engine".into(),
            downloads: 12345,
            already_installed: true,
        };
        let json = serde_json::to_string(&h).unwrap();
        let parsed: ModrinthSearchHit = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, h);
    }

    #[test]
    fn test_modrinth_version_entry_roundtrip() {
        let e = ModrinthVersionEntry {
            version_id: "Yp8wLY1P".into(),
            version_label: "0.5.8".into(),
            channel: "release".into(),
            is_latest_stable: true,
        };
        let json = serde_json::to_string(&e).unwrap();
        let parsed: ModrinthVersionEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, e);
    }

    #[test]
    fn test_mod_browser_fetch_state_equality() {
        assert_eq!(ModBrowserFetchState::Loading, ModBrowserFetchState::Loading);
        assert_ne!(ModBrowserFetchState::Loading, ModBrowserFetchState::Ready);
        assert_eq!(
            ModBrowserFetchState::Error("x".into()),
            ModBrowserFetchState::Error("x".into())
        );
    }

    // --- Phase 11 migration tests (InstalledItemKind + ModSource::Local) ------

    #[test]
    fn test_pre_phase11_ledger_loads_with_default_kind_mod() {
        // A pre-Phase-11 ledger row (no `kind` field) must deserialize as
        // `InstalledItemKind::Mod` thanks to `#[serde(default)]`.
        // Mirrors test_pre_phase9_ledger_loads_with_default_hash_algo above.
        let toml_text = r#"
mod_id = "AANobbMI"
project_slug = "sodium"
display_name = "Sodium"
version_id = "Yp8wLY1P"
version_label = "0.5.8"
file_name = "sodium.jar"
sha512 = "deadbeef"
size = 1024
source = "modrinth"
enabled = true
installed_at = "2026-01-01T00:00:00Z"
"#;
        let row: InstalledModRow = toml::from_str(toml_text).unwrap();
        assert_eq!(
            row.kind,
            InstalledItemKind::Mod,
            "pre-Phase-11 row must default to InstalledItemKind::Mod"
        );
    }

    #[test]
    fn test_installed_item_kind_serde_snake_case() {
        // Verify exact wire form so cross-version ledger compat is pinned.
        assert_eq!(
            serde_json::to_string(&InstalledItemKind::Mod).unwrap(),
            "\"mod\""
        );
        assert_eq!(
            serde_json::to_string(&InstalledItemKind::ResourcePack).unwrap(),
            "\"resource_pack\""
        );
        assert_eq!(
            serde_json::to_string(&InstalledItemKind::Shader).unwrap(),
            "\"shader\""
        );
        // Default is Mod.
        assert_eq!(InstalledItemKind::default(), InstalledItemKind::Mod);
    }

    #[test]
    fn test_mod_source_local_serializes_as_local() {
        // ModSource::Local must round-trip via toml as `source = "local"`.
        let row = InstalledModRow {
            mod_id: "pack:abc123".into(),
            project_slug: "faithful-32x".into(),
            display_name: "Faithful 32x".into(),
            version_id: "local".into(),
            version_label: "local".into(),
            file_name: "Faithful 32x.zip".into(),
            sha512: "deadbeef".into(),
            size: 512 * 1024 * 1024,
            hash_algo: HashAlgo::Sha1,
            kind: InstalledItemKind::ResourcePack,
            source: ModSource::Local,
            enabled: true,
            installed_at: "2026-05-08T00:00:00Z".into(),
        };
        let s = toml::to_string_pretty(&row).unwrap();
        assert!(s.contains("source = \"local\""), "snake_case Local: {s}");
        let parsed: InstalledModRow = toml::from_str(&s).unwrap();
        assert_eq!(parsed.source, ModSource::Local);
        assert_eq!(parsed.kind, InstalledItemKind::ResourcePack);
    }
}
