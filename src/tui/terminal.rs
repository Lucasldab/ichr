//! Terminal init, restore, and panic hook for ratatui.
//!
//! Guarantee: if the process panics, the terminal is restored BEFORE the
//! default panic handler runs, so the user's shell remains usable.
//! See PITFALLS.md Pitfall 1 (terminal left in raw mode after panic).

use std::io::{stdout, Stdout};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::Terminal;

/// Canonical `Terminal` type used throughout the TUI layer.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Install the panic hook, enter the alternate screen, enable raw mode,
/// and return a ratatui `Terminal` ready to draw.
///
/// Order matters: the panic hook is installed first so that even a panic
/// during terminal setup (e.g., `enable_raw_mode` succeeds but the next
/// call fails) will still restore the terminal.
pub fn init() -> std::io::Result<Tui> {
    install_panic_hook();
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout()))
}

/// Disable raw mode and leave the alternate screen. Idempotent — safe to
/// call from both the normal exit path and the panic hook.
pub fn restore() -> std::io::Result<()> {
    let _ = execute!(stdout(), LeaveAlternateScreen);
    let _ = disable_raw_mode();
    Ok(())
}

/// Install a panic hook that restores the terminal before the default hook
/// prints the panic message. Called automatically by `init()`.
///
/// Composed with any previously installed hook so other panic handlers
/// (e.g., from `color-eyre`) are not displaced.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Best-effort restore: ignore IO errors during panic path.
        let _ = restore();
        prev(panic_info);
    }));
}
