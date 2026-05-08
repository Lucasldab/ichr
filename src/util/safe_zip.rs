//! Shared zip path-traversal guard. Used by `install::natives_extract` and
//! `modpack::overrides`. ATLauncher (GHSA-7cff-8xv4-mvx6, CVSS 7.8) and
//! Prism Launcher (GHSA-wxgx-8v36-mj2m, CVSS 7.8) shipped CVE-rated
//! path-traversal vulnerabilities by re-implementing this check per-site;
//! this module is the canonical single-site enforcement point.

use std::path::{Component, Path, PathBuf};

/// Guard a zip entry name against path traversal.
///
/// Returns `Some(base.join(<safe-relative-path>))` when `entry_name` consists
/// entirely of [`Component::Normal`] components (i.e., plain filename
/// segments with no `..`, `.`, absolute prefix, or Windows drive letter).
///
/// Returns `None` if any component would allow the resulting path to escape
/// `base`. Callers MUST skip the entry (not error) when `None` is returned —
/// this matches the "skip-not-error" semantics established in
/// `install::natives_extract`.
///
/// # Important notes for callers
///
/// - **Empty entry names** produce `Some(base.to_path_buf())` — zero Normal
///   components are pushed. Callers must not write to bare `base`.
/// - **Leading `./`** (CurDir component) → `None`. Strip `./` before calling
///   if the archive may use this convention.
/// - **Backslash paths on Linux** are treated as a single Normal component
///   and will return `Some`. This is correct: backslash is a valid filename
///   character on Linux and has no traversal semantics.
/// - **This function never calls `canonicalize()`** — canonicalization
///   requires the target path to already exist and performs filesystem I/O,
///   defeating the pre-creation safety check.
pub fn safe_extract_path(entry_name: &str, base: &Path) -> Option<PathBuf> {
    let mut result = base.to_path_buf();
    for component in Path::new(entry_name).components() {
        match component {
            Component::Normal(c) => result.push(c),
            // Rejects RootDir, CurDir, ParentDir, Prefix — all traversal vectors.
            _ => return None,
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn unit_accepts_nested_path() {
        let base = Path::new("/tmp/base");
        let result = safe_extract_path("mods/sodium.jar", base);
        assert_eq!(result, Some(base.join("mods/sodium.jar")));
    }

    #[test]
    fn unit_accepts_single_component() {
        let base = Path::new("/tmp/base");
        let result = safe_extract_path("options.txt", base);
        assert_eq!(result, Some(base.join("options.txt")));
    }

    #[test]
    fn unit_rejects_parent_traversal() {
        let base = Path::new("/tmp/base");
        assert_eq!(safe_extract_path("../secret", base), None);
    }

    #[test]
    fn unit_rejects_absolute_path() {
        let base = Path::new("/tmp/base");
        assert_eq!(safe_extract_path("/etc/passwd", base), None);
    }

    #[test]
    fn unit_rejects_curdir() {
        let base = Path::new("/tmp/base");
        assert_eq!(safe_extract_path("./mods/foo.jar", base), None);
    }
}
