//! Minecraft version domain types. Placeholder; Phase 2 extends.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionType {
    Release,
    Snapshot,
    OldBeta,
    OldAlpha,
}

#[derive(Debug, Clone)]
pub struct McVersion {
    pub id: String,               // e.g., "1.21.4"
    pub version_type: VersionType,
}
