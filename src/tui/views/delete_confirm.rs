use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::app::{ActiveView, AppState};

pub fn render_delete_confirm(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::DeleteConfirm { display_name, .. } = &state.active_view else {
        return;
    };

    let palette = &state.config.colors;
    let modal_area = centered_rect(50, 20, area);
    f.render_widget(Clear, modal_area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("Delete \"{display_name}\"?"),
            Style::default().fg(palette.error.to_color()),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press y to confirm, any other key to cancel.",
            Style::default().fg(palette.dim.to_color()),
        )),
    ];

    let para = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Confirm Delete (y / N)"),
    );
    f.render_widget(para, modal_area);
}

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
