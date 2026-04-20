//! Mojang protocol: version manifest, version JSON, asset index, library schema.
//!
//! Pure types + pure logic only (args/inherits/natives/rules/types).
//! HTTP client and disk cache live in client.rs and cache.rs respectively.
//! See `.planning/phases/02-mojang-protocol-and-instance-management/02-RESEARCH.md`
//! (§1–§7 for verified schemas, §Pitfall 1–4 for protocol quirks).

pub mod args;
pub mod cache;
pub mod client;
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
