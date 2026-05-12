//! Small palette-aware widget helpers. Centralizes the "what color does
//! an idle border use" question so every view doesn't redeclare the
//! same `border_style` chain. Active/focused borders are still set
//! per-view (mod_browser, pack_browser, cf_browser) because the
//! focused-vs-idle distinction is view-local state.

use ratatui::style::Style;
use ratatui::widgets::{Block, Borders};

use crate::config::Palette;

/// Style for inactive frame borders -- reads `palette.dim`.
pub fn dim_border(palette: &Palette) -> Style {
    Style::default().fg(palette.dim.to_color())
}

/// Convenience constructor: `Block::default().borders(Borders::ALL)` with
/// the idle border color preset. Chain `.title(...)` on top as needed.
pub fn block(palette: &Palette) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(dim_border(palette))
}
