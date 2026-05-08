//! MrpackFile and related parse types — stub for Plan 10-03 parallel execution.
//!
//! Plan 10-02 owns the canonical version of this file. This stub provides
//! the serde types and helper functions needed by download.rs so the crate
//! compiles before the Wave-2 merge resolves the dependency. Plan 10-02's
//! full implementation will overlay this at merge time.

use serde::{Deserialize, Serialize};

/// One entry in a Modrinth modpack's `files[]` array.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackFile {
    /// Destination path, relative to the instance's `.minecraft/` directory.
    /// May include a leading `./` (strip before use).
    pub path: String,

    /// Hash values for integrity verification.
    pub hashes: MrpackHashes,

    /// Optional `env` block controlling client/server applicability.
    #[serde(default)]
    pub env: Option<MrpackEnv>,

    /// HTTPS download URLs (at least one required by spec).
    pub downloads: Vec<String>,

    /// Declared file size in bytes (0 if absent). Used for progress display.
    #[serde(default)]
    pub file_size: u64,
}

/// Hash values for a `MrpackFile`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MrpackHashes {
    /// SHA-1 hex string (legacy; recorded but not used as primary).
    pub sha1: String,
    /// SHA-512 hex string (primary verification hash per spec).
    pub sha512: String,
}

/// `env` block indicating which environments require this file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MrpackEnv {
    /// Whether this file should be downloaded on the client.
    pub client: EnvRequirement,
    /// Whether this file should be installed on the server.
    pub server: EnvRequirement,
}

/// `env.client` / `env.server` value as defined by the Modrinth `.mrpack` spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EnvRequirement {
    /// File is required for this environment.
    #[default]
    Required,
    /// File is optional for this environment (installer still installs it in v1).
    Optional,
    /// File is not applicable to this environment (skip silently).
    Unsupported,
}

/// Returns `true` when the file should be downloaded for a client install.
///
/// Per PACK-02 and Pitfall 3:
/// - `env` absent → treat as required (universal file).
/// - `env.client == Unsupported` → skip.
/// - `env.client == Required | Optional` → download.
pub fn should_download_for_client(env: Option<&MrpackEnv>) -> bool {
    match env {
        None => true,
        Some(e) => e.client != EnvRequirement::Unsupported,
    }
}

/// Strip a leading `./` from a path string (per RESEARCH.md §Open Questions #1).
///
/// Removes the `./` prefix that some packs include in `files[].path`.
/// This cannot enable path traversal: removing `./` from `./../../etc/passwd`
/// yields `../../etc/passwd`, which `safe_zip_path` still rejects.
pub fn strip_leading_dot_slash(path: &str) -> &str {
    path.strip_prefix("./").unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_download_for_client_required() {
        let env = MrpackEnv { client: EnvRequirement::Required, server: EnvRequirement::Required };
        assert!(should_download_for_client(Some(&env)));
    }

    #[test]
    fn test_should_download_for_client_optional() {
        let env = MrpackEnv { client: EnvRequirement::Optional, server: EnvRequirement::Required };
        assert!(should_download_for_client(Some(&env)));
    }

    #[test]
    fn test_should_download_for_client_unsupported() {
        let env =
            MrpackEnv { client: EnvRequirement::Unsupported, server: EnvRequirement::Required };
        assert!(!should_download_for_client(Some(&env)));
    }

    #[test]
    fn test_should_download_for_client_no_env() {
        assert!(should_download_for_client(None));
    }

    #[test]
    fn test_strip_leading_dot_slash() {
        assert_eq!(strip_leading_dot_slash("./mods/foo.jar"), "mods/foo.jar");
        assert_eq!(strip_leading_dot_slash("mods/foo.jar"), "mods/foo.jar");
        assert_eq!(strip_leading_dot_slash("./"), "");
    }
}
