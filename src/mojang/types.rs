//! Mojang version manifest and version JSON serde types.
//!
//! All structs use `#[serde(default)]` on optional fields for forward-compat
//! (unknown fields are silently ignored — no `deny_unknown_fields`).
//! See PITFALLS.md Pitfall 5 and 02-RESEARCH.md §1–§5.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Version Manifest v2
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<VersionEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub version_type: String,
    pub url: String,
    pub time: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
    pub sha1: String,
    #[serde(rename = "complianceLevel")]
    pub compliance_level: u8,
}

// ---------------------------------------------------------------------------
// Per-version JSON (client.json)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    pub id: String,
    #[serde(rename = "type")]
    pub version_type: String,
    pub main_class: String,
    /// Vanilla-only field; loader JSONs (Fabric/Quilt/Forge/NeoForge) lack this
    /// field on disk and inherit it via `inherits_from` from the vanilla parent.
    /// Demoted to `Option` so a real loader JSON deserialises cleanly. Use the
    /// `ResolvedVersion` struct (post-`resolve_inherits`) when the field MUST
    /// be present — the type system then enforces it.
    #[serde(default)]
    pub asset_index: Option<AssetIndex>,
    /// See `asset_index` doc-comment — same Option/inherit semantics.
    #[serde(default)]
    pub assets: Option<String>,
    /// See `asset_index` doc-comment — same Option/inherit semantics.
    #[serde(default)]
    pub downloads: Option<VersionDownloads>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub java_version: Option<JavaVersion>,
    #[serde(default)]
    pub logging: Option<LoggingConfig>,
    #[serde(default)]
    pub compliance_level: Option<u8>,
    #[serde(default)]
    pub minimum_launcher_version: Option<u32>,
    pub release_time: String,
    pub time: String,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(rename = "minecraftArguments", default)]
    pub minecraft_arguments: Option<String>,
    #[serde(rename = "inheritsFrom", default)]
    pub inherits_from: Option<String>,
}

// ---------------------------------------------------------------------------
// Arguments (1.13+ structured format)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<ArgumentEntry>,
    #[serde(default)]
    pub jvm: Vec<ArgumentEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArgumentEntry {
    Plain(String),
    Conditional(ConditionalArg),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConditionalArg {
    pub rules: Vec<Rule>,
    pub value: ArgValue,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArgValue {
    Single(String),
    Multiple(Vec<String>),
}

// ---------------------------------------------------------------------------
// Asset index reference (inline in VersionJson)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndex {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub total_size: u64,
    pub url: String,
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct VersionDownloads {
    #[serde(default)]
    pub client: Option<DownloadArtifact>,
    #[serde(default)]
    pub client_mappings: Option<DownloadArtifact>,
    #[serde(default)]
    pub server: Option<DownloadArtifact>,
    #[serde(default)]
    pub server_mappings: Option<DownloadArtifact>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadArtifact {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

// ---------------------------------------------------------------------------
// Libraries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: LibraryDownloads,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default)]
    pub extract: Option<ExtractConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<LibraryArtifact>,
    #[serde(default)]
    pub classifiers: Option<HashMap<String, LibraryArtifact>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LibraryArtifact {
    pub path: String,
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ExtractConfig {
    #[serde(default)]
    pub exclude: Vec<String>,
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

// ---------------------------------------------------------------------------
// Java version + logging
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    pub component: String,
    pub major_version: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    #[serde(default)]
    pub client: Option<LoggingClient>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingClient {
    pub argument: String,
    pub file: LoggingFile,
    #[serde(rename = "type")]
    pub logging_type: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingFile {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

// ---------------------------------------------------------------------------
// Asset index file (standalone JSON)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssetIndexFile {
    pub objects: HashMap<String, AssetObject>,
    #[serde(rename = "virtual", default)]
    pub virtual_: Option<bool>,
    #[serde(default)]
    pub map_to_resources: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

// ---------------------------------------------------------------------------
// ResolvedVersion — post-inheritsFrom-merge invariant: required fields populated.
// Produced by `resolve_inherits` and consumed by the launcher pipeline.
// The launcher pipeline (Steps 5-10 of `launch_instance`) reads ONLY this
// struct; it must not deserialize a raw on-disk JSON into ResolvedVersion
// (the type system's narrow shape is the whole point — it guarantees no
// caller accidentally reads a vanilla-only field off a loader's JSON).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ResolvedVersion {
    /// `version.id` (loader id when launching a modded instance, vanilla id
    /// otherwise). Used for the `${version_name}` substitution token in
    /// JVM/game args (MultiMC convention).
    pub id: String,
    /// `version.id` of the inheritsFrom chain's TERMINAL parent (the vanilla
    /// MC id where the JAR actually lives on disk). For a Fabric instance
    /// inheriting `1.20.4`, this is `"1.20.4"`. For a vanilla-only install
    /// (no `inheritsFrom`), this equals `id`. Used by the classpath builder
    /// to construct `paths.version_jar(&self.root_id)` — the loader's
    /// `paths.version_jar(&self.id)` would resolve to a non-existent file
    /// because Phase 6's loader install only writes `{loader-id}.json`
    /// (never `{loader-id}.jar`); the JAR is inherited from vanilla.
    pub root_id: String,
    pub version_type: String,
    pub main_class: String,
    /// Post-merge invariant: child.asset_index OR ancestor.asset_index.
    /// `resolve_inherits` errors with `InheritsFromMissingRequired { field: "asset_index" }`
    /// when neither side declares it (no further fallback is sound).
    pub asset_index: AssetIndex,
    /// Same invariant as `asset_index`.
    pub assets: String,
    /// Same invariant as `asset_index`.
    pub downloads: VersionDownloads,
    pub libraries: Vec<Library>,
    pub java_version: Option<JavaVersion>,
    pub logging: Option<LoggingConfig>,
    pub compliance_level: Option<u8>,
    pub minimum_launcher_version: Option<u32>,
    pub release_time: String,
    pub time: String,
    pub arguments: Option<Arguments>,
    pub minecraft_arguments: Option<String>,
}
