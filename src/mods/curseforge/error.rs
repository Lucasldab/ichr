//! Stub created by 09-01; populated in Task 2 of this plan with the full
//! `CurseForgeError` enum (thiserror, 9 variants) per 09-PATTERNS.md §`src/mods/curseforge/error.rs`.
//!
//! The minimal definition below exists only to keep the crate compiling
//! between Task 1 and Task 2; Task 2 replaces the entire enum.

#[derive(Debug, thiserror::Error)]
pub enum CurseForgeError {
    /// I/O error placeholder — Task 2 replaces this enum with the full 9-variant set.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
