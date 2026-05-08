//! Top-level error type for the library layer of `mineltui`.
//!
//! Library/domain code returns `Result<T, AppError>`.
//! The `main.rs` / TUI boundary uses `anyhow::Result` (added by Plan 05).

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Path resolution failed: platform home directory unavailable")]
    PathResolution,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Mojang JSON parse error: {0}")]
    MojangParse(#[from] serde_json::Error),

    #[error("inheritsFrom chain cycle detected at {0}")]
    InheritsFromCycle(String),

    #[error("inheritsFrom chain exceeded max depth {max} at {current}")]
    InheritsFromDepthExceeded { current: String, max: u32 },

    #[error("inheritsFrom parent {0} not present in parents map (caller must pre-fetch)")]
    InheritsFromParentMissing(String),

    /// The `inheritsFrom` chain resolved without populating a vanilla-required
    /// field (`asset_index`, `assets`, or `downloads`). Real Fabric/Quilt/Forge/
    /// NeoForge loader JSONs are metadata-only and depend on the vanilla parent
    /// supplying these fields; if the vanilla parent is itself missing the field
    /// (truncated install, hand-rolled JSON, future Mojang format change) we
    /// surface a typed error instead of panicking on `.unwrap()`.
    #[error("version `{version_id}` and its inheritsFrom chain do not declare required field `{field}` (Fabric/Quilt loaders inherit this from vanilla — install the parent vanilla version first)")]
    InheritsFromMissingRequired { field: String, version_id: String },

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("SHA1 mismatch for {target}: expected {expected}, got {got}")]
    Sha1Mismatch {
        target: String,
        expected: String,
        got: String,
    },

    #[error("Instance manifest serde error: {0}")]
    InstanceSerde(String),

    #[error("Invalid instance name: {reason}")]
    InvalidInstanceName { reason: String },

    #[error("Instance not found: {slug}")]
    InstanceNotFound { slug: String },

    #[error("Launch failed (exit code {code}): {message}")]
    LaunchFailed { code: i32, message: String },

    #[error("Version not installed for instance {slug} — run install first")]
    VersionNotInstalled { slug: String },

    #[error("Java binary not found: checked MINELTUI_JAVA env var and PATH")]
    JavaNotFound,

    #[error(
        "Java override path does not exist: {path:?} — fix or clear instance.json `java_override`"
    )]
    JavaOverrideNotFound { path: std::path::PathBuf },

    #[error("Java major version mismatch: Minecraft requires Java {required}, found Java {found} at {path:?} — {hint}")]
    JavaMismatch {
        required: u32,
        found: u32,
        path: std::path::PathBuf,
        hint: String,
    },

    #[error("Failed to download Java runtime ({variant}): {reason}")]
    JavaDownloadFailed { variant: String, reason: String },

    #[error("Failed to extract Java runtime to {dest:?}: {reason}")]
    JavaExtractFailed {
        dest: std::path::PathBuf,
        reason: String,
    },

    #[error("Process spawn failed: {0}")]
    SpawnFailed(String),

    #[error("Authentication error: {0}")]
    Auth(#[from] crate::auth::AuthError),

    #[error("Modrinth error: {0}")]
    Modrinth(#[from] crate::mods::error::ModrinthError),

    #[error("CurseForge error: {0}")]
    CurseForge(#[from] crate::mods::curseforge::error::CurseForgeError),

    #[error("Modpack error: {0}")]
    Modpack(#[from] crate::modpack::error::ModpackError),

    #[error("No active account — add a Microsoft account or launch in offline mode")]
    NoActiveAccount,
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod error_display_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_java_mismatch_display_contains_both_versions() {
        let e = AppError::JavaMismatch {
            required: 21,
            found: 17,
            path: PathBuf::from("/usr/bin/java"),
            hint: "Install Java 21".into(),
        };
        let s = e.to_string();
        assert!(s.contains("21"), "expected '21' in: {s}");
        assert!(s.contains("17"), "expected '17' in: {s}");
    }

    #[test]
    fn test_java_download_failed_display() {
        let e = AppError::JavaDownloadFailed {
            variant: "adoptium-21".into(),
            reason: "timeout".into(),
        };
        let s = e.to_string();
        assert!(s.contains("adoptium-21"), "expected variant in: {s}");
        assert!(s.contains("timeout"), "expected reason in: {s}");
    }

    #[test]
    fn test_java_extract_failed_display() {
        let e = AppError::JavaExtractFailed {
            dest: PathBuf::from("/tmp/jre"),
            reason: "corrupt archive".into(),
        };
        let s = e.to_string();
        assert!(s.contains("jre"), "expected dest in: {s}");
        assert!(s.contains("corrupt archive"), "expected reason in: {s}");
    }
}
