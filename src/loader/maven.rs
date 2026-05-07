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

/// Returns true iff every byte of `s` is in `[A-Za-z0-9._+-]`, `s` is non-empty,
/// and `s` is not the path-traversal sentinel `..`.
///
/// `+` is allowed because Fabric ships transitive Maven coords with build-metadata
/// versions like `net.fabricmc:sponge-mixin:0.15.4+mixin.0.8.7`. `+` has no special
/// meaning on Linux or Windows path components, so traversal protection (no `..`,
/// no `/`, no `\`) is preserved.
fn is_safe_maven_segment(s: &str) -> bool {
    !s.is_empty()
        && s != ".."
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-' || b == b'+'
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

    #[test]
    fn test_maven_coord_to_path_accepts_plus_in_version() {
        // Fabric ships transitive deps with build-metadata versions:
        //   net.fabricmc:sponge-mixin:0.15.4+mixin.0.8.7
        // The `+` must round-trip into the on-disk path.
        let p = maven_coord_to_path("net.fabricmc:sponge-mixin:0.15.4+mixin.0.8.7").unwrap();
        assert_eq!(
            p,
            "net/fabricmc/sponge-mixin/0.15.4+mixin.0.8.7/sponge-mixin-0.15.4+mixin.0.8.7.jar"
        );

        let q = maven_coord_to_path("net.fabricmc:sponge-mixin:0.17.0+mixin.0.8.7").unwrap();
        assert_eq!(
            q,
            "net/fabricmc/sponge-mixin/0.17.0+mixin.0.8.7/sponge-mixin-0.17.0+mixin.0.8.7.jar"
        );
    }

    // -----------------------------------------------------------------
    // GAP-7-C fixtures (07.2-02): 4- and 5-segment Maven coordinates
    // -----------------------------------------------------------------
    // The Apache Maven coord layout per https://maven.apache.org/pom.html#Maven_Coordinates is:
    //   groupId:artifactId:version[:classifier[:extension]]
    // The original parser (07-01) hardcoded splitn(3, ':') and rejected
    // 4- and 5-segment coords. NeoForge's installer-produced version JSON
    // for MC 21.4.x references `net.neoforged:mergetool:2.0.0:api` (a real
    // upstream artifact published with classifier `api`); harvest panicked
    // at 85% with InvalidMavenCoord. These fixtures pin the new behaviour.

    #[test]
    fn test_parse_maven_coord_accepts_4_segment_classifier() {
        // GAP-7-C trigger: real-world coord from NeoForge 21.4.x installer output.
        let c = parse_maven_coord("net.neoforged:mergetool:2.0.0:api").unwrap();
        assert_eq!(c.group, "net.neoforged");
        assert_eq!(c.artifact, "mergetool");
        assert_eq!(c.version, "2.0.0");
        assert_eq!(c.classifier, Some("api"));
        assert_eq!(c.extension, None);
    }

    #[test]
    fn test_maven_coord_to_path_4_segment_classifier_neoforge_mergetool() {
        // The exact failing coord from the 2026-05-07 UAT — pins the GAP-7-C closure.
        let p = maven_coord_to_path("net.neoforged:mergetool:2.0.0:api").unwrap();
        assert_eq!(p, "net/neoforged/mergetool/2.0.0/mergetool-2.0.0-api.jar");
    }

    #[test]
    fn test_maven_coord_to_path_5_segment_classifier_extension() {
        // LWJGL natives shape — Phase 12 launch wiring will need this; closing
        // GAP-7-C delivers the infrastructure forward.
        let p = maven_coord_to_path("org.lwjgl:lwjgl-glfw:3.3.3:natives-linux:zip").unwrap();
        assert_eq!(
            p,
            "org/lwjgl/lwjgl-glfw/3.3.3/lwjgl-glfw-3.3.3-natives-linux.zip"
        );
    }

    #[test]
    fn test_parse_maven_coord_rejects_6_segments() {
        // 6 segments must be rejected — Apache Maven defines exactly 5.
        let r = parse_maven_coord("g:a:v:c:e:extra");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_parse_maven_coord_rejects_7_segments() {
        // Defense-in-depth: 7+ segments must be rejected.
        let r = parse_maven_coord("g:a:v:c:e:extra:more");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_maven_coord_to_path_rejects_classifier_with_traversal() {
        // is_safe_maven_segment must apply to classifier — `..` rejected.
        let r = maven_coord_to_path("net.neoforged:mergetool:2.0.0:..");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_maven_coord_to_path_rejects_classifier_with_forward_slash() {
        // is_safe_maven_segment must apply to classifier — `/` rejected.
        let r = maven_coord_to_path("net.neoforged:mergetool:2.0.0:foo/bar");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }

    #[test]
    fn test_maven_coord_to_path_rejects_extension_with_backslash() {
        // is_safe_maven_segment must apply to extension — `\` rejected (Windows path sep).
        let r = maven_coord_to_path("net.neoforged:mergetool:2.0.0:api:foo\\bar");
        assert!(matches!(r, Err(LoaderError::InvalidMavenCoord { .. })));
    }
}
