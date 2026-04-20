//! File-based tracing setup.
//!
//! Writes structured logs to `{data_dir}/mineltui.log` using
//! `tracing-appender`'s non-blocking writer so the render loop is
//! never blocked by log I/O.
//!
//! IMPORTANT: the returned `WorkerGuard` MUST be bound to a named
//! variable in `main()` for the entire process lifetime. Dropping it
//! early flushes the buffer and shuts down the background writer,
//! silently discarding any subsequent log events. See PITFALLS.md
//! Pitfall 3 in the Phase 1 research file.

use std::fs::OpenOptions;

use anyhow::Context;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::persistence::AppPaths;

/// Initialize the global tracing subscriber to write to
/// `{paths.data_dir}/mineltui.log` with ANSI colors disabled.
///
/// The returned `WorkerGuard` must be held for the process lifetime.
///
/// Returns an error if the global subscriber has already been installed
/// (i.e., `init` was called a second time in the same process).
pub fn init(paths: &AppPaths) -> anyhow::Result<WorkerGuard> {
    let log_path = paths.log_file();

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("creating log file parent directory {}", parent.display())
        })?;
    }

    // Open append-create so multiple runs accumulate history.
    // No rotation yet — Phase 12 or v2 may add `tracing-appender::rolling`.
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening log file {}", log_path.display()))?;

    let (non_blocking, guard) = tracing_appender::non_blocking(file);

    let filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new("mineltui=debug,info"));

    let fmt_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_file(true)
        .with_line_number(true)
        .with_target(false);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .try_init()
        .context("installing global tracing subscriber")?;

    Ok(guard)
}
