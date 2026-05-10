//! Terminal init, restore, and panic hook for ratatui.
//!
//! Guarantee: if the process panics, the terminal is restored BEFORE the
//! default panic handler runs, so the user's shell remains usable.
//! See PITFALLS.md Pitfall 1 (terminal left in raw mode after panic).

use std::io::{stdout, Stdout};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::Terminal;
use ratatui_image::picker::Picker;

/// Canonical `Terminal` type used throughout the TUI layer.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Install the panic hook, run image-protocol detection (Phase 13),
/// enter the alternate screen, enable raw mode, enable bracketed paste,
/// and return the ratatui `Terminal` plus the detected picker.
///
/// Order matters: the panic hook is installed first so that even a panic
/// during terminal setup (e.g., `enable_raw_mode` succeeds but the next
/// call fails) will still restore the terminal.
///
/// Image-protocol detection runs BEFORE `enable_raw_mode()` because
/// `Picker::from_query_stdio()` reads escape-sequence replies on stdin
/// and needs the terminal in cooked mode. On detection failure (timeout,
/// unsupported terminal, missing terminfo) we log a warn and return
/// `None`; downstream code treats `None` as "icons disabled" and falls
/// through to the existing text-only render path.
///
/// Bracketed paste (DECSET `?2004h`) is enabled so that pasted text arrives
/// as a single `Event::Paste(String)` instead of a stream of synthetic key
/// events. Terminals without bracketed-paste support fall through to the
/// per-character `KeyEvent` path. See GAP-8-C / 08.1-04.
pub fn init() -> std::io::Result<(Tui, Option<Picker>)> {
    install_panic_hook();
    // Probe the terminal for image-protocol support BEFORE flipping raw
    // mode -- the query reads stdin and needs cooked mode to receive the
    // terminal's reply. Failures are non-fatal: ichr just runs without
    // icons, exactly like users on gnome-terminal / xterm / Konsole.
    let picker = match Picker::from_query_stdio() {
        Ok(p) => {
            tracing::debug!(
                protocol = ?p.protocol_type(),
                "image protocol detected"
            );
            Some(p)
        }
        Err(e) => {
            tracing::warn!(error = %e, "image protocol detection failed -- icons disabled");
            None
        }
    };
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    Ok((terminal, picker))
}

/// Disable bracketed paste, leave the alternate screen, and disable raw
/// mode (in that order). Idempotent -- safe to call from both the normal
/// exit path and the panic hook. Best-effort: errors are intentionally
/// swallowed so the panic-restore path does not itself panic.
pub fn restore() -> std::io::Result<()> {
    let _ = execute!(stdout(), DisableBracketedPaste);
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
