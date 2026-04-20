//! Elm-style app state, Action enum, and `update()` dispatcher.
//!
//! The `update` function is the ONLY place state mutation happens.
//! Background tasks communicate via `Action::Task(TaskEvent)`.

use crate::domain::platform::{Arch, OsName};
use crate::tasks::{JobId, TaskEvent};

/// All mutable UI state. Extend in later phases; keep mutations confined to `update()`.
#[derive(Debug, Default)]
pub struct AppState {
    pub should_quit: bool,
    /// Summary of currently-known jobs: (id, pct, last message).
    /// Phase 1 displays nothing from this; it proves the wiring works end-to-end.
    pub active_jobs: Vec<(JobId, u8, String)>,
    pub arch: Option<Arch>,
    pub os: Option<OsName>,
}

/// All inputs to `update()`. Exhaustive in Phase 1; later phases add variants.
#[derive(Debug, Clone)]
pub enum Action {
    /// User-requested shutdown (pressed `q` or received Ctrl+C).
    Quit,
    /// Background job event bubbling up through the action bus.
    Task(TaskEvent),
    /// Render-tick heartbeat (currently unused by state transitions).
    Tick,
}

/// Declarative side-effect requests that `update()` emits. The event loop
/// executes them after each update.
#[derive(Debug)]
pub enum Effect {
    Quit,
}

/// Apply an `Action`, mutate `state`, and return the side-effects to execute.
///
/// This is the single mutation point for all UI state. Views receive `&AppState`
/// (immutable) — no other code path mutates `AppState`.
pub fn update(state: &mut AppState, action: Action) -> Vec<Effect> {
    match action {
        Action::Quit => {
            state.should_quit = true;
            vec![Effect::Quit]
        }
        Action::Task(TaskEvent::Progress { id, pct, msg }) => {
            // Upsert the job entry; keep the list short & simple for Phase 1.
            match state.active_jobs.iter_mut().find(|(jid, _, _)| *jid == id) {
                Some(slot) => {
                    slot.1 = pct;
                    slot.2 = msg;
                }
                None => state.active_jobs.push((id, pct, msg)),
            }
            vec![]
        }
        Action::Task(TaskEvent::Completed { id, result }) => {
            state.active_jobs.retain(|(jid, _, _)| *jid != id);
            let _ = result; // Phase 2+ will surface success/failure in the UI
            vec![]
        }
        Action::Tick => vec![],
    }
}
