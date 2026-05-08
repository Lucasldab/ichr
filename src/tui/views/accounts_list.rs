//! AUTH-06 accounts list view. Renders state.accounts as a table with
//! the active account marked. Empty state prompts for first add.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;

use crate::tui::app::{ActiveView, AppState};

pub fn render_accounts_list(f: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // header
        Constraint::Min(5),    // table
        Constraint::Length(3), // keybind hint
    ])
    .split(area);

    let header = Paragraph::new("Microsoft Accounts")
        .block(Block::default().borders(Borders::ALL).title(" Accounts "));
    f.render_widget(header, chunks[0]);

    let selected = match &state.active_view {
        ActiveView::AccountsList { selected } => *selected,
        _ => 0,
    };

    if state.accounts.is_empty() {
        let empty = Paragraph::new("No accounts — press `a` to add a Microsoft account.")
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(empty, chunks[1]);
    } else {
        let rows: Vec<Row> = state
            .accounts
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let marker = if a.is_active { "▶" } else { " " };
                let style = if i == selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                let storage_label = match a.storage {
                    crate::auth::StorageBackend::Keyring => "Keyring",
                    crate::auth::StorageBackend::EncryptedFile => "File",
                };
                Row::new(vec![
                    marker.to_string(),
                    a.mc_username.clone(),
                    short_uuid(&a.mc_uuid),
                    storage_label.to_string(),
                ])
                .style(style)
            })
            .collect();
        let widths = [
            Constraint::Length(3),
            Constraint::Length(20),
            Constraint::Length(14),
            Constraint::Length(10),
        ];
        let table = Table::new(rows, widths)
            .header(Row::new(["", "Username", "UUID", "Storage"]))
            .block(Block::default().borders(Borders::ALL).title(" Accounts "));
        f.render_widget(table, chunks[1]);
    }

    let hint = Paragraph::new(Line::from("a add  x remove  Enter activate  Esc close"))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(hint, chunks[2]);
}

/// "c6bf8193-0000-..." -> "c6bf8193..." (8 chars + …).
fn short_uuid(uuid: &str) -> String {
    if uuid.len() > 8 {
        format!("{}...", &uuid[..8])
    } else {
        uuid.to_string()
    }
}
