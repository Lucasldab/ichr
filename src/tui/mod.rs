//! Terminal-UI layer: ratatui init/restore, Elm-style app state, event loop.

pub mod app;
pub mod run;
pub mod terminal;
pub mod theme;
pub mod view;
pub mod views;

pub use app::{update, Action, ActiveView, AppState, CreateStep, Effect, VersionFilter};
pub use run::run;
pub use terminal::{init as init_terminal, restore as restore_terminal, Tui};
