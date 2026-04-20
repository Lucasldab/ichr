//! Instance domain types. Placeholder for Phase 1; Phase 2 fills in fields.

/// Opaque instance identifier. Phase 1 uses `String`; Phase 2 migrates to a
/// UUID newtype without API break for callers that only compare/clone/display.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstanceId(pub String);

impl std::fmt::Display for InstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModloaderKind {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

/// In-memory instance record. Phase 2 adds: modloader details, java_override,
/// created_at, last_played_at, group/tag.
#[derive(Debug, Clone)]
pub struct Instance {
    pub id: InstanceId,
    pub name: String,
    pub mc_version: String,
    pub modloader: ModloaderKind,
}
