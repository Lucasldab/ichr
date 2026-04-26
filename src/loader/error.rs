//! Typed errors for the modloader install pipeline.
//!
//! Library-layer errors. Convert to `AppError` at the `execute_effects`
//! boundary in `src/tui/run.rs` (or surface directly via
//! `Action::LoaderInstallFailed { error: e.to_string(), .. }`).

/// All failure modes for `LoaderService` operations.
#[derive(Debug, thiserror::Error)]
pub enum LoaderError {
    /// HTTP fetch of the meta API failed (network / non-2xx status / body read).
    #[error("Failed to fetch {loader} meta: {reason}")]
    MetaFetch { loader: &'static str, reason: String },

    /// JSON parse of a meta API response failed (unexpected shape).
    #[error("Failed to parse {loader} meta response: {reason}")]
    MetaParse { loader: &'static str, reason: String },

    /// SHA-1 verification of a downloaded loader library failed.
    #[error("SHA1 mismatch for {path}: expected {expected}, got {got}")]
    Sha1Mismatch { path: String, expected: String, got: String },

    /// A Maven coordinate was rejected by `maven_coord_to_path` validation.
    #[error("Invalid Maven coordinate: {coord}")]
    InvalidMavenCoord { coord: String },

    /// Writing the merged loader version JSON to disk failed.
    #[error("Failed to write loader version JSON to {path}: {reason}")]
    ProfileWrite { path: String, reason: String },

    /// User cancelled the install via the modal Esc key (CancellationToken fired).
    #[error("Loader install cancelled")]
    Cancelled,

    /// Underlying I/O error (filesystem, atomic_write, etc.).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_fetch_display_contains_loader_and_reason() {
        let e = LoaderError::MetaFetch { loader: "fabric", reason: "connect timeout".into() };
        let s = e.to_string();
        assert!(s.contains("fabric"), "missing loader name: {s}");
        assert!(s.contains("connect timeout"), "missing reason: {s}");
    }

    #[test]
    fn test_meta_parse_display() {
        let e = LoaderError::MetaParse { loader: "quilt", reason: "expected array at line 1".into() };
        let s = e.to_string();
        assert!(s.contains("quilt"), "loader missing: {s}");
        assert!(s.contains("expected array"), "reason missing: {s}");
    }

    #[test]
    fn test_sha1_mismatch_display() {
        let e = LoaderError::Sha1Mismatch {
            path: "libraries/org/ow2/asm/asm/9.7.1/asm-9.7.1.jar".into(),
            expected: "aabbccdd".into(),
            got: "ffeeddcc".into(),
        };
        let s = e.to_string();
        assert!(s.contains("aabbccdd"), "expected hash missing: {s}");
        assert!(s.contains("ffeeddcc"), "got hash missing: {s}");
        assert!(s.contains("asm-9.7.1.jar"), "path missing: {s}");
    }

    #[test]
    fn test_invalid_maven_coord_display() {
        let e = LoaderError::InvalidMavenCoord { coord: "org.evil:../traversal:1.0".into() };
        let s = e.to_string();
        assert!(s.contains("Invalid Maven coordinate"), "headline missing: {s}");
        assert!(s.contains("org.evil"), "coord missing: {s}");
    }

    #[test]
    fn test_profile_write_display() {
        let e = LoaderError::ProfileWrite {
            path: "/data/versions/fabric-loader-0.16.9-1.21.4/...json".into(),
            reason: "no space left on device".into(),
        };
        let s = e.to_string();
        assert!(s.contains("Failed to write"), "headline missing: {s}");
        assert!(s.contains("no space"), "reason missing: {s}");
    }

    #[test]
    fn test_cancelled_display() {
        let e = LoaderError::Cancelled;
        assert_eq!(e.to_string(), "Loader install cancelled");
    }

    #[test]
    fn test_io_from_conversion() {
        // The `#[from]` attribute provides automatic conversion from std::io::Error.
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let le: LoaderError = io_err.into();
        let s = le.to_string();
        assert!(s.contains("I/O error"), "I/O headline missing: {s}");
        assert!(s.contains("denied"), "io message missing: {s}");
    }
}
