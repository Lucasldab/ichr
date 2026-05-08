//! CurseForge for Studios API v1 wire types + UI-state shapes.
//! See 09-RESEARCH.md §Endpoint Reference (lines 96-148).
//!
//! Per 09-PATTERNS.md §`src/mods/curseforge/types.rs`: every CurseForge
//! wire field uses camelCase (opposite of Modrinth's snake_case) -- every
//! Optional field carries `#[serde(default, skip_serializing_if = "Option::is_none")]`
//! per the `LoaderLibrary` precedent.
//!
//! All types are pure data -- no I/O, no async. Serde-derive on every type.

use serde::{Deserialize, Serialize};

// ============================================================================
// === Search hit (data[] of /v1/mods/search)                              ===
// ============================================================================

/// One row in the CurseForge mod browser results list.
///
/// Wire shape: element of the `data` array returned by
/// `GET /v1/mods/search?gameId=432&classId=6&...`. Field set is the strict
/// subset the UI needs -- categories are kept so the right-pane can render
/// labels without a follow-up `/v1/mods/{id}` round-trip.
///
/// `already_installed` is **not** a CurseForge wire field -- it is stamped
/// client-side after consulting the per-instance ledger. `#[serde(skip)]`
/// keeps it out of any serialised wire payload (caches, fixtures, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurseForgeSearchHit {
    pub id: u64,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(rename = "downloadCount", default)]
    pub download_count: u64,
    #[serde(default)]
    pub categories: Vec<CurseForgeCategory>,
    /// Stamped client-side from the per-instance ledger (NOT a wire field).
    #[serde(skip)]
    pub already_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurseForgeCategory {
    pub id: i32,
    pub name: String,
}

// ============================================================================
// === Project detail (data of /v1/mods/{modId})                           ===
// ============================================================================

/// Detail-pane view of a single CurseForge project.
///
/// Wire shape: the `data` envelope of `GET /v1/mods/{modId}`. Authors are
/// directly available (unlike Modrinth which requires a separate
/// `/v2/team/{id}/members` lookup).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurseForgeProjectDetail {
    pub id: u64,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "downloadCount", default)]
    pub download_count: u64,
    #[serde(default)]
    pub authors: Vec<CurseForgeAuthor>,
    #[serde(default)]
    pub links: CurseForgeLinks,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurseForgeAuthor {
    pub id: i32,
    pub name: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CurseForgeLinks {
    #[serde(rename = "websiteUrl", default)]
    pub website_url: String,
    #[serde(rename = "sourceUrl", default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(rename = "wikiUrl", default, skip_serializing_if = "Option::is_none")]
    pub wiki_url: Option<String>,
}

// ============================================================================
// === File entry (data[] of /v1/mods/{modId}/files)                       ===
// ============================================================================

/// One file entry on a CurseForge mod.
///
/// **CRITICAL:** `download_url` is `Option<String>` -- `null` iff the author
/// has disabled third-party distribution. This nullability is the
/// load-bearing fact for MOD-04 (the FileNotDownloadable UX path).
/// Per 09-RESEARCH.md §"downloadUrl null UX" lines 252-289.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurseForgeFileEntry {
    pub id: u64,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "fileName")]
    pub file_name: String,
    /// 1=Release, 2=Beta, 3=Alpha
    #[serde(rename = "releaseType", default)]
    pub release_type: i32,
    #[serde(rename = "fileStatus", default)]
    pub file_status: i32,
    #[serde(default)]
    pub hashes: Vec<CurseForgeHash>,
    #[serde(rename = "fileDate", default)]
    pub file_date: String,
    #[serde(rename = "fileLength", default)]
    pub file_length: u64,
    #[serde(rename = "downloadCount", default)]
    pub download_count: u64,
    /// **NULLABLE** -- `null` iff author has disabled third-party distribution.
    /// Per 09-RESEARCH.md §"downloadUrl null UX" lines 252-289.
    #[serde(rename = "downloadUrl", default)]
    pub download_url: Option<String>,
    #[serde(rename = "gameVersions", default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<CurseForgeFileDep>,
    #[serde(rename = "isAvailable", default)]
    pub is_available: bool,
}

/// CurseForge file hash entry.
///
/// `algo`: 1=SHA-1 (CurseForge default), 2=MD5, 3=SHA-256.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurseForgeHash {
    pub value: String,
    pub algo: i32,
}

