//! ModpackError — stub for Plan 10-03 parallel execution.
//!
//! Plan 10-02 owns the canonical version of this file. This stub provides
//! the minimal variant set needed by download.rs so the crate compiles
//! before the Wave-2 merge resolves the dependency. Plan 10-02's full enum
//! will overlay this at merge time (3-way merge; both define the same variants).

/// All failure modes for modpack import operations.
#[derive(Debug, thiserror::Error)]
pub enum ModpackError {
    /// Download URL's host is not in the 7-host hardcoded allowlist.
    #[error("Disallowed download source: host {host:?} in URL {url:?}")]
    DisallowedSource { url: String, host: String },

    /// SHA-512 hash of the downloaded file does not match the manifest.
    #[error("Hash mismatch for {path}: expected {expected}, got {got}")]
    HashMismatch { path: String, expected: String, got: String },

    /// HTTP fetch failed (network / non-2xx / body read).
    #[error("HTTP error: {0}")]
    Http(String),

    /// Underlying I/O error.
    #[error("I/O error: {0}")]
    Io(std::io::Error),

    /// User cancelled the import via CancellationToken.
    #[error("Modpack import cancelled")]
    Cancelled,

    /// Manifest parse failed (serde_json).
    #[error("Manifest parse error: {0}")]
    ManifestParse(String),

    /// formatVersion != 1.
    #[error("Unsupported .mrpack format version {version} (expected 1)")]
    UnsupportedFormat { version: u32 },

    /// game != "minecraft".
    #[error("Unsupported game {game:?} (expected \"minecraft\")")]
    UnsupportedGame { game: String },

    /// dependencies map is missing the "minecraft" key.
    #[error("Missing minecraft dependency in dependencies map")]
    MissingMinecraftDependency,

    /// ZIP extraction error.
    #[error("ZIP error: {0}")]
    Zip(String),
}

impl From<std::io::Error> for ModpackError {
    fn from(e: std::io::Error) -> Self {
        ModpackError::Io(e)
    }
}
