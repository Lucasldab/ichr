//! Pure CurseForge filter mapping — no I/O.
//!
//! Maps mineltui's `LoaderInfo` to CurseForge's `ModLoaderType` integer enum.
//!
//! **Pitfall 4 (09-RESEARCH.md line 954) — DO NOT confuse with Modrinth's
//! string enum.** CurseForge's enum is:
//!   1 = Forge
//!   2 = Cauldron (legacy — not in mineltui's ModloaderKind)
//!   3 = LiteLoader (legacy — not in mineltui's ModloaderKind)
//!   4 = Fabric
//!   5 = Quilt
//!   6 = NeoForge
//!
//! Off-by-one mapping is a high-frequency bug: a planner who reads "Fabric"
//! and assumes it is value 3 (because Modrinth ordering uses
//! `["fabric","quilt","forge","neoforge"]` with no Cauldron/LiteLoader gap)
//! will get LiteLoader-only results and an empty browser for modern MC.
//!
//! **Pitfall 6 (08-RESEARCH.md Quilt → fabric expansion) DOES NOT APPLY for
//! CurseForge.** The API has no "include both" mechanism. Quilt instances
//! query modLoaderType=5 only; Quilt users wanting Fabric mods use the
//! Modrinth browser (M keybind). Documented in 09-HUMAN-UAT step 12.

use crate::domain::instance::ModloaderKind;
use crate::loader::types::LoaderInfo;

/// Map mineltui loader to CurseForge's ModLoaderType integer enum.
/// Returns `None` for `Vanilla` — caller omits the `modLoaderType` query param.
pub fn curseforge_loader_type(loader: Option<&LoaderInfo>) -> Option<i32> {
    match loader.map(|l| l.kind) {
        None | Some(ModloaderKind::Vanilla) => None,
        Some(ModloaderKind::Forge) => Some(1),
        Some(ModloaderKind::Fabric) => Some(4),
        Some(ModloaderKind::Quilt) => Some(5),
        Some(ModloaderKind::NeoForge) => Some(6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loader(kind: ModloaderKind) -> LoaderInfo {
        LoaderInfo {
            kind,
            version: "test".into(),
            version_id: "test-version".into(),
        }
    }

    #[test]
    fn test_none_loader_returns_none() {
        assert_eq!(curseforge_loader_type(None), None);
    }

    #[test]
    fn test_vanilla_returns_none() {
        let l = loader(ModloaderKind::Vanilla);
        assert_eq!(curseforge_loader_type(Some(&l)), None);
    }

    #[test]
    fn test_forge_returns_one() {
        let l = loader(ModloaderKind::Forge);
        assert_eq!(curseforge_loader_type(Some(&l)), Some(1));
    }

    #[test]
    fn test_fabric_returns_four() {
        let l = loader(ModloaderKind::Fabric);
        assert_eq!(curseforge_loader_type(Some(&l)), Some(4));
    }

    #[test]
    fn test_quilt_returns_five_not_expanded_to_fabric() {
        // Per 09-RESEARCH.md Pattern 3 lines 808-816 — CurseForge has no
        // "include both" mechanism, so Quilt instances query modLoaderType=5
        // ONLY (unlike Modrinth where Quilt expands to ["fabric", "quilt"]).
        let l = loader(ModloaderKind::Quilt);
        assert_eq!(curseforge_loader_type(Some(&l)), Some(5));
    }

    #[test]
    fn test_neoforge_returns_six() {
        let l = loader(ModloaderKind::NeoForge);
        assert_eq!(curseforge_loader_type(Some(&l)), Some(6));
    }

    #[test]
    fn test_all_loaders_map_to_distinct_values() {
        // Regression guard against accidentally mapping two loaders to the
        // same value (Pitfall 4 cascade).
        let f = curseforge_loader_type(Some(&loader(ModloaderKind::Forge))).unwrap();
        let fa = curseforge_loader_type(Some(&loader(ModloaderKind::Fabric))).unwrap();
        let q = curseforge_loader_type(Some(&loader(ModloaderKind::Quilt))).unwrap();
        let n = curseforge_loader_type(Some(&loader(ModloaderKind::NeoForge))).unwrap();
        // All four values are distinct.
        assert!(f != fa && f != q && f != n);
        assert!(fa != q && fa != n);
        assert!(q != n);
        // Sanity: matches the documented enum values from docs.curseforge.com.
        assert_eq!((f, fa, q, n), (1, 4, 5, 6));
    }
}
