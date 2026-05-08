//! Typed errors for the modpack import module.
//!
//! All failure modes for `.mrpack` v1 import: manifest parse, validation,
//! download allowlist enforcement, hash verification, zip extraction, and
//! cancellation. Converts to `AppError` via `AppError::Modpack(#[from] ModpackError)`.

/// All failure modes for `.mrpack` v1 import.
#[derive(Debug, thiserror::Error)]
pub enum ModpackError {
    /// `formatVersion` in the manifest was not `1`.
    #[error("unsupported format version: {version} (expected 1)")]
    UnsupportedFormat { version: u32 },

    /// `game` in the manifest was not `"minecraft"`.
    #[error("unsupported game type: {game} (expected \"minecraft\")")]
    UnsupportedGame { game: String },

    /// `modrinth.index.json` could not be parsed as valid JSON or did not
    /// match the expected schema.
    ///
    /// NOT `#[from]` — `serde_json::Error` implements `Into<std::io::Error>`,
    /// which would create a conflicting `From` impl alongside
    /// `Io(#[from] std::io::Error)`. Callers wrap via
    /// `.map_err(ModpackError::ManifestParse)`.
    #[error("manifest parse error: {0}")]
    ManifestParse(serde_json::Error),

    /// The `dependencies` map in the manifest does not contain the required
    /// `"minecraft"` key.
    #[error("missing minecraft dependency in dependencies map")]
    MissingMinecraftDependency,

    /// The `dependencies` map contains a modloader key that is not one of the
    /// four supported loaders (fabric-loader, quilt-loader, forge, neoforge).
    #[error("unsupported modloader in dependencies: {loader}")]
    UnsupportedLoader { loader: String },

    /// A `downloads[]` URL in the manifest points to a host that is not on the
    /// hardcoded allowlist. Carries both the rejecting host and the manifest
    /// `path` field so the user can identify which file to investigate.
    #[error("disallowed download source {host} in file {path}")]
    DisallowedSource { host: String, path: String },

    /// SHA-512 hash of a downloaded file did not match the manifest.
    #[error("SHA-512 mismatch for {path}: expected {expected}, got {got}")]
    HashMismatch { path: String, expected: String, got: String },

    /// ZIP extraction error (reading `.mrpack` archive or override zip entries).
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    /// Underlying filesystem I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// HTTP error during file download (non-2xx response, connection failure, etc.).
    #[error("http error: {0}")]
    Http(String),

    /// Import was cancelled by the user via `CancellationToken`.
    #[error("cancelled")]
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;

    #[test]
    fn test_modpack_error_display_includes_field_data() {
        let e = ModpackError::UnsupportedFormat { version: 2 };
        let s = e.to_string();
        assert!(s.contains("2"), "expected '2' in display: {s}");
        assert!(s.contains("unsupported format version"), "missing headline: {s}");
    }

    #[test]
    fn test_apperror_from_modpack_error_compiles() {
        // Compile-time proof that AppError::Modpack(#[from] ModpackError) works.
        let e: AppError = ModpackError::Cancelled.into();
        let s = e.to_string();
        assert!(s.contains("cancelled"), "expected 'cancelled' in: {s}");
    }

    #[test]
    fn test_io_from_conversion_works() {
        let e: ModpackError = std::io::Error::other("x").into();
        assert!(matches!(e, ModpackError::Io(_)));
    }
}
