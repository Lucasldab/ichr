//! Installed mods list -- full-screen 4-column table of mods in an instance.
//!
//! Source: 08-UI-SPEC.md §"Installed Mods List" lines 326-365.
//! Mirrors `instance_list.rs` (Table widget + REVERSED selection +
//! block title with keybind hints).

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use crate::mods::types::ModSource;
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_installed_mods_list(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::InstalledModsList {
        slug,
        mods,
        selected,
    } = &state.active_view
    else {
        return;
    };

    if mods.is_empty() {
        // UI-SPEC line 365 -- empty-state copy, DIM, single line.
        let p = Paragraph::new("No mods installed -- press Esc and M to browse")
            .style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Installed Mods -- {slug}  (e/x/Esc)")),
            );
        f.render_widget(p, area);
        return;
    }

    let rows: Vec<Row> = mods
        .iter()
        .map(|m| {
            // Source cell -- short tags for body sources (Phase 9 09-07
            // Pitfall 10 visual disambiguator: `[M]` Modrinth, `[CF]` CurseForge);
            // DIM long-form for the rare manual/modpack paths.
            let (source_label, source_style) = match m.source {
                ModSource::Modrinth => ("[M]", Style::default()),
                ModSource::CurseForge => ("[CF]", Style::default()),
                ModSource::Manual => (
                    "manual",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                ),
                ModSource::Modpack => (
                    "modpack",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                ),
                ModSource::Local => (
                    "local",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                ),
            };
            // State cell -- body for enabled, DIM for disabled.
            let (state_label, state_style) = if m.enabled {
                ("enabled", Style::default())
            } else {
                (
                    "disabled",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                )
            };

            Row::new(vec![
                Cell::from(m.display_name.clone()),
                Cell::from(m.version_label.clone()),
                Cell::from(source_label).style(source_style),
                Cell::from(state_label).style(state_style),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(40),
            Constraint::Percentage(25),
            Constraint::Percentage(15),
            Constraint::Percentage(20),
        ],
    )
    .header(Row::new(vec!["Name", "Version", "Source", "State"]))
    // REVERSED across the entire row when selected (UI-SPEC line 362).
    // Stateful render keeps the selected row in view even when the list
    // exceeds the viewport.
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Installed Mods -- {slug}  (e/x/Esc)")),
    );
    let mut ts = TableState::default().with_selected(Some(*selected));
    f.render_stateful_widget(table, area, &mut ts);
}

pub fn map_installed_mods_list_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Up, ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            ..
        }) => Some(Action::InstalledModsMove(-1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Down,
            ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            ..
        }) => Some(Action::InstalledModsMove(1)),
        // Phase 11 D-LOCK Tab switcher: Tab from InstalledMods cycles to Resource.
        CtEvent::Key(KeyEvent {
            code: KeyCode::Tab, ..
        }) => Some(Action::InstalledPacksCycleKind),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('e'),
            ..
        }) => Some(Action::ToggleModEnabled),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('x'),
            ..
        }) => Some(Action::OpenUninstallModConfirm),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CloseInstalledMods),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> CtEvent {
        CtEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn jk_arrows_move() {
        assert!(matches!(
            map_installed_mods_list_event(key(KeyCode::Char('j'))),
            Some(Action::InstalledModsMove(1))
        ));
        assert!(matches!(
            map_installed_mods_list_event(key(KeyCode::Char('k'))),
            Some(Action::InstalledModsMove(-1))
        ));
        assert!(matches!(
            map_installed_mods_list_event(key(KeyCode::Up)),
            Some(Action::InstalledModsMove(-1))
        ));
        assert!(matches!(
            map_installed_mods_list_event(key(KeyCode::Down)),
            Some(Action::InstalledModsMove(1))
        ));
    }

    #[test]
    fn e_toggles_x_uninstalls_esc_closes() {
        assert!(matches!(
            map_installed_mods_list_event(key(KeyCode::Char('e'))),
            Some(Action::ToggleModEnabled)
        ));
        assert!(matches!(
            map_installed_mods_list_event(key(KeyCode::Char('x'))),
            Some(Action::OpenUninstallModConfirm)
        ));
        assert!(matches!(
            map_installed_mods_list_event(key(KeyCode::Esc)),
            Some(Action::CloseInstalledMods)
        ));
    }
}
