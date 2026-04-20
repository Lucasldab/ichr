//! Terminal-UI layer: ratatui init/restore, Elm-style app state, event loop.

pub mod app;
pub mod run;
pub mod terminal;
pub mod view;

pub use app::{update, Action, AppState, Effect};
pub use run::run;
pub use terminal::{init as init_terminal, restore as restore_terminal, Tui};
