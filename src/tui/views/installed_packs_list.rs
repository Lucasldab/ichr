//! Installed packs list -- full-screen table of resource or shader packs for an instance.
//!
//! Cloned-and-parameterised from `installed_mods_list.rs` per Phase 11 plan 04.
//!
//! D-LOCK:
//!  - Tab → InstalledPacksCycleKind (Mod→Resource→Shader→Mod cycle).
//!  - `e` on Resource row → TogglePackEnabled.
//!  - `e` on Shader row → ShaderToggleNotice (transient "cannot toggle" notice).
//!  - `x` on any kind → OpenUninstallPackConfirm.
//!  - State cell shows "n/a" for Shader packs (no enable/disable concept).

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use crate::mods::types::ModSource;
use crate::packs::kind::PackKind;
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_installed_packs_list(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::InstalledPacksList {
        slug,
        kind,
        packs,
        selected,
        transient_status,
    } = &state.active_view
    else {
        return;
    };

    let palette = &state.config.colors;
    let dim_style = Style::default()
        .fg(palette.dim.to_color())
        .add_modifier(Modifier::DIM);

    let kind_label = match kind {
        PackKind::Resource => "Resource Packs",
        PackKind::Shader => "Shader Packs",
    };

    // Reserve 1 line at bottom for transient status (or empty).
    let table_area = if transient_status.is_some() && area.height > 3 {
        Rect {
            height: area.height - 1,
            ..area
        }
    } else {
        area
    };
    let status_area = if transient_status.is_some() && area.height > 3 {
        Some(Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        })
    } else {
        None
    };

    if packs.is_empty() {
        let p = Paragraph::new("No packs installed -- press Esc and R/S to browse")
            .style(dim_style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("{kind_label} -- {slug}  (e/x/Tab/Esc)")),
            );
        f.render_widget(p, table_area);
    } else {
        let rows: Vec<Row> = packs
            .iter()
            .map(|m| {
                let (source_label, source_style) = match m.source {
                    ModSource::Modrinth => ("[M]", Style::default()),
                    ModSource::CurseForge => ("[CF]", Style::default()),
                    ModSource::Manual => ("manual", dim_style),
                    ModSource::Modpack => ("modpack", dim_style),
                    ModSource::Local => ("local", dim_style),
                };
                // Shader packs cannot be toggled -- state cell shows "n/a".
                let (state_label, state_style) = match kind {
                    PackKind::Shader => ("n/a", dim_style),
                    PackKind::Resource => {
                        if m.enabled {
                            ("enabled", Style::default())
                        } else {
                            ("disabled", dim_style)
                        }
                    }
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
        // Stateful render auto-scrolls so the selected pack stays visible
        // when the list exceeds the viewport.
        .row_highlight_style(Style::default().bg(palette.selected_bg.to_color()))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("{kind_label} -- {slug}  (e/x/Tab/Esc)")),
        );
        let mut ts = TableState::default().with_selected(Some(*selected));
        f.render_stateful_widget(table, table_area, &mut ts);
    }

    // Transient status line (below table).
    if let (Some(area), Some(msg)) = (status_area, transient_status) {
        let p = Paragraph::new(Line::from(msg.clone())).style(
            Style::default()
                .fg(palette.accent.to_color())
                .add_modifier(Modifier::DIM),
        );
        f.render_widget(p, area);
    }
}

pub fn map_installed_packs_list_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    let kind = match &state.active_view {
        ActiveView::InstalledPacksList { kind, .. } => *kind,
        _ => return None,
    };

    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Up, ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            ..
        }) => Some(Action::InstalledPacksMove(-1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Down,
            ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            ..
        }) => Some(Action::InstalledPacksMove(1)),
        // Tab cycles Mod→Resource→Shader→Mod.
        CtEvent::Key(KeyEvent {
            code: KeyCode::Tab, ..
        }) => Some(Action::InstalledPacksCycleKind),
        // `e` -- toggle for Resource; notice for Shader.
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('e'),
            ..
        }) => match kind {
            PackKind::Resource => Some(Action::TogglePackEnabled),
            PackKind::Shader => Some(Action::ShaderToggleNotice),
        },
        // `x` -- open uninstall confirm regardless of kind.
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('x'),
            ..
        }) => Some(Action::OpenUninstallPackConfirm),
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

    fn state_with_kind(kind: PackKind) -> AppState {
        AppState {
            active_view: ActiveView::InstalledPacksList {
                slug: "foo".into(),
                kind,
                packs: Vec::new(),
                selected: 0,
                transient_status: None,
            },
            ..AppState::default()
        }
    }

    #[test]
    fn jk_arrows_move() {
        let s = state_with_kind(PackKind::Resource);
        assert!(matches!(
            map_installed_packs_list_event(key(KeyCode::Char('j')), &s),
            Some(Action::InstalledPacksMove(1))
        ));
        assert!(matches!(
            map_installed_packs_list_event(key(KeyCode::Char('k')), &s),
            Some(Action::InstalledPacksMove(-1))
        ));
        assert!(matches!(
            map_installed_packs_list_event(key(KeyCode::Down), &s),
            Some(Action::InstalledPacksMove(1))
        ));
        assert!(matches!(
            map_installed_packs_list_event(key(KeyCode::Up), &s),
            Some(Action::InstalledPacksMove(-1))
        ));
    }

    #[test]
    fn tab_cycles_kind() {
        let s = state_with_kind(PackKind::Resource);
        assert!(matches!(
            map_installed_packs_list_event(key(KeyCode::Tab), &s),
            Some(Action::InstalledPacksCycleKind)
        ));
    }

    #[test]
    fn e_on_resource_dispatches_toggle() {
        let s = state_with_kind(PackKind::Resource);
        assert!(matches!(
            map_installed_packs_list_event(key(KeyCode::Char('e')), &s),
            Some(Action::TogglePackEnabled)
        ));
    }

    #[test]
    fn e_on_shader_dispatches_notice() {
        let s = state_with_kind(PackKind::Shader);
        assert!(matches!(
            map_installed_packs_list_event(key(KeyCode::Char('e')), &s),
            Some(Action::ShaderToggleNotice)
        ));
    }

    #[test]
    fn x_opens_confirm_for_any_kind() {
        for kind in [PackKind::Resource, PackKind::Shader] {
            let s = state_with_kind(kind);
            assert!(matches!(
                map_installed_packs_list_event(key(KeyCode::Char('x')), &s),
                Some(Action::OpenUninstallPackConfirm)
            ));
        }
    }

    #[test]
    fn esc_closes() {
        let s = state_with_kind(PackKind::Resource);
        assert!(matches!(
            map_installed_packs_list_event(key(KeyCode::Esc), &s),
            Some(Action::CloseInstalledMods)
        ));
    }
}
