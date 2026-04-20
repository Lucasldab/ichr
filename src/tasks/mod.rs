//! Background task system. `TaskManager` owns a bounded semaphore and a
//! progress channel; each job receives a `CancellationToken` so the TUI
//! can cancel individual work without touching the rest.
//!
//! Design: see `.planning/research/ARCHITECTURE.md` (Pattern 2) and
//! `.planning/phases/01-project-scaffold-and-core-infrastructure/01-RESEARCH.md`
//! (Pattern 4).

pub mod cancel;
pub mod job;
pub mod manager;

pub use cancel::CancellationToken;
pub use job::{JobId, TaskEvent, TaskResult};
pub use manager::TaskManager;

/// Default maximum concurrent jobs. Chosen per ARCHITECTURE.md
/// (download semaphore size) and PITFALLS.md download-bounding guidance.
pub const DEFAULT_MAX_CONCURRENT: usize = 8;
