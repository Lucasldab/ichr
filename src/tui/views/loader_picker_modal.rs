//! Loader picker modal — None / Fabric / Quilt selector.
//!
//! Mirrors `java_picker_modal.rs`. Three rows; footer hint; REVERSED for
//! selected, DIM for others.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState};

const ROW_LABELS: [&str; 5] = [
    "None (vanilla — remove installed loader)",
    "Fabric Loader ▶",
    "Quilt Loader ▶",
    "Forge ▶",
    "NeoForge ▶",
];

pub fn render_loader_picker_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let (slug, selected) = match &state.active_view {
        ActiveView::LoaderPickerModal { slug, selected } => (slug, *selected),
        _ => return,
    };

    let modal_w = area.width.min(70);
    let modal_h = 11u16.min(area.height.saturating_sub(4));
    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal_area = Rect { x, y, width: modal_w, height: modal_h };

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Install Loader — {slug} "));
    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

    let items: Vec<ListItem> = ROW_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let style = if i == selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            ListItem::new(Line::from(*label)).style(style)
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, chunks[0]);

    let hint = Paragraph::new("↑/k up  ↓/j down  Enter select  Esc cancel")
        .style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(hint, chunks[1]);
}

pub fn map_loader_picker_event(ev: ratatui::crossterm::event::Event) -> Option<Action> {
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Up, .. })
        | CtEvent::Key(KeyEvent { code: KeyCode::Char('k'), .. }) => {
            Some(Action::LoaderPickerMove(-1))
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Down, .. })
        | CtEvent::Key(KeyEvent { code: KeyCode::Char('j'), .. }) => {
            Some(Action::LoaderPickerMove(1))
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => Some(Action::LoaderPickerSelect),
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::LoaderPickerCancel),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> CtEvent {
        CtEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn test_y_is_unbound_in_picker() {
        // y/Y are NOT bound in the picker (UI-SPEC §Keybind Contract).
        assert!(map_loader_picker_event(key(KeyCode::Char('y'))).is_none());
        assert!(map_loader_picker_event(key(KeyCode::Char('Y'))).is_none());
    }

    #[test]
    fn test_up_down_kj_move() {
        assert!(matches!(
            map_loader_picker_event(key(KeyCode::Up)),
            Some(Action::LoaderPickerMove(-1))
        ));
        assert!(matches!(
            map_loader_picker_event(key(KeyCode::Char('k'))),
            Some(Action::LoaderPickerMove(-1))
        ));
        assert!(matches!(
            map_loader_picker_event(key(KeyCode::Down)),
            Some(Action::LoaderPickerMove(1))
        ));
        assert!(matches!(
            map_loader_picker_event(key(KeyCode::Char('j'))),
            Some(Action::LoaderPickerMove(1))
        ));
    }

    #[test]
    fn test_enter_selects() {
        assert!(matches!(
            map_loader_picker_event(key(KeyCode::Enter)),
            Some(Action::LoaderPickerSelect)
        ));
    }

    #[test]
    fn test_esc_cancels() {
        assert!(matches!(
            map_loader_picker_event(key(KeyCode::Esc)),
            Some(Action::LoaderPickerCancel)
        ));
    }

    #[test]
    fn test_arbitrary_char_is_noop() {
        // Any char outside k/j (and outside Enter/Esc/arrows) is a no-op.
        assert!(map_loader_picker_event(key(KeyCode::Char('q'))).is_none());
        assert!(map_loader_picker_event(key(KeyCode::Char('1'))).is_none());
    }
}
