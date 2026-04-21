use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
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
            let last_col = if state.running_instances.contains_key(&m.slug) {
                Cell::from("running").style(Style::default().add_modifier(Modifier::BOLD))
            } else {
                Cell::from(m.last_played_at.clone().unwrap_or_default())
            };
            Row::new(vec![
                Cell::from(m.display_name.clone()),
                Cell::from(m.mc_version_id.clone()),
                Cell::from(m.group.clone().unwrap_or_default()),
                last_col,
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
    .block(Block::default().borders(Borders::ALL).title("Instances (c/r/x/d/g/Enter/s)"));
    f.render_widget(table, area);
}

pub fn render_group_inline_overlay(f: &mut Frame, area: Rect, state: &AppState) {
    if let ActiveView::GroupInline { slug, buffer, .. } = &state.active_view {
        let w = area.width.min(60);
        let h = 3u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect { x, y, width: w, height: h };
        let text = format!("Group for {slug}: {buffer}_   (Enter=save, empty=clear, Esc=cancel)");
        let p = Paragraph::new(text).block(
            Block::default().borders(Borders::ALL).title("Set group (g)"),
        );
        f.render_widget(ratatui::widgets::Clear, rect);
        f.render_widget(p, rect);
    }
}
