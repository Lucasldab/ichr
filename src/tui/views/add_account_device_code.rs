//! AUTH-01 device-code modal. Displays user_code + verification_uri +
//! countdown + current stage. `Esc` cancels.

use std::time::Instant;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::app::{ActiveView, AppState};

pub fn render_add_account_device_code(state: &AppState, f: &mut Frame) {
    let (user_code, uri, expires_at, stage) = match &state.active_view {
        ActiveView::AddAccountDeviceCode {
            user_code,
            verification_uri,
            expires_at,
            stage,
        } => (user_code.clone(), verification_uri.clone(), *expires_at, stage.clone()),
        _ => return,
    };

    let area = centered_rect(60, 40, f.area());
    f.render_widget(Clear, area);

    let remaining = expires_at.saturating_duration_since(Instant::now()).as_secs();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(3),
    ])
    .split(area);

    let title = Paragraph::new(Line::from(Span::styled(
        "Add Microsoft Account",
        Style::default().add_modifier(Modifier::BOLD),
    )))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, chunks[0]);

    let code = Paragraph::new(Line::from(Span::styled(
        user_code.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )))
    .block(Block::default().borders(Borders::ALL).title(" Code "));
    f.render_widget(code, chunks[1]);

    let uri_p = Paragraph::new(Line::from(format!("Visit: {uri}")));
    f.render_widget(uri_p, chunks[2]);

    let count = Paragraph::new(Line::from(format!("Expires in: {remaining}s  |  {stage}")));
    f.render_widget(count, chunks[3]);

    let hint = Paragraph::new("Esc to cancel")
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(hint, chunks[4]);
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
