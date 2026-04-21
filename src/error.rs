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

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("SHA1 mismatch for {target}: expected {expected}, got {got}")]
    Sha1Mismatch { target: String, expected: String, got: String },

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

    #[error("Process spawn failed: {0}")]
    SpawnFailed(String),

    #[error("Authentication error: {0}")]
    Auth(#[from] crate::auth::AuthError),

    #[error("No active account — add a Microsoft account or launch in offline mode")]
    NoActiveAccount,
}

pub type Result<T> = std::result::Result<T, AppError>;
