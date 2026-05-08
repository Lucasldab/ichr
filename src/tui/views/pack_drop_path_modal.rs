//! Pack drop-from-path modal -- text-entry for a local .zip file path.
//!
//! Cloned-and-parameterised from `modpack_import_path_modal.rs` per Phase 11 plan 04.
//! Title adapts by PackKind:
//!  - Resource: "Add Resource Pack (Enter / Esc)"
//!  - Shader:   "Add Shader Pack (Enter / Esc)"

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::packs::kind::PackKind;
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_pack_drop_path_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::PackDropPathInput {
        kind,
        buffer,
        error,
        ..
    } = &state.active_view
    else {
        return;
    };

    let modal_area = centered_rect(60, 30, area);
    f.render_widget(Clear, modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(modal_area);

    let prompt = match kind {
        PackKind::Resource => "Install resource pack -- enter .zip file path:",
        PackKind::Shader => "Install shader pack -- enter .zip file path:",
    };
    let title = match kind {
        PackKind::Resource => "Add Resource Pack (Enter to install / Esc to cancel)",
        PackKind::Shader => "Add Shader Pack (Enter to install / Esc to cancel)",
    };

    let mut lines = vec![
        Line::from(prompt),
        Line::from(""),
        Line::from(Span::styled(
            format!("> {buffer}_"),
            Style::default().fg(Color::Yellow),
        )),
    ];
    if let Some(err) = error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(Color::Red),
        )));
    }

    let para = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(para, chunks[0]);
}

pub fn map_pack_drop_path_event(ev: ratatui::crossterm::event::Event) -> Option<Action> {
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::PackDropPathCancel),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => Some(Action::PackDropPathSubmit),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) => Some(Action::PackDropPathBackspace),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::PackDropPathType(c)),
        CtEvent::Paste(s) => Some(Action::PackDropPathPaste(s)),
        _ => None,
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> CtEvent {
        CtEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn esc_cancels() {
        assert!(matches!(
            map_pack_drop_path_event(key(KeyCode::Esc)),
            Some(Action::PackDropPathCancel)
        ));
    }

    #[test]
    fn enter_submits() {
        assert!(matches!(
            map_pack_drop_path_event(key(KeyCode::Enter)),
            Some(Action::PackDropPathSubmit)
        ));
    }

    #[test]
    fn backspace_pops() {
        assert!(matches!(
            map_pack_drop_path_event(key(KeyCode::Backspace)),
            Some(Action::PackDropPathBackspace)
        ));
    }

    #[test]
    fn char_types() {
        assert!(matches!(
            map_pack_drop_path_event(key(KeyCode::Char('a'))),
            Some(Action::PackDropPathType('a'))
        ));
    }

    #[test]
    fn paste_event() {
        let result = map_pack_drop_path_event(CtEvent::Paste("hi".into()));
        assert!(matches!(result, Some(Action::PackDropPathPaste(s)) if s == "hi"));
    }
}
