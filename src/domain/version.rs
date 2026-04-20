//! Minecraft version domain types.
//!
//! Phase 2 moves the canonical schema types into `crate::mojang::types`.
//! The legacy `McVersion` / `VersionType` placeholders are retained only
//! for tests that imported them from Phase 1.

pub use crate::mojang::{VersionEntry, VersionManifest};

/// Legacy placeholder retained for Phase 1 test compatibility. New code
/// should use `mojang::VersionEntry` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionType {
    Release,
    Snapshot,
    OldBeta,
    OldAlpha,
}

/// Legacy placeholder retained for Phase 1 test compatibility.
#[derive(Debug, Clone)]
pub struct McVersion {
    pub id: String,
    pub version_type: VersionType,
}
