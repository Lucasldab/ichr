//! Small palette-aware widget helpers. Centralizes the "what color does
//! an idle border use" question so every view doesn't redeclare the
//! same `border_style` chain. Active/focused borders are still set
//! per-view (mod_browser, pack_browser, cf_browser) because the
//! focused-vs-idle distinction is view-local state.

use ratatui::style::Style;
use ratatui::widgets::{Block, Borders};

use crate::config::Palette;

/// Style for inactive frame borders -- reads `palette.frame_idle`.
pub fn idle_border(palette: &Palette) -> Style {
    Style::default().fg(palette.frame_idle.to_color())
}

/// Convenience constructor: `Block::default().borders(Borders::ALL)` with
/// the idle border color preset. Chain `.title(...)` on top as needed.
pub fn block(palette: &Palette) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(idle_border(palette))
}
