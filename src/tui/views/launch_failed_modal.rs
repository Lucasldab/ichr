//! Launch-failed modal -- surfaces AppError::LaunchFailed's `message`
//! (the ring-buffer log tail from launcher::spawn) plus the error headline.
//!
//! Phase 3 v1 is non-scrollable. A scrollable log viewer is v2 (LOG-01).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::app::{ActiveView, AppState};

pub fn render_launch_failed_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::LaunchFailedModal {
        slug,
        error,
        log_tail,
    } = &state.active_view
    else {
        return;
    };
    let w = area.width.min(80);
    let h = area.height.min(20);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    f.render_widget(Clear, rect);
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(format!("Launch failed: {slug}   (Esc to dismiss)"));
    f.render_widget(outer, rect);

    let inner = Rect {
        x: rect.x + 1,
        y: rect.y + 1,
        width: rect.width.saturating_sub(2),
        height: rect.height.saturating_sub(2),
    };
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(inner);

    let err_p = Paragraph::new(error.as_str())
        .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .wrap(Wrap { trim: false });
    f.render_widget(err_p, split[0]);

    let tail_p = Paragraph::new(log_tail.as_str()).wrap(Wrap { trim: false });
    f.render_widget(tail_p, split[1]);
}
