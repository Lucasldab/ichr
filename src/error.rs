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
}

pub type Result<T> = std::result::Result<T, AppError>;
