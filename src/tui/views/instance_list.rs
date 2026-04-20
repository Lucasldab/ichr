use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::tui::app::{ActiveView, AppState};

pub fn render_instance_list(f: &mut Frame, area: Rect, state: &AppState) {
    let selected = match &state.active_view {
        ActiveView::InstanceList { selected } => Some(*selected),
        _ => None,
    };
    let rows: Vec<Row> = state
        .instances
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let style = if Some(i) == selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(m.display_name.clone()),
                Cell::from(m.mc_version_id.clone()),
                Cell::from(m.group.clone().unwrap_or_default()),
                Cell::from(m.last_played_at.clone().unwrap_or_default()),
            ])
            .style(style)
        })
        .collect();
    let table = Table::new(
        rows,
        [
            ratatui::layout::Constraint::Percentage(40),
            ratatui::layout::Constraint::Percentage(15),
            ratatui::layout::Constraint::Percentage(20),
            ratatui::layout::Constraint::Percentage(25),
        ],
    )
    .header(Row::new(vec!["Name", "MC Version", "Group", "Last played"]))
    .block(Block::default().borders(Borders::ALL).title("Instances (c/r/x/d)"));
    f.render_widget(table, area);
}
