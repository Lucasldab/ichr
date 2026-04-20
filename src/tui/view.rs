//! Render functions. Pure: take `&AppState`, write into a `Frame`.

use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::app::AppState;

pub fn view(state: &AppState, f: &mut Frame) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let header = Paragraph::new(Span::styled(
        format!("mineltui v{}  —  scaffold", env!("CARGO_PKG_VERSION")),
        Style::default().add_modifier(Modifier::BOLD),
    ))
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    let arch = state.arch.map(|a| a.mojang_str().to_string()).unwrap_or_else(|| "?".into());
    let os = state.os.map(|o| o.mojang_str().to_string()).unwrap_or_else(|| "?".into());

    let body_lines = vec![
        Line::from(format!("platform: {os} / {arch}")),
        Line::from(format!("active jobs: {}", state.active_jobs.len())),
        Line::from(""),
        Line::from(Span::styled(
            "press q to quit",
            Style::default().add_modifier(Modifier::DIM),
        )),
    ];
    let body = Paragraph::new(body_lines)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(body, chunks[1]);

    let footer = Paragraph::new(Span::raw("mineltui — phase 1 scaffold"))
        .alignment(Alignment::Center);
    f.render_widget(footer, chunks[2]);
}
