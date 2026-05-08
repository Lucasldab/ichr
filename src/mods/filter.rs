//! Pure mapping helpers for the Modrinth integration.
//!
//! No I/O, no async — every function is testable with `cargo nextest run -E 'test(mods::filter)'`
//! in milliseconds.
//!
//! Functions:
//! - `modrinth_filter_for` — instance loader/MC → Modrinth `loaders` + `game_versions` query params (Pitfall 2: Quilt expands to `['fabric','quilt']`).
//! - `search_facets`        — build the URL-encodable facets JSON for `/v2/search`.
//! - `pick_primary_file`    — primary file selection per 08-RESEARCH.md §Endpoint #4 line 148.
//! - `pick_latest_by_date`  — latest-by-`date_published` selection (ISO 8601 lex-sort).
//! - `is_safe_modrinth_slug` / `is_safe_mod_filename` — V5 input validation gates BEFORE disk-path construction.
//! - `disabled_filename` / `enabled_filename` — `.jar.disabled` toggle convention helpers.

use crate::domain::instance::ModloaderKind;
use crate::loader::types::LoaderInfo;
use crate::mods::types::{ModrinthFile, ModrinthVersion};

// ============================================================================
// === Loader / MC version → Modrinth query parameters                     ===
// ============================================================================

/// Returns `(loaders_to_query, game_versions_to_query)` for the Modrinth API.
///
/// Quilt expands to `["fabric","quilt"]` because Quilt is binary-compatible with
/// Fabric — Fabric-only mods load on Quilt instances. This is Pitfall 2 from
/// 08-RESEARCH.md and is a load-bearing invariant.
///
/// `game_versions` is just `[mc_version_id]` for v1; future enhancement noted
/// in 08-RESEARCH.md §Pattern 4.
pub fn modrinth_filter_for(
    loader: Option<&LoaderInfo>,
    mc_version_id: &str,
) -> (Vec<&'static str>, Vec<String>) {
    let loaders = match loader.map(|l| l.kind) {
        None | Some(ModloaderKind::Vanilla) => vec!["minecraft"],
        Some(ModloaderKind::Fabric) => vec!["fabric"],
        Some(ModloaderKind::Quilt) => vec!["fabric", "quilt"],
        Some(ModloaderKind::Forge) => vec!["forge"],
        Some(ModloaderKind::NeoForge) => vec!["neoforge"],
    };
    (loaders, vec![mc_version_id.to_string()])
}

// ============================================================================
// === Search facets builder                                               ===
// ============================================================================

/// Build the URL-encodable `facets` parameter for `GET /v2/search`.
///
/// Outer brackets group with AND; inner with OR. Caller is responsible for
/// URL-encoding the returned string before appending to a URL.
///
/// Example output for `(["fabric"], ["1.20.4"], "mod")`:
/// `[["categories:fabric"],["versions:1.20.4"],["project_type:mod"]]`
pub fn search_facets(loaders: &[&str], mc_versions: &[String], project_type: &str) -> String {
    // GAP-FACETS-EMPTY-08 (Phase 8.2 gap closure): empty inner OR-arrays
    // (e.g. `[]`) inside the outer AND make Modrinth match nothing — the
    // outer AND requires every inner OR to satisfy at least one term. So
    // when a category is "any" (empty input slice), we MUST omit that
    // entire AND group, not emit `[]`. Caller contract (client.rs:97)
    // still emits `&facets=` whenever `mc.is_some() || !loaders.is_empty()`;
    // the project_type group is always present so the parameter is never
    // empty in practice.
    let make = |k: &str, vs: &[String]| -> String {
        let parts: Vec<String> = vs.iter().map(|v| format!("\"{k}:{v}\"")).collect();
        format!("[{}]", parts.join(","))
    };
    let loader_arr: Vec<String> = loaders.iter().map(|s| (*s).to_string()).collect();
    let pt_arr = [project_type.to_string()];

    let mut parts: Vec<String> = Vec::with_capacity(3);
    if !loader_arr.is_empty() {
        parts.push(make("categories", &loader_arr));
    }
    if !mc_versions.is_empty() {
        parts.push(make("versions", mc_versions));
    }
    // project_type is always non-empty — it's a required positional arg.
    parts.push(make("project_type", &pt_arr));

    format!("[{}]", parts.join(","))
}

