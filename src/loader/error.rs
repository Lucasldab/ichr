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

    /// Subprocess (Forge/NeoForge installer) exited non-zero. `tail` is the
    /// last LOG_TAIL_LINES (200) lines of interleaved stdout/stderr captured
    /// by the ring buffer in `installer_subprocess::run_installer`.
    #[error("Installer exited with code {code}; last lines:\n{tail}")]
    SubprocessExit { code: i32, tail: String },

    /// Failed to populate the staging directory (mkdir, file copy, or skeleton
    /// `launcher_profiles.json` write). PITFALL 2 fix.
    #[error("Failed to populate staging directory: {reason}")]
    StagingPopulate { reason: String },

    /// Post-install harvest failed — staging didn't produce the expected
    /// `versions/<id>/` tree, version JSON malformed, or library walk failed.
    #[error("Failed to harvest install output: {reason}")]
    HarvestFailed { reason: String },

    /// Installer JAR download failed (network, non-2xx status, body read,
    /// or `.sha1` sidecar mismatch).
    #[error("Failed to fetch installer JAR: {reason}")]
    InstallerJarFetch { reason: String },

    /// Maven metadata XML parse error (extracted version list empty or
    /// upstream returned non-XML payload).
    #[error("Failed to parse Maven metadata: {reason}")]
    MavenMetadataParse { reason: String },

    /// Maven metadata HTTP fetch error (connect timeout, non-2xx, or body read).
    #[error("Failed to fetch Maven metadata: {reason}")]
    MavenMetadataFetch { reason: String },
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

    #[test]
    fn test_subprocess_exit_display_includes_code_and_tail() {
        let e = LoaderError::SubprocessExit {
            code: 1,
            tail: "java.lang.NullPointerException at Foo.bar".into(),
        };
        let s = e.to_string();
        assert!(s.contains("1"), "code missing: {s}");
        assert!(s.contains("NullPointerException"), "tail missing: {s}");
    }

    #[test]
    fn test_staging_populate_display() {
        let e = LoaderError::StagingPopulate { reason: "no space left".into() };
        let s = e.to_string();
        assert!(s.contains("staging"), "headline missing: {s}");
        assert!(s.contains("no space"), "reason missing: {s}");
    }

    #[test]
    fn test_harvest_failed_display() {
        let e = LoaderError::HarvestFailed { reason: "no version dir produced".into() };
        let s = e.to_string();
        assert!(s.contains("harvest"), "headline missing: {s}");
        assert!(s.contains("no version dir"), "reason missing: {s}");
    }

    #[test]
    fn test_installer_jar_fetch_display() {
        let e = LoaderError::InstallerJarFetch { reason: "404 Not Found".into() };
        let s = e.to_string();
        assert!(s.contains("installer"), "headline missing: {s}");
        assert!(s.contains("404"), "reason missing: {s}");
    }

    #[test]
    fn test_maven_metadata_parse_display() {
        let e = LoaderError::MavenMetadataParse { reason: "empty version list".into() };
        let s = e.to_string();
        assert!(s.contains("Maven"), "headline missing: {s}");
        assert!(s.contains("empty version list"), "reason missing: {s}");
    }

    #[test]
    fn test_maven_metadata_fetch_display() {
        let e = LoaderError::MavenMetadataFetch { reason: "tls handshake failed".into() };
        let s = e.to_string();
        assert!(s.contains("Maven"), "headline missing: {s}");
        assert!(s.contains("tls handshake failed"), "reason missing: {s}");
    }
}
