//! ichr — terminal-UI Minecraft Java Edition launcher.
//!
//! Library crate; `src/main.rs` is the binary entry point.

pub mod auth;
pub mod domain;
pub mod error;
pub mod install;
pub mod instance;
pub mod java;
pub mod launcher;
pub mod loader;
pub mod modpack;
pub mod mods;
pub mod mojang;
pub mod observability;
pub mod packs;
pub mod persistence;
pub mod services;
pub mod tasks;
pub mod tui;
pub mod util;

pub use error::{AppError, Result};
