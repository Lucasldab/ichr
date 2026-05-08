//! Cross-cutting observability: logging, (eventually) metrics and tracing spans.
//!
//! Rule: NEVER write diagnostic output to stdout/stderr -- it corrupts the
//! TUI's alternate screen. Everything routes through `tracing::*` macros,
//! and the subscriber configured in `logging::init` sends them to a file.

pub mod logging;
