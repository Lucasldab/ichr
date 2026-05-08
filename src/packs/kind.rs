//! `PackKind` -- discriminates resource packs from shader packs.
//!
//! Per 11-CONTEXT.md §"Module symmetry": ONE module parameterised by `PackKind`,
//! not two near-identical modules. All pack-kind-specific constants live here.

use serde::{Deserialize, Serialize};

use crate::mods::types::InstalledItemKind;

/// Discriminates a resource pack from a shader pack.
///
/// `Default` is intentionally NOT derived -- every call site must name a kind
/// explicitly. Deriving `Default` would create a silent `Resource` bias in
/// shader code paths. Per 11-01-PLAN.md must_haves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackKind {
    /// Resource pack -- lands in `.minecraft/resourcepacks/`.
    Resource,
    /// Shader pack -- lands in `.minecraft/shaderpacks/`.
    Shader,
}

impl PackKind {
    /// Returns the Minecraft subdirectory name for this kind.
    ///
    /// - `PackKind::Resource` → `"resourcepacks"`
    /// - `PackKind::Shader`   → `"shaderpacks"`
    ///
    /// Used by `AppPaths::instance_packs_dir` and `AppPaths::instance_pack_file`.
    pub fn subdir(&self) -> &'static str {
        match self {
            PackKind::Resource => "resourcepacks",
            PackKind::Shader => "shaderpacks",
        }
    }

    /// Returns the Modrinth `project_type` facet string for this kind.
    ///
    /// - `PackKind::Resource` → `"resourcepack"`
    /// - `PackKind::Shader`   → `"shader"`
    ///
    /// Used by the pack browser to filter Modrinth search results.
    /// Per 11-CONTEXT.md §"Modrinth browse (LOCKED -- reuse Phase 8)".
    pub fn modrinth_project_type(&self) -> &'static str {
        match self {
            PackKind::Resource => "resourcepack",
            PackKind::Shader => "shader",
        }
    }

    /// Maps `PackKind` → `InstalledItemKind` for ledger rows.
    ///
    /// Canonical definition lives here (not in service.rs or install.rs)
    /// so all callers share one definition. Previously `pack_kind_to_item_kind`
    /// in `service.rs` was a free function; install.rs uses this method instead
    /// to avoid duplication.
    pub fn into_installed_item_kind(self) -> InstalledItemKind {
        match self {
            PackKind::Resource => InstalledItemKind::ResourcePack,
            PackKind::Shader => InstalledItemKind::Shader,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subdir_resource_returns_resourcepacks() {
        assert_eq!(PackKind::Resource.subdir(), "resourcepacks");
    }

    #[test]
    fn test_subdir_shader_returns_shaderpacks() {
        assert_eq!(PackKind::Shader.subdir(), "shaderpacks");
    }

    #[test]
    fn test_modrinth_project_type_resource() {
        assert_eq!(PackKind::Resource.modrinth_project_type(), "resourcepack");
    }

    #[test]
    fn test_modrinth_project_type_shader() {
        assert_eq!(PackKind::Shader.modrinth_project_type(), "shader");
    }

    #[test]
    fn test_serde_roundtrip_resource() {
        // TOML can't serialize a bare enum at root level; wrap in a struct.
        // Verify the wire form is snake_case "resource".
        #[derive(serde::Serialize, serde::Deserialize)]
        struct W {
            kind: PackKind,
        }
        let w = W {
            kind: PackKind::Resource,
        };
        let s = toml::to_string(&w).unwrap();
        assert!(
            s.contains("kind = \"resource\""),
            "expected snake_case wire form, got: {s}"
        );
        let parsed: W = toml::from_str(&s).unwrap();
        assert_eq!(parsed.kind, PackKind::Resource);
    }

    #[test]
    fn test_into_installed_item_kind_resource() {
        use crate::mods::types::InstalledItemKind;
        assert_eq!(
            PackKind::Resource.into_installed_item_kind(),
            InstalledItemKind::ResourcePack
        );
    }

    #[test]
    fn test_into_installed_item_kind_shader() {
        use crate::mods::types::InstalledItemKind;
        assert_eq!(
            PackKind::Shader.into_installed_item_kind(),
            InstalledItemKind::Shader
        );
    }

    #[test]
    fn test_serde_roundtrip_shader() {
        // TOML can't serialize a bare enum at root level; wrap in a struct.
        // Verify the wire form is snake_case "shader".
        #[derive(serde::Serialize, serde::Deserialize)]
        struct W {
            kind: PackKind,
        }
        let w = W {
            kind: PackKind::Shader,
        };
        let s = toml::to_string(&w).unwrap();
        assert!(
            s.contains("kind = \"shader\""),
            "expected snake_case wire form, got: {s}"
        );
        let parsed: W = toml::from_str(&s).unwrap();
        assert_eq!(parsed.kind, PackKind::Shader);
    }
}
