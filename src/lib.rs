//! mineltui — terminal-UI Minecraft Java Edition launcher.
//!
//! Library crate; `src/main.rs` is the binary entry point.

pub mod auth;
pub mod domain;
pub mod error;
pub mod install;
pub mod java;
pub mod instance;
pub mod launcher;
pub mod loader;
pub mod mods;
pub mod mojang;
pub mod observability;
pub mod persistence;
pub mod services;
pub mod tasks;
pub mod tui;

pub use error::{AppError, Result};
