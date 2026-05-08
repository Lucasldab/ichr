//! Job identity and event types.

/// Monotonically-increasing job identifier, issued by `TaskManager::next_job_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "job#{}", self.0)
    }
}

/// Result type for a job body. We stringify the error before surfacing it to
/// the TUI so downstream code doesn't need to depend on every job's error type.
pub type TaskResult = std::result::Result<(), String>;

/// Events emitted by running jobs. Plan 05 will wrap these inside the TUI
/// `Action` enum; keeping them decoupled makes `tasks/` testable without
/// the TUI layer.
#[derive(Debug, Clone)]
pub enum TaskEvent {
    /// Progress update from an active job.
    Progress { id: JobId, pct: u8, msg: String },
    /// Terminal event -- either Ok or Err (including cancellation).
    Completed { id: JobId, result: TaskResult },
}
