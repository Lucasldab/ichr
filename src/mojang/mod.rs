//! Mojang protocol: version manifest, version JSON, asset index, library schema.
//!
//! Pure types + pure logic only. No HTTP, no disk, no async. See
//! `.planning/phases/02-mojang-protocol-and-instance-management/02-RESEARCH.md`
//! (§1–§7 for verified schemas, §Pitfall 1–4 for protocol quirks).

pub mod args;
pub mod inherits;
pub mod natives;
pub mod rules;
pub mod types;

pub use types::{
    ArgValue, ArgumentEntry, Arguments, AssetIndex, AssetIndexFile, AssetObject, ConditionalArg,
    DownloadArtifact, ExtractConfig, JavaVersion, LatestVersions, Library, LibraryArtifact,
    LibraryDownloads, LoggingClient, LoggingConfig, LoggingFile, OsRule, Rule, VersionDownloads,
    VersionEntry, VersionJson, VersionManifest,
};
