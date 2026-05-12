//! Mod version picker modal -- centered list of available versions for one mod.
//!
//! Source: 08-UI-SPEC.md §"Mod Detail Action: Version Picker" lines 253-275.
//! Mirrors `loader_version_picker_modal.rs` (analog), minus the
//! filter_stable_only toggle and the in-modal search input -- UI-SPEC version
//! picker is a static modal with no in-modal filtering.

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_mod_version_picker_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::ModVersionPickerModal {
        slug: _,
        project_id: _,
        project_title,
        versions,
        selected,
    } = &state.active_view
    else {
        return;
    };

    let palette = &state.config.colors;
    let dim_style = Style::default()
        .fg(palette.dim.to_color())
        .add_modifier(Modifier::DIM);
    let modal_w = area.width.min(70);
    let modal_h = (area.height.saturating_sub(4)).min(20);
    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal_area = Rect {
        x,
        y,
        width: modal_w,
        height: modal_h,
    };

    f.render_widget(Clear, modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(modal_area);

    // ---- Version list ----
    let items: Vec<ListItem> = if versions.is_empty() {
        // UI-SPEC line 651 -- empty/no-compatible-versions copy.
        vec![
            ListItem::new("No versions match MC + loader -- press Esc and adjust filters")
                .style(dim_style),
        ]
    } else {
        versions
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let label = format!("{} ({})", v.version_label, v.channel);
                let mut spans = vec![Span::raw(label)];
                if v.is_latest_stable {
                    // UI-SPEC line 650: "← latest" suffix on first stable row, DIM.
                    spans.push(Span::styled("   ← latest".to_string(), dim_style));
                }
                let style = if i == *selected {
                    Style::default().bg(palette.selected_bg.to_color())
                } else {
                    Style::default().add_modifier(Modifier::DIM)
                };
                ListItem::new(Line::from(spans)).style(style)
            })
            .collect()
    };

    let list = List::new(items).block(
        crate::tui::theme::block(palette)
            .title(format!("{project_title} -- versions")),
    );
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(*selected));
    f.render_stateful_widget(list, chunks[0], &mut list_state);

    // ---- Footer hint (DIM) ----
    let hint = Paragraph::new("↑/k ↓/j  Enter resolve deps  Esc cancel")
        .style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(hint, chunks[1]);
}

pub fn map_mod_version_picker_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Up, ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            ..
        }) => Some(Action::ModVersionPickerMove(-1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Down,
            ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            ..
        }) => Some(Action::ModVersionPickerMove(1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => Some(Action::ModVersionPickerSelect),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::ModVersionPickerCancel),
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
    fn up_down_kj_move() {
        assert!(matches!(
            map_mod_version_picker_event(key(KeyCode::Up)),
            Some(Action::ModVersionPickerMove(-1))
        ));
        assert!(matches!(
            map_mod_version_picker_event(key(KeyCode::Char('k'))),
            Some(Action::ModVersionPickerMove(-1))
        ));
        assert!(matches!(
            map_mod_version_picker_event(key(KeyCode::Down)),
            Some(Action::ModVersionPickerMove(1))
        ));
        assert!(matches!(
            map_mod_version_picker_event(key(KeyCode::Char('j'))),
            Some(Action::ModVersionPickerMove(1))
        ));
    }

    #[test]
    fn enter_selects_esc_cancels() {
        assert!(matches!(
            map_mod_version_picker_event(key(KeyCode::Enter)),
            Some(Action::ModVersionPickerSelect)
        ));
        assert!(matches!(
            map_mod_version_picker_event(key(KeyCode::Esc)),
            Some(Action::ModVersionPickerCancel)
        ));
    }
}
