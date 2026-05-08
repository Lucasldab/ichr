use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, LineGauge, Paragraph};
use ratatui::Frame;

use crate::tui::app::AppState;

pub fn render_download_pane(f: &mut Frame, area: Rect, state: &AppState) {
    if state.active_jobs.is_empty() {
        let idle = Paragraph::new(Span::raw("idle -- press c to create an instance"))
            .block(Block::default().borders(Borders::ALL).title("Downloads"));
        f.render_widget(idle, area);
        return;
    }

    let job_count = state.active_jobs.len();
    let inner_height = area.height.saturating_sub(2); // subtract block borders
    let row_height = if job_count == 0 {
        1u16
    } else {
        (inner_height / job_count as u16).max(1)
    };

    let constraints: Vec<Constraint> = state
        .active_jobs
        .iter()
        .map(|_| Constraint::Length(row_height))
        .collect();

    let block = Block::default().borders(Borders::ALL).title("Downloads");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (idx, (_, pct, msg)) in state.active_jobs.iter().enumerate() {
        if idx >= rows.len() {
            break;
        }
        let ratio = (*pct as f64) / 100.0;
        let gauge = LineGauge::default()
            .ratio(ratio)
            .label(Span::raw(format!("{msg} ({pct}%)")))
            .filled_style(Style::default().fg(Color::Green));
        f.render_widget(gauge, rows[idx]);
    }
}
