//! `TaskManager` — bounded concurrent job pool with per-job cancellation.
//!
//! Invariants (see PITFALLS.md Pitfall 4, 17):
//!   * All `tx.send(...)` calls use `let _ =` — never `.unwrap()`; the receiver
//!     being dropped is a legitimate shutdown signal.
//!   * Job bodies must not hold a blocking I/O call across `.await`.
//!     For blocking work use `tokio::task::spawn_blocking` inside the job body.
//!   * The semaphore permit is acquired BEFORE the job body runs so
//!     `tokio::spawn` submission itself is unbounded but execution is capped.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use super::job::{JobId, TaskEvent};

pub struct TaskManager {
    semaphore: Arc<Semaphore>,
    event_tx: mpsc::Sender<TaskEvent>,
    root_token: CancellationToken,
    next_id: Arc<AtomicU64>,
    max_concurrent: usize,
}

impl TaskManager {
    pub fn new(event_tx: mpsc::Sender<TaskEvent>, max_concurrent: usize) -> Self {
        assert!(max_concurrent > 0, "max_concurrent must be > 0");
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            event_tx,
            root_token: CancellationToken::new(),
            next_id: Arc::new(AtomicU64::new(1)),
            max_concurrent,
        }
    }

    /// Issue a fresh `JobId`. Monotonic, never repeats for the manager lifetime.
    pub fn next_job_id(&self) -> JobId {
        JobId(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Number of semaphore permits currently available. Diagnostic / test-only.
    pub fn active_permits_available(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Cancel every currently-running child job.
    pub fn cancel_all(&self) {
        self.root_token.cancel();
    }

    /// Spawn a new job. Returns the child `CancellationToken` so callers
    /// can cancel this single job without touching siblings.
    ///
    /// The job body receives (sender, cancellation_token). The body SHOULD
    /// periodically check `token.is_cancelled()` during long-running loops
    /// so it can short-circuit cleanup work; however, the `select!` below
    /// will always observe cancellation and fire `Completed { Err("Cancelled") }`
    /// even if the body does not cooperate.
    pub fn spawn_task<F, Fut>(&self, job_id: JobId, f: F) -> CancellationToken
    where
        F: FnOnce(mpsc::Sender<TaskEvent>, CancellationToken) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = std::result::Result<(), anyhow::Error>> + Send + 'static,
    {
        let child_token = self.root_token.child_token();
        let tx = self.event_tx.clone();
        let sem = Arc::clone(&self.semaphore);
        let token_for_job = child_token.clone();

        tokio::spawn(async move {
            // Acquire a permit BEFORE running the body. `acquire_owned` returns
            // an error only if the semaphore is closed; we never close it.
            let _permit = match sem.acquire_owned().await {
                Ok(p) => p,
                Err(_) => {
                    // Semaphore closed: treat as cancellation.
                    let _ = tx
                        .send(TaskEvent::Completed {
                            id: job_id,
                            result: Err("Semaphore closed".to_string()),
                        })
                        .await;
                    return;
                }
            };

            let fut = f(tx.clone(), token_for_job.clone());
            tokio::select! {
                biased;
                _ = token_for_job.cancelled() => {
                    let _ = tx
                        .send(TaskEvent::Completed {
                            id: job_id,
                            result: Err("Cancelled".to_string()),
                        })
                        .await;
                }
                res = fut => {
                    let completed = match res {
                        Ok(()) => TaskEvent::Completed { id: job_id, result: Ok(()) },
                        Err(e) => TaskEvent::Completed {
                            id: job_id,
                            result: Err(format!("{e:#}")),
                        },
                    };
                    let _ = tx.send(completed).await;
                }
            }
        });

        child_token
    }
}

impl std::fmt::Debug for TaskManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskManager")
            .field("max_concurrent", &self.max_concurrent)
            .field("available_permits", &self.semaphore.available_permits())
            .finish()
    }
}
