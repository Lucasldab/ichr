//! mineltui — terminal-UI Minecraft Java Edition launcher.
//!
//! Library crate; `src/main.rs` is the binary entry point.

pub mod domain;
pub mod error;
pub mod mojang;
pub mod observability;
pub mod persistence;
pub mod tasks;
pub mod tui;

pub use error::{AppError, Result};