// ============================================================================
// === File / version selection                                            ===
// ============================================================================

/// Return the file marked `primary == true`. If none, fall back to the first
/// file (08-RESEARCH.md §Endpoint #4 line 148).
pub fn pick_primary_file(files: &[ModrinthFile]) -> Option<&ModrinthFile> {
    files.iter().find(|f| f.primary).or_else(|| files.first())
}

/// Return the version with the lexicographically-greatest `date_published`.
/// Modrinth uses ISO 8601 with `Z` suffix on every timestamp, so lex sort
/// equals chronological sort.
pub fn pick_latest_by_date(versions: &[ModrinthVersion]) -> Option<&ModrinthVersion> {
    versions
        .iter()
        .max_by(|a, b| a.date_published.cmp(&b.date_published))
}

// ============================================================================
// === Input validation (V5 — path-traversal mitigation)                   ===
// ============================================================================

/// True iff `s` is non-empty and every byte is `[A-Za-z0-9_-]`.
/// Mirrors the `is_safe_maven_segment` allowlist from `src/loader/maven.rs`.
pub fn is_safe_modrinth_slug(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// True iff `s` is a safe `.jar` filename — no path-traversal characters.
///
/// Required BEFORE joining `s` to `instance_minecraft_dir(slug).join("mods")`
/// (08-RESEARCH.md §Security Domain row "Path traversal via crafted filename").
///
/// Rules:
/// - Must end in `.jar` (case-sensitive — Modrinth always lower-cases).
/// - Must NOT start with `.` (no hidden files / no `..`).
/// - Must NOT contain `/`, `\`, or any `..` substring.
/// - Every byte must be `[A-Za-z0-9._+-]`.
pub fn is_safe_mod_filename(s: &str) -> bool {
    if !s.ends_with(".jar") {
        return false;
    }
    if s.starts_with('.') {
        return false;
    }
    if s.contains('/') || s.contains('\\') || s.contains("..") {
        return false;
    }
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'+' || b == b'-')
}

/// 500 MB cap for resource/shader pack files. Larger than mods because
/// high-resolution texture packs (Faithful 32x, Patrix 128x) can be
/// 200-400 MB legitimately. Per 11-CONTEXT.md D-LOCK pack-size cap.
pub const MAX_PACK_FILE_BYTES: u64 = 500 * 1024 * 1024;

/// True iff `s` is a safe `.zip` pack filename — no path-traversal.
/// Mirrors `is_safe_mod_filename` rules with three deltas:
///   1. Extension is `.zip` (case-insensitive — Windows NTFS).
///   2. SPACE byte is allowed (community packs: `Faithful 32x.zip`).
///   3. Otherwise identical: no leading dot, no slash/backslash/`..`,
///      ASCII alphanumeric + `._+- ` allowlist.
pub fn is_safe_pack_filename(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if !lower.ends_with(".zip") {
        return false;
    }
    if s.starts_with('.') {
        return false;
    }
    if s.contains('/') || s.contains('\\') || s.contains("..") {
        return false;
    }
    s.bytes().all(|b| {
        b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'+' || b == b'-' || b == b' '
    })
}

// ============================================================================
// === .jar.disabled toggle helpers (08-RESEARCH.md §Pattern 7)            ===
// ============================================================================

/// `"sodium-fabric-0.5.8.jar"` → `"sodium-fabric-0.5.8.jar.disabled"`.
pub fn disabled_filename(file_name: &str) -> String {
    format!("{file_name}.disabled")
}