/// CurseForge file dependency edge.
///
/// `relationType`: 1=embedded library, 2=optional dependency,
/// 3=required dependency, 4=tool, 5=incompatible, 6=include.
///
/// Phase 9 v1 does NOT auto-resolve transitive deps per
/// 09-RESEARCH.md §"Dependency resolution scope" line 460. The field is
/// kept on the wire shape so future plans can reuse it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurseForgeFileDep {
    #[serde(rename = "modId")]
    pub mod_id: u64,
    #[serde(rename = "relationType")]
    pub relation_type: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_hit_parses_data_envelope_element() {
        let json = r#"{
            "id": 443959,
            "slug": "wonderful-world-mod",
            "name": "Wonderful World Mod",
            "summary": "Adds wonders.",
            "downloadCount": 12345,
            "categories": [{"id": 1, "name": "Adventure"}]
        }"#;
        let h: CurseForgeSearchHit = serde_json::from_str(json).unwrap();
        assert_eq!(h.id, 443959);
        assert_eq!(h.slug, "wonderful-world-mod");
        assert_eq!(h.download_count, 12345);
        assert_eq!(h.categories.len(), 1);
        assert!(!h.already_installed, "default is false");
    }

    #[test]
    fn test_project_detail_parses() {
        let json = r#"{
            "id": 443959, "slug": "x", "name": "X",
            "downloadCount": 100, "authors": [{"id": 1, "name": "Auth", "url": "https://x"}],
            "links": {"websiteUrl": "https://x", "sourceUrl": null, "wikiUrl": null}
        }"#;
        let d: CurseForgeProjectDetail = serde_json::from_str(json).unwrap();
        assert_eq!(d.id, 443959);
        assert_eq!(d.authors.len(), 1);
        assert!(d.links.source_url.is_none());
        assert_eq!(d.links.website_url, "https://x");
    }

    #[test]
    fn test_file_entry_parses_with_null_download_url() {
        // The load-bearing fact for MOD-04: downloadUrl is null when the
        // author has disabled third-party distribution.
        let json = r#"{
            "id": 4567890,
            "displayName": "Wonderful World Mod 1.5.0",
            "fileName": "wonderful-world-mod-1.5.0.jar",
            "releaseType": 1,
            "fileStatus": 4,
            "hashes": [{"value": "ABCDEF", "algo": 1}],
            "fileDate": "2026-01-01T00:00:00Z",
            "fileLength": 1234567,
            "downloadCount": 999,
            "downloadUrl": null,
            "gameVersions": ["1.20.4", "Fabric"],
            "dependencies": [{"modId": 306612, "relationType": 3}],
            "isAvailable": true
        }"#;
        let f: CurseForgeFileEntry = serde_json::from_str(json).unwrap();
        assert_eq!(f.id, 4567890);
        assert!(
            f.download_url.is_none(),
            "downloadUrl null must deserialize as None"
        );
        assert_eq!(f.hashes.len(), 1);
        assert_eq!(f.hashes[0].algo, 1);
        assert_eq!(f.hashes[0].value, "ABCDEF");
        assert_eq!(f.dependencies.len(), 1);
        assert_eq!(f.dependencies[0].mod_id, 306612);
        assert_eq!(f.dependencies[0].relation_type, 3);
    }

    #[test]
    fn test_file_entry_parses_with_present_download_url() {
        let json = r#"{
            "id": 1, "displayName": "x", "fileName": "x.jar",
            "releaseType": 1, "fileStatus": 4, "hashes": [],
            "fileDate": "z", "fileLength": 0, "downloadCount": 0,
            "downloadUrl": "https://edge.forgecdn.net/files/1/2/x.jar",
            "gameVersions": [], "dependencies": [], "isAvailable": true
        }"#;
        let f: CurseForgeFileEntry = serde_json::from_str(json).unwrap();
        assert_eq!(
            f.download_url.as_deref(),
            Some("https://edge.forgecdn.net/files/1/2/x.jar")
        );
    }

    #[test]
    fn test_search_hit_roundtrip() {
        let h = CurseForgeSearchHit {
            id: 1,
            slug: "x".into(),
            name: "X".into(),
            summary: "s".into(),
            download_count: 100,
            categories: vec![],
            already_installed: false,
        };
        let json = serde_json::to_string(&h).unwrap();
        let parsed: CurseForgeSearchHit = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, h.id);
        assert_eq!(parsed.download_count, h.download_count);
    }
}
