//! AUTH-02 failure modal. Shows the XSTS-mapped reason string +
//! Esc-to-dismiss hint.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::app::{ActiveView, AppState};

pub fn render_account_auth_failed(f: &mut Frame, area: Rect, state: &AppState) {
    let reason = match &state.active_view {
        ActiveView::AccountAuthFailed { reason } => reason.clone(),
        _ => return,
    };
    let palette = &state.config.colors;
    let area = centered_rect(60, 40, area);
    f.render_widget(Clear, area);
    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).split(area);
    let body = Paragraph::new(reason).wrap(Wrap { trim: true }).block(
        crate::tui::theme::block(palette)
            .title(" Authentication Failed "),
    );
    f.render_widget(body, chunks[0]);
    let hint =
        Paragraph::new(Line::from("Esc to dismiss")).block(crate::tui::theme::block(palette));
    f.render_widget(hint, chunks[1]);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(v[1])[1]
}
