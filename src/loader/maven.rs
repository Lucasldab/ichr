//! Maven coordinate utilities — pure string transformations, no I/O.
//!
//! Used by the loader install pipeline to derive download URLs and
//! local library paths from Maven coordinates in loader profile JSON.
//!
//! Security: every coordinate segment is validated against
//! `[A-Za-z0-9._-]+` BEFORE constructing any disk path, blocking
//! path-traversal via crafted Maven coordinates (V5 Input Validation,
//! mirrors `safe_extract_path` in Phase 2).

use crate::loader::error::LoaderError;

/// Returns true iff every byte of `s` is in `[A-Za-z0-9._-]`, `s` is non-empty,
/// and `s` is not the path-traversal sentinel `..`.
fn is_safe_maven_segment(s: &str) -> bool {
    !s.is_empty()
        && s != ".."
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-'
        })
}

/// Parse a Maven coordinate `"group:artifact:version"` into its three parts.
///
/// Each of the three parts must satisfy `is_safe_maven_segment` — any
/// `..`, `/`, `\\`, or other unexpected character causes
/// `LoaderError::InvalidMavenCoord` and prevents disk-path construction.
pub fn parse_maven_coord(coord: &str) -> Result<(&str, &str, &str), LoaderError> {
    let parts: Vec<&str> = coord.splitn(3, ':').collect();
    if parts.len() != 3 {
        return Err(LoaderError::InvalidMavenCoord { coord: coord.to_string() });
    }
    let (group, artifact, version) = (parts[0], parts[1], parts[2]);
    if !is_safe_maven_segment(group)
        || !is_safe_maven_segment(artifact)
        || !is_safe_maven_segment(version)
    {
        return Err(LoaderError::InvalidMavenCoord { coord: coord.to_string() });
    }
    Ok((group, artifact, version))
}

/// Convert a Maven coordinate to its standard repo-relative path:
/// `"org.ow2.asm:asm:9.7.1"` → `"org/ow2/asm/asm/9.7.1/asm-9.7.1.jar"`.
///
/// Returns `LoaderError::InvalidMavenCoord` if the coordinate is malformed
/// or contains any unsafe character (path-traversal guard).
pub fn maven_coord_to_path(coord: &str) -> Result<String, LoaderError> {
    let (group, artifact, version) = parse_maven_coord(coord)?;
    let group_path = group.replace('.', "/");
    Ok(format!("{group_path}/{artifact}/{version}/{artifact}-{version}.jar"))
}

/// Build the full download URL for a Maven coordinate against a repo base.
///
/// Trailing slashes on `repo_base` are stripped before joining.
pub fn maven_download_url(repo_base: &str, coord: &str) -> Result<String, LoaderError> {
    let path = maven_coord_to_path(coord)?;
    let base = repo_base.trim_end_matches('/');
    Ok(format!("{base}/{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maven_coord_to_path_fabric_loader() {
        let p = maven_coord_to_path("net.fabricmc:fabric-loader:0.16.9").unwrap();
        assert_eq!(p, "net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar");
    }

    #[test]
    fn test_maven_coord_to_path_quilt_loader() {
        let p = maven_coord_to_path("org.quiltmc:quilt-loader:0.30.0-beta.7").unwrap();
        assert_eq!(p, "org/quiltmc/quilt-loader/0.30.0-beta.7/quilt-loader-0.30.0-beta.7.jar");
    }

    #[test]
    fn test_maven_coord_to_path_asm() {
        let p = maven_coord_to_path("org.ow2.asm:asm:9.7.1").unwrap();
        assert_eq!(p, "org/ow2/asm/asm/9.7.1/asm-9.7.1.jar");
    }

    #[test]
    fn test_maven_coord_to_path_rejects_traversal_in_group() {
        let r = maven_coord_to_path("../etc/passwd:asm:9.7.1");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_maven_coord_to_path_rejects_traversal_in_artifact() {
        let r = maven_coord_to_path("org.evil:..:1.0");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_maven_coord_to_path_rejects_traversal_in_version() {
        let r = maven_coord_to_path("org.evil:lib:../1.0");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_maven_coord_to_path_rejects_forward_slash() {
        let r = maven_coord_to_path("org/evil:lib:1.0");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_maven_coord_to_path_rejects_backslash() {
        let r = maven_coord_to_path("org.evil:lib\\foo:1.0");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_maven_coord_to_path_rejects_two_colons() {
        let r = maven_coord_to_path("org.ow2.asm:asm");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_maven_coord_to_path_rejects_empty_segment() {
        let r = maven_coord_to_path("org.ow2.asm::9.7.1");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_maven_download_url_fabric() {
        let u = maven_download_url("https://maven.fabricmc.net/", "net.fabricmc:fabric-loader:0.16.9").unwrap();
        assert_eq!(
            u,
            "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar"
        );
    }

    #[test]
    fn test_maven_download_url_strips_trailing_slash() {
        // Both with and without trailing slash should produce identical URLs.
        let with    = maven_download_url("https://maven.example.com/", "g:a:1").unwrap();
        let without = maven_download_url("https://maven.example.com",  "g:a:1").unwrap();
        assert_eq!(with, without);
    }

    #[test]
    fn test_parse_maven_coord_extracts_three_parts() {
        let (g, a, v) = parse_maven_coord("net.fabricmc:fabric-loader:0.16.9").unwrap();
        assert_eq!(g, "net.fabricmc");
        assert_eq!(a, "fabric-loader");
        assert_eq!(v, "0.16.9");
    }
}
