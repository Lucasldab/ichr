//! Async event loop. Reads crossterm events, `TaskEvent`s, and a render tick,
//! dispatches them through `update`, executes any returned `Effect`s, and
//! redraws.
//!
//! Invariants (see PITFALLS.md 17, 4):
//!   * No blocking I/O inside `.await` bodies here.
//!   * `action_tx.send(..)` in background arms uses `let _ =` — receiver
//!     being dropped is a valid shutdown signal.

use std::time::Duration;

use futures::StreamExt;
use ratatui::crossterm::event::{Event as CtEvent, EventStream, KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use tokio::time::interval;

use super::app::{update, Action, AppState, Effect};
use super::terminal::Tui;
use super::view::view;
use crate::domain::platform::{Arch, OsName};
use crate::tasks::{TaskEvent, TaskManager, DEFAULT_MAX_CONCURRENT};

/// Run the TUI event loop until `Action::Quit` is dispatched.
///
/// Consumes the terminal; callers must call `tui::restore_terminal` after
/// this returns (whether Ok or Err).
pub async fn run(mut terminal: Tui) -> anyhow::Result<()> {
    // Build the task plumbing. TaskEvents arrive on task_rx; we convert each
    // into an Action::Task and forward to the event loop via action_tx.
    let (action_tx, mut action_rx) = mpsc::channel::<Action>(256);
    let (task_tx, mut task_rx) = mpsc::channel::<TaskEvent>(256);
    let _task_manager = TaskManager::new(task_tx, DEFAULT_MAX_CONCURRENT);

    // Pump TaskEvents into the Action bus.
    {
        let action_tx = action_tx.clone();
        tokio::spawn(async move {
            while let Some(evt) = task_rx.recv().await {
                if action_tx.send(Action::Task(evt)).await.is_err() {
                    break; // event loop shut down — receiver dropped
                }
            }
        });
    }

    let mut state = AppState {
        arch: Some(Arch::current()),
        os: Some(OsName::current()),
        ..AppState::default()
    };

    let mut events = EventStream::new();
    let mut render_tick = interval(Duration::from_millis(16));
    // Interval fires immediately on the first poll; consume that initial tick
    // so the first real render happens after the loop starts.
    render_tick.tick().await;

    // Initial render before entering the select loop.
    terminal.draw(|f| view(&state, f))?;

    loop {
        tokio::select! {
            biased;

            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(ev)) => {
                        if let Some(action) = map_event(ev) {
                            let effects = update(&mut state, action);
                            execute_effects(effects, &action_tx).await;
                        }
                    }
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "crossterm event stream error");
                    }
                    None => break, // stream closed
                }
            }

            maybe_action = action_rx.recv() => {
                match maybe_action {
                    Some(action) => {
                        let effects = update(&mut state, action);
                        execute_effects(effects, &action_tx).await;
                    }
                    None => break, // all senders dropped
                }
            }

            _ = render_tick.tick() => {
                terminal.draw(|f| view(&state, f))?;
            }
        }

        if state.should_quit {
            break;
        }
    }

    Ok(())
}

/// Translate a crossterm event into an `Action`, if any.
fn map_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Char('q'), .. }) => Some(Action::Quit),
        CtEvent::Key(KeyEvent { code: KeyCode::Char('c'), modifiers, .. })
            if modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(Action::Quit)
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::Quit),
        _ => None,
    }
}

/// Execute declarative side-effects. Keeps `update` pure.
/// Full wiring is implemented in Task 2-07-03; this stub makes 01a/01b compile.
async fn execute_effects(effects: Vec<Effect>, _tx: &mpsc::Sender<Action>) {
    for ef in effects {
        match ef {
            Effect::Quit => {
                // No additional work — the `should_quit` flag in AppState
                // drives the event loop to break on the next iteration.
            }
            Effect::FetchManifest => {}
            Effect::FetchInstances => {}
            Effect::DeleteInstance(_) => {}
            Effect::RenameInstance { .. } => {}
            Effect::CloneInstance { .. } => {}
            Effect::CreateInstance { .. } => {}
        }
    }
}