/// `"sodium-fabric-0.5.8.jar.disabled"` → `Some("sodium-fabric-0.5.8.jar")`.
/// Returns `None` if the filename does not end in `.disabled`.
pub fn enabled_filename(disabled_name: &str) -> Option<&str> {
    disabled_name.strip_suffix(".disabled")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::types::{ModrinthHashes, ModrinthVersion};

    // --- modrinth_filter_for ---------------------------------------------------

    #[test]
    fn test_filter_vanilla_none_loader() {
        let (loaders, gv) = modrinth_filter_for(None, "1.20.4");
        assert_eq!(loaders, vec!["minecraft"]);
        assert_eq!(gv, vec!["1.20.4".to_string()]);
    }

    #[test]
    fn test_filter_vanilla_explicit() {
        let info = LoaderInfo {
            kind: ModloaderKind::Vanilla,
            version: "".into(),
            version_id: "".into(),
        };
        let (loaders, _) = modrinth_filter_for(Some(&info), "1.20.4");
        assert_eq!(loaders, vec!["minecraft"]);
    }

    #[test]
    fn test_filter_fabric() {
        let info = LoaderInfo {
            kind: ModloaderKind::Fabric,
            version: "0.16.9".into(),
            version_id: "fabric-loader-0.16.9-1.20.4".into(),
        };
        let (loaders, _) = modrinth_filter_for(Some(&info), "1.20.4");
        assert_eq!(loaders, vec!["fabric"]);
    }

    #[test]
    fn test_filter_quilt_expands_to_fabric_plus_quilt() {
        // PITFALL 2 — load-bearing: Quilt is binary-compatible with Fabric.
        // Modrinth's facet system does AND/OR not "compat-aware OR".
        let info = LoaderInfo {
            kind: ModloaderKind::Quilt,
            version: "0.30.0".into(),
            version_id: "quilt-loader-0.30.0-1.20.4".into(),
        };
        let (loaders, _) = modrinth_filter_for(Some(&info), "1.20.4");
        assert_eq!(
            loaders,
            vec!["fabric", "quilt"],
            "Quilt MUST expand to ['fabric','quilt']"
        );
    }

    #[test]
    fn test_filter_forge() {
        let info = LoaderInfo {
            kind: ModloaderKind::Forge,
            version: "0.0.0".into(),
            version_id: "x".into(),
        };
        let (loaders, _) = modrinth_filter_for(Some(&info), "1.20.4");
        assert_eq!(loaders, vec!["forge"]);
    }

    #[test]
    fn test_filter_neoforge() {
        let info = LoaderInfo {
            kind: ModloaderKind::NeoForge,
            version: "0.0.0".into(),
            version_id: "x".into(),
        };
        let (loaders, _) = modrinth_filter_for(Some(&info), "1.20.4");
        assert_eq!(loaders, vec!["neoforge"]);
    }

    // --- search_facets --------------------------------------------------------

    #[test]
    fn test_search_facets_fabric_1_20_4_mod() {
        let f = search_facets(&["fabric"], &["1.20.4".to_string()], "mod");
        assert_eq!(
            f,
            "[[\"categories:fabric\"],[\"versions:1.20.4\"],[\"project_type:mod\"]]"
        );
    }

    #[test]
    fn test_search_facets_quilt_pair() {
        let f = search_facets(&["fabric", "quilt"], &["1.20.4".to_string()], "mod");
        assert_eq!(
            f,
            "[[\"categories:fabric\",\"categories:quilt\"],[\"versions:1.20.4\"],[\"project_type:mod\"]]"
        );
    }

    #[test]
    fn test_search_facets_resource_pack_project_type() {
        // Forward-compat: Phase 11 will pass project_type="resourcepack".
        let f = search_facets(&["minecraft"], &["1.20.4".to_string()], "resourcepack");
        assert!(f.contains("project_type:resourcepack"), "got: {f}");
    }

    /// GAP-FACETS-EMPTY-08 (Phase 8.2): empty loaders + non-empty mc must NOT
    /// emit an empty AND group `[]`. Modrinth treats `[]` inside an outer AND
    /// as "match nothing" → zero hits even when other groups would match.
    /// Round-2 UAT test 3a regression: loader='any' + mc='1.20.4' returned
    /// zero results because of this bug.
    #[test]
    fn test_search_facets_empty_loaders_with_mc_no_empty_and_group() {
        let f = search_facets(&[], &["1.20.4".to_string()], "mod");
        assert_eq!(
            f, "[[\"versions:1.20.4\"],[\"project_type:mod\"]]",
            "empty loaders must drop the categories AND group entirely; got: {f}"
        );
        assert!(
            !f.contains("[]"),
            "facets string MUST NOT contain an empty AND group `[]`; got: {f}"
        );
    }

    /// Inverse: non-empty loaders + empty mc must also drop the empty
    /// versions group (today this is unreachable from client.rs because
    /// search() short-circuits, but the function is callable from
    /// list_versions and Phase 11 entry points; lock the contract).
    #[test]
    fn test_search_facets_with_loader_no_mc_no_empty_and_group() {
        let f = search_facets(&["fabric"], &[], "mod");
        assert_eq!(
            f, "[[\"categories:fabric\"],[\"project_type:mod\"]]",
            "empty mc_versions must drop the versions AND group entirely; got: {f}"
        );
        assert!(!f.contains("[]"), "got: {f}");
    }

    /// Both empty: only the always-present project_type group remains.
    #[test]
    fn test_search_facets_both_empty_only_project_type() {
        let f = search_facets(&[], &[], "mod");
        assert_eq!(
            f, "[[\"project_type:mod\"]]",
            "both empty: only project_type AND group remains; got: {f}"
        );
        assert!(!f.contains("[]"), "got: {f}");
    }

    // --- pick_primary_file ----------------------------------------------------

    fn mk_file(name: &str, primary: bool) -> ModrinthFile {
        ModrinthFile {
            url: format!("https://cdn.modrinth.com/{name}"),
            filename: name.to_string(),
            primary,
            size: 1024,
            hashes: ModrinthHashes {
                sha1: "abc".into(),
                sha512: "def".into(),
            },
        }
    }

    #[test]
    fn test_pick_primary_returns_primary_when_present() {
        let files = vec![
            mk_file("a.jar", false),
            mk_file("b.jar", true),
            mk_file("c.jar", false),
        ];
        assert_eq!(pick_primary_file(&files).unwrap().filename, "b.jar");
    }

    #[test]
    fn test_pick_primary_falls_back_to_first_when_none_primary() {
        let files = vec![mk_file("a.jar", false), mk_file("b.jar", false)];
        assert_eq!(pick_primary_file(&files).unwrap().filename, "a.jar");
    }

    #[test]
    fn test_pick_primary_empty_returns_none() {
        assert!(pick_primary_file(&[]).is_none());
    }

    // --- pick_latest_by_date --------------------------------------------------

    fn mk_version(id: &str, date: &str) -> ModrinthVersion {
        ModrinthVersion {
            id: id.into(),
            project_id: "p".into(),
            name: "n".into(),
            version_number: "0.0.1".into(),
            version_type: "release".into(),
            game_versions: vec!["1.20.4".into()],
            loaders: vec!["fabric".into()],
            downloads: 0,
            date_published: date.into(),
            dependencies: vec![],
            files: vec![mk_file("x.jar", true)],
        }
    }

    #[test]
    fn test_pick_latest_by_date_picks_max() {
        let v = vec![
            mk_version("a", "2024-01-01T00:00:00Z"),
            mk_version("b", "2026-01-01T00:00:00Z"),
            mk_version("c", "2025-01-01T00:00:00Z"),
        ];
        assert_eq!(pick_latest_by_date(&v).unwrap().id, "b");
    }

    #[test]
    fn test_pick_latest_by_date_empty_returns_none() {
        assert!(pick_latest_by_date(&[]).is_none());
    }

    // --- is_safe_modrinth_slug -----------------------------------------------

    #[test]
    fn test_safe_slug_accepts_normal() {
        assert!(is_safe_modrinth_slug("sodium"));
        assert!(is_safe_modrinth_slug("fabric-api"));
        assert!(is_safe_modrinth_slug("AABBCCDD"));
        assert!(is_safe_modrinth_slug("with_underscore"));
    }

    #[test]
    fn test_safe_slug_rejects_traversal_and_separators() {
        assert!(!is_safe_modrinth_slug(""));
        assert!(!is_safe_modrinth_slug(".."));
        assert!(!is_safe_modrinth_slug("../etc/passwd"));
        assert!(!is_safe_modrinth_slug("foo/bar"));
        assert!(!is_safe_modrinth_slug("foo\\bar"));
        assert!(!is_safe_modrinth_slug("with.dot")); // dot NOT in slug allowlist
        assert!(!is_safe_modrinth_slug("with space"));
    }

    // --- is_safe_mod_filename ------------------------------------------------

    #[test]
    fn test_safe_filename_accepts_normal_jars() {
        assert!(is_safe_mod_filename("sodium-fabric-0.5.8+mc1.20.4.jar"));
        assert!(is_safe_mod_filename("fabric-api-0.92.0.jar"));
        assert!(is_safe_mod_filename("a.jar"));
    }

    #[test]
    fn test_safe_filename_rejects_traversal() {
        assert!(!is_safe_mod_filename("../etc/passwd.jar"));
        assert!(!is_safe_mod_filename("foo/bar.jar"));
        assert!(!is_safe_mod_filename("foo\\bar.jar"));
        assert!(!is_safe_mod_filename("foo..bar.jar"));
        assert!(!is_safe_mod_filename(".hidden.jar"));
    }

    #[test]
    fn test_safe_filename_rejects_non_jar() {
        assert!(!is_safe_mod_filename("sodium.zip"));
        assert!(!is_safe_mod_filename("readme.txt"));
        assert!(!is_safe_mod_filename(""));
        assert!(!is_safe_mod_filename("noext"));
    }

    #[test]
    fn test_safe_filename_rejects_jar_disabled() {
        // .jar.disabled is NOT a safe filename for download — the toggle layer adds .disabled
        // *after* placing the .jar; downloads always land as .jar.
        assert!(!is_safe_mod_filename("sodium.jar.disabled"));
    }

    #[test]
    fn test_safe_filename_rejects_unicode() {
        // Modrinth's CDN paths are ASCII; non-ASCII would indicate a tampered response.
        assert!(!is_safe_mod_filename("sodium-名前.jar"));
    }

    // --- disabled_filename / enabled_filename --------------------------------

    #[test]
    fn test_disabled_filename_appends_suffix() {
        assert_eq!(disabled_filename("sodium.jar"), "sodium.jar.disabled");
    }

    #[test]
    fn test_enabled_filename_strips_suffix() {
        assert_eq!(enabled_filename("sodium.jar.disabled"), Some("sodium.jar"));
    }

    #[test]
    fn test_enabled_filename_returns_none_when_not_disabled() {
        assert_eq!(enabled_filename("sodium.jar"), None);
        assert_eq!(enabled_filename("readme.txt"), None);
        assert_eq!(enabled_filename(""), None);
    }

    #[test]
    fn test_disabled_then_enabled_roundtrip() {
        let f = "sodium-fabric-0.5.8.jar";
        let d = disabled_filename(f);
        assert_eq!(enabled_filename(&d), Some(f));
    }

    // --- is_safe_pack_filename + MAX_PACK_FILE_BYTES -------------------------

    #[test]
    fn test_is_safe_pack_filename_accepts_basic_zip() {
        assert!(is_safe_pack_filename("Faithful.zip"));
    }

    #[test]
    fn test_is_safe_pack_filename_accepts_uppercase_extension() {
        // Case-insensitive .zip check — Windows NTFS users may see .ZIP.
        assert!(is_safe_pack_filename("Pack.ZIP"));
    }

    #[test]
    fn test_is_safe_pack_filename_accepts_space() {
        // D-LOCK: packs allow spaces (e.g. "Faithful 32x.zip").
        assert!(is_safe_pack_filename("Faithful 32x.zip"));
    }

    #[test]
    fn test_is_safe_pack_filename_rejects_jar() {
        assert!(!is_safe_pack_filename("mod.jar"));
    }

    #[test]
    fn test_is_safe_pack_filename_rejects_dot_dot() {
        assert!(!is_safe_pack_filename("../etc/passwd.zip"));
    }

    #[test]
    fn test_is_safe_pack_filename_rejects_path_separator_unix() {
        assert!(!is_safe_pack_filename("subdir/pack.zip"));
    }

    #[test]
    fn test_is_safe_pack_filename_rejects_path_separator_windows() {
        assert!(!is_safe_pack_filename("subdir\\pack.zip"));
    }

    #[test]
    fn test_is_safe_pack_filename_rejects_dotfile() {
        assert!(!is_safe_pack_filename(".hidden.zip"));
    }

    #[test]
    fn test_is_safe_pack_filename_rejects_no_extension() {
        assert!(!is_safe_pack_filename("pack"));
    }

    #[test]
    fn test_max_pack_file_bytes_is_500mb() {
        assert_eq!(MAX_PACK_FILE_BYTES, 500 * 1024 * 1024);
    }
}
