//! CurseForge file picker modal — centered list of available files for one mod.
//!
//! Source: 09-RESEARCH.md §"TUI Integration Plumbing" + 09-PATTERNS.md
//! §`src/tui/views/cf_file_picker_modal.rs` (deltas off `mod_version_picker_modal.rs`).
//!
//! Differences from Phase 8 ModVersionPickerModal:
//!  - Wire types are CurseForge (`CurseForgeFileEntry`).
//!  - Row label uses `release_type` integer enum (1=release, 2=beta, 3=alpha)
//!    rather than Modrinth's `channel` string.
//!  - "← latest" suffix derived in-view: first row whose `release_type == 1`.
//!  - Footer hint omits dep-resolution language (Phase 9 v1 has no auto-deps).

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_cf_file_picker_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::CfFilePickerModal {
        slug: _,
        mod_detail,
        files,
        selected,
    } = &state.active_view
    else {
        return;
    };

    let modal_w = area.width.min(70);
    let modal_h = (area.height.saturating_sub(4)).min(20);
    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal_area = Rect { x, y, width: modal_w, height: modal_h };

    f.render_widget(Clear, modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(modal_area);

    // Find the first release-type-1 row to mark it `← latest`.
    let first_release_idx = files.iter().position(|f| f.release_type == 1);

    // ---- File list ----
    let items: Vec<ListItem> = if files.is_empty() {
        vec![ListItem::new(
            "No files match MC + loader — press Esc and adjust filters",
        )
        .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM))]
    } else {
        files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let label = format!("{} ({})", f.display_name, release_type_str(f.release_type));
                let mut spans = vec![Span::raw(label)];
                if Some(i) == first_release_idx {
                    spans.push(Span::styled(
                        "   ← latest".to_string(),
                        Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                    ));
                }
                let style = if i == *selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default().add_modifier(Modifier::DIM)
                };
                ListItem::new(Line::from(spans)).style(style)
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("{} — files", mod_detail.name)),
    );
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(*selected));
    f.render_stateful_widget(list, chunks[0], &mut list_state);

    // ---- Footer hint (DIM) ----
    let hint = Paragraph::new("↑/k ↓/j  Enter install  Esc cancel")
        .style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(hint, chunks[1]);
}

fn release_type_str(rt: i32) -> &'static str {
    match rt {
        1 => "release",
        2 => "beta",
        3 => "alpha",
        _ => "unknown",
    }
}

pub fn map_cf_file_picker_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Up, .. })
        | CtEvent::Key(KeyEvent { code: KeyCode::Char('k'), .. }) => {
            Some(Action::CfFilePickerMove(-1))
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Down, .. })
        | CtEvent::Key(KeyEvent { code: KeyCode::Char('j'), .. }) => {
            Some(Action::CfFilePickerMove(1))
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => Some(Action::CfFilePickerConfirm),
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::CloseModal),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::curseforge::types::{
        CurseForgeAuthor, CurseForgeFileEntry, CurseForgeLinks, CurseForgeProjectDetail,
    };
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> CtEvent {
        CtEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn buffer_to_string(buf: &ratatui::prelude::Buffer) -> String {
        let area = buf.area();
        let mut s = String::with_capacity((area.width as usize + 1) * area.height as usize);
        for y in 0..area.height {
            for x in 0..area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    fn fx_detail() -> CurseForgeProjectDetail {
        CurseForgeProjectDetail {
            id: 1,
            slug: "x".into(),
            name: "Sodium".into(),
            summary: String::new(),
            description: String::new(),
            download_count: 0,
            authors: vec![CurseForgeAuthor {
                id: 1,
                name: "JellySquid".into(),
                url: String::new(),
            }],
            links: CurseForgeLinks::default(),
        }
    }

    fn fx_file(id: u64, name: &str, rt: i32) -> CurseForgeFileEntry {
        CurseForgeFileEntry {
            id,
            display_name: name.into(),
            file_name: format!("{name}.jar"),
            release_type: rt,
            file_status: 4,
            hashes: vec![],
            file_date: String::new(),
            file_length: 0,
            download_count: 0,
            download_url: None,
            game_versions: vec![],
            dependencies: vec![],
            is_available: true,
        }
    }

    #[test]
    fn up_down_kj_move() {
        assert!(matches!(
            map_cf_file_picker_event(key(KeyCode::Up)),
            Some(Action::CfFilePickerMove(-1))
        ));
        assert!(matches!(
            map_cf_file_picker_event(key(KeyCode::Char('k'))),
            Some(Action::CfFilePickerMove(-1))
        ));
        assert!(matches!(
            map_cf_file_picker_event(key(KeyCode::Down)),
            Some(Action::CfFilePickerMove(1))
        ));
        assert!(matches!(
            map_cf_file_picker_event(key(KeyCode::Char('j'))),
            Some(Action::CfFilePickerMove(1))
        ));
    }

    #[test]
    fn enter_confirms() {
        assert!(matches!(
            map_cf_file_picker_event(key(KeyCode::Enter)),
            Some(Action::CfFilePickerConfirm)
        ));
    }

    #[test]
    fn esc_closes_modal() {
        // Esc returns CloseModal — the update() arm then returns to InstanceList.
        assert!(matches!(
            map_cf_file_picker_event(key(KeyCode::Esc)),
            Some(Action::CloseModal)
        ));
    }

    #[test]
    fn release_type_labels() {
        assert_eq!(release_type_str(1), "release");
        assert_eq!(release_type_str(2), "beta");
        assert_eq!(release_type_str(3), "alpha");
        assert_eq!(release_type_str(99), "unknown");
    }

    #[test]
    fn renders_file_labels() {
        // Smoke-test: render against a TestBackend buffer and assert the file
        // labels (and `← latest` marker) appear.
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(80, 20);
        let mut term = Terminal::new(backend).unwrap();
        let state = AppState {
            active_view: ActiveView::CfFilePickerModal {
                slug: "inst".into(),
                mod_detail: fx_detail(),
                files: vec![
                    fx_file(1, "Sodium 0.5.8", 1),
                    fx_file(2, "Sodium 0.5.7-beta", 2),
                ],
                selected: 0,
            },
            ..AppState::default()
        };
        term.draw(|f| {
            let area = f.area();
            render_cf_file_picker_modal(f, area, &state);
        })
        .unwrap();
        let text = buffer_to_string(term.backend().buffer());
        assert!(text.contains("Sodium 0.5.8"), "file 1 label missing:\n{text}");
        assert!(
            text.contains("(release)"),
            "release label missing:\n{text}"
        );
        assert!(text.contains("(beta)"), "beta label missing:\n{text}");
        assert!(text.contains("← latest"), "latest marker missing:\n{text}");
        assert!(
            text.contains("Sodium — files"),
            "mod-name title missing:\n{text}"
        );
    }
}
