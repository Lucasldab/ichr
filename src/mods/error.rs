//! Typed errors for the Modrinth integration module.
//!
//! Library-layer errors. Convert to `AppError` at the `execute_effects`
//! boundary in `src/tui/run.rs` (or surface directly via
//! `Action::ModInstallFailed { error: e.to_string(), .. }`).
//!
//! Variants enumerated in 08-PATTERNS.md §`src/mods/error.rs` deltas.

/// All failure modes for `ModrinthService` operations.
///
/// Stub introduced by 08-01 Task 1 to keep the crate building; Task 2
/// of this same plan replaces this with the full 11-variant enum.
#[derive(Debug, thiserror::Error)]
pub enum ModrinthError {
    /// Underlying I/O error (filesystem, atomic_write, fs::rename, fs::remove_file).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
