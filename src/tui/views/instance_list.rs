use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use crate::tui::app::{ActiveView, AppState};

pub fn render_instance_list(f: &mut Frame, area: Rect, state: &AppState) {
    // Reserve the last row for the active-account footer.
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);

    let table_area = chunks[0];
    let footer_area = chunks[1];

    let palette = &state.config.colors;
    let selected = match &state.active_view {
        ActiveView::InstanceList { selected } => Some(*selected),
        _ => None,
    };
    let rows: Vec<Row> = state
        .instances
        .iter()
        .map(|m| {
            let last_col = if state.running_instances.contains_key(&m.slug) {
                Cell::from("running").style(
                    Style::default()
                        .fg(palette.accent.to_color())
                        .add_modifier(Modifier::BOLD),
                )
            } else if let Some(loader) = &m.loader {
                let kind = match loader.kind {
                    crate::domain::instance::ModloaderKind::Fabric => "fabric",
                    crate::domain::instance::ModloaderKind::Quilt => "quilt",
                    crate::domain::instance::ModloaderKind::Forge => "forge",
                    crate::domain::instance::ModloaderKind::NeoForge => "neoforge",
                    crate::domain::instance::ModloaderKind::Vanilla => "vanilla",
                };
                let n = loader.version.len().min(6);
                Cell::from(format!("{kind}:{}", &loader.version[..n]))
            } else {
                Cell::from(m.last_played_at.clone().unwrap_or_default())
            };
            Row::new(vec![
                Cell::from(m.display_name.clone()),
                Cell::from(m.mc_version_id.clone()),
                Cell::from(m.group.clone().unwrap_or_default()),
                last_col,
            ])
        })
        .collect();
    // Phase 9 (09-07): F keybind opens the CurseForge browser. Title shows the
    // disabled hint `(no API key)` when no CurseForge API key was resolved at
    // startup -- matches the Phase 6 `L (running)` DIM disabled-feature pattern.
    let title = if state.cf_api_key_present {
        "Instances (c/r/x/d/g/Enter/s/A/L/M/m/F)"
    } else {
        "Instances (c/r/x/d/g/Enter/s/A/L/M/m/F (no API key))"
    };
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(40),
            Constraint::Percentage(15),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
        ],
    )
    .header(Row::new(vec!["Name", "MC Version", "Group", "Last played"]))
    // Stateful render keeps the selected instance in view when the list
    // exceeds the terminal height.
    .row_highlight_style(Style::default().bg(palette.selected_bg.to_color()))
    .block(crate::tui::theme::block(palette).title(title));
    let mut ts = TableState::default().with_selected(selected);
    f.render_stateful_widget(table, table_area, &mut ts);

    // Active-account footer row. Hint reads the live keybind label so a
    // user-rebound `open_accounts_list` propagates into the footer text
    // (no more lying "press A" when they've remapped to Ctrl+A or
    // similar).
    let accounts_label = state
        .config
        .keybinds
        .label(crate::config::ActionKey::OpenAccountsList);
    let footer_text = match state
        .active_account_id
        .as_ref()
        .and_then(|id| state.accounts.iter().find(|a| &a.id == id))
    {
        Some(a) => format!(
            "Launching as: {}  (press {accounts_label} to manage accounts)",
            a.mc_username
        ),
        None => format!("Offline mode -- press {accounts_label} to add a Microsoft account"),
    };
    let footer = Paragraph::new(footer_text).style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(footer, footer_area);
}

pub fn render_group_inline_overlay(f: &mut Frame, area: Rect, state: &AppState) {
    let palette = &state.config.colors;
    if let ActiveView::GroupInline { slug, buffer, .. } = &state.active_view {
        let w = area.width.min(60);
        let h = 3u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect {
            x,
            y,
            width: w,
            height: h,
        };
        let text = format!("Group for {slug}: {buffer}_   (Enter=save, empty=clear, Esc=cancel)");
        let p = Paragraph::new(text).block(
            crate::tui::theme::block(palette)
                .title("Set group (g)"),
        );
        f.render_widget(ratatui::widgets::Clear, rect);
        f.render_widget(p, rect);
    }
}
