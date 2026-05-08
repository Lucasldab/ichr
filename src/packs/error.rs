//! Typed errors for the resource-pack / shader-pack module.
//!
//! Mirrors `src/mods/error.rs` structure. Variants enumerated in
//! 11-01-PLAN.md must_haves. `FileTooLarge` carries the file size in bytes
//! and the configured cap so the user can see why the import was rejected.

/// All failure modes for pack operations (drop-from-path, Modrinth install,
/// enable/disable, remove). Used by Plans 02/03/04.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// Requested pack file or ledger row does not exist.
    #[error("Pack not found: {path}")]
    NotFound { path: String },

    /// File is not a `.zip` archive (wrong extension or not a regular file).
    #[error("Not a ZIP archive: {path}")]
    NotAZip { path: String },

    /// File exceeds the configured size cap (`MAX_PACK_FILE_BYTES`).
    #[error("File too large: {bytes} bytes (cap is {cap} bytes)")]
    FileTooLarge { bytes: u64, cap: u64 },

    /// A pack with this filename is already installed in the instance.
    #[error("A pack with that filename is already installed; remove it first")]
    FilenameCollision,

    /// Filename contains path-traversal or disallowed characters.
    #[error("Unsafe pack filename: {filename}")]
    UnsafeFilename { filename: String },

    /// Underlying Modrinth operation failed (HTTP, parse, etc.).
    #[error("Modrinth error: {0}")]
    Modrinth(#[from] crate::mods::error::ModrinthError),

    /// Underlying I/O error (filesystem, copy, rename, remove).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// User cancelled the operation (Esc / CancellationToken).
    /// Treated as a clean return at the TUI boundary — not an error.
    #[error("Pack operation cancelled")]
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_found_display_contains_path() {
        let e = PackError::NotFound { path: "/tmp/Faithful.zip".into() };
        let s = e.to_string();
        assert!(s.contains("/tmp/Faithful.zip"), "path missing: {s}");
    }

    #[test]
    fn test_not_a_zip_display_contains_path() {
        let e = PackError::NotAZip { path: "pack.tar.gz".into() };
        let s = e.to_string();
        assert!(s.contains("pack.tar.gz"), "path missing: {s}");
    }

    #[test]
    fn test_file_too_large_display_contains_bytes_and_cap() {
        let e = PackError::FileTooLarge {
            bytes: 600 * 1024 * 1024,
            cap: 500 * 1024 * 1024,
        };
        let s = e.to_string();
        // Both byte values appear in the message.
        assert!(s.contains("629145600") || s.contains("600"), "file size missing: {s}");
        assert!(s.contains("524288000") || s.contains("500"), "cap missing: {s}");
    }

    #[test]
    fn test_filename_collision_display_static_message() {
        let e = PackError::FilenameCollision;
        let s = e.to_string();
        assert!(s.contains("already installed"), "headline missing: {s}");
    }

    #[test]
    fn test_unsafe_filename_display_contains_filename() {
        let e = PackError::UnsafeFilename { filename: "../etc/passwd.zip".into() };
        let s = e.to_string();
        assert!(s.contains("../etc/passwd.zip"), "filename missing: {s}");
    }

    #[test]
    fn test_modrinth_wrap_via_from() {
        let inner = crate::mods::error::ModrinthError::Http("connect timeout".into());
        let e: PackError = inner.into();
        let s = e.to_string();
        assert!(s.contains("Modrinth error"), "Modrinth wrapper missing: {s}");
        assert!(s.contains("connect timeout"), "inner message missing: {s}");
    }

    #[test]
    fn test_io_wrap_via_from() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let e: PackError = io_err.into();
        let s = e.to_string();
        assert!(s.contains("I/O error"), "I/O headline missing: {s}");
        assert!(s.contains("denied"), "io message missing: {s}");
    }

    #[test]
    fn test_cancelled_display_static_message() {
        let e = PackError::Cancelled;
        assert_eq!(e.to_string(), "Pack operation cancelled");
    }
}
