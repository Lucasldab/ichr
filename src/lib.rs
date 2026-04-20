//! mineltui — terminal-UI Minecraft Java Edition launcher.
//!
//! Library crate; `src/main.rs` is the binary entry point.

pub mod domain;
pub mod error;
pub mod persistence;

pub use error::{AppError, Result};
