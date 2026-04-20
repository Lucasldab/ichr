//! mineltui binary entry point.
//!
//! Startup order (critical — see PITFALLS.md):
//!   1. Resolve AppPaths (pre-TUI; plain errors print to stderr normally)
//!   2. Initialize logging (pre-TUI, same reason; guard MUST be bound for
//!      the entire `main` to avoid silent log loss — see Pitfall 3)
//!   3. Install panic hook + enter raw mode + alternate screen
//!   4. Run the event loop
//!   5. Restore the terminal (always, even on error)

use anyhow::Context;

use mineltui::observability::logging;
use mineltui::persistence::AppPaths;
use mineltui::tui;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // 1. Paths — must succeed before any terminal manipulation.
    let paths = AppPaths::resolve()
        .context("resolving platform paths (no home directory available)")?;

    // 2. Logging — must live until the end of main.
    //    Bound as `_log_guard` (not `_`) so Rust does not drop it immediately.
    let _log_guard = logging::init(&paths)
        .context("initializing file logging")?;
    tracing::info!(
        data_dir = %paths.data_dir.display(),
        config_dir = %paths.config_dir.display(),
        cache_dir = %paths.cache_dir.display(),
        "mineltui starting"
    );

    // 3. Terminal — panic hook is installed inside tui::init_terminal so that
    //    even a panic during setup leaves the terminal in a usable state.
    let terminal = tui::init_terminal().context("initializing terminal")?;

    // 4. Event loop. Restore on both success and error paths.
    let run_result = tui::run(terminal).await;

    // 5. Restore. Ignore secondary errors — we're already exiting and cannot
    //    write to a potentially dead terminal.
    let _ = tui::restore_terminal();

    if let Err(ref e) = run_result {
        // Logging is still live here (guard not yet dropped).
        tracing::error!(error = ?e, "TUI event loop terminated with error");
    } else {
        tracing::info!("mineltui exiting cleanly");
    }

    run_result
}
