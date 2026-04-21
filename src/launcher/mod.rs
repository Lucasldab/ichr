//! Phase 3 launcher: compose JVM command, spawn Minecraft, drain stdio.
//!
//! Layered:
//!   * Pure composition (`command`, `substitute`, `classpath`, `offline`,
//!     `argfile`) — synchronous functions, no I/O, unit-testable on
//!     fixtures.
//!   * IO layer (`spawn`, `service`) — async, owns process lifecycle,
//!     drains stdio to the per-instance log file, updates the manifest.
//!
//! See `.planning/phases/03-launcher-process-and-offline-launch/03-RESEARCH.md`
//! for the full design. Contracts and function signatures are defined
//! per plan (03-02 onward).

pub mod argfile;
pub mod classpath;
pub mod command;
pub mod offline;
pub mod service;
pub mod spawn;
pub mod substitute;
