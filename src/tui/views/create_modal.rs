use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::app::{ActiveView, AppState, CreateStep};

pub fn render_create_modal(f: &mut Frame, area: Rect, state: &AppState) {
    // Center a modal box.
    let palette = &state.config.colors;
    let modal_area = centered_rect(60, 30, area);
    f.render_widget(Clear, modal_area);

    match &state.active_view {
        ActiveView::CreateModal(CreateStep::NameInput { current, error }) => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(modal_area);

            let mut lines = vec![
                Line::from("Enter a name for the new instance:"),
                Line::from(""),
                Line::from(Span::styled(
                    format!("> {current}_"),
                    Style::default().fg(palette.accent.to_color()),
                )),
            ];
            if let Some(err) = error {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    err.clone(),
                    Style::default().fg(palette.error.to_color()),
                )));
            }

            let para = Paragraph::new(lines).alignment(Alignment::Left).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Create Instance (Enter / Esc)"),
            );
            f.render_widget(para, chunks[0]);
        }
        ActiveView::RenameInline { current, .. } => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(modal_area);

            let lines = vec![
                Line::from("Rename instance:"),
                Line::from(""),
                Line::from(Span::styled(
                    format!("> {current}_"),
                    Style::default().fg(palette.accent.to_color()),
                )),
            ];

            let para = Paragraph::new(lines).alignment(Alignment::Left).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Rename Instance (Enter / Esc)"),
            );
            f.render_widget(para, chunks[0]);
        }
        _ => {}
    }
}

/// Return a Rect centered in `r` with the given percentage width/height.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
