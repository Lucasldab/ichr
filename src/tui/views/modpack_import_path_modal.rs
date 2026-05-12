//! Modpack import path-entry modal -- single-line text entry for the `.mrpack` file path.
//!
//! Mirrors `create_modal.rs` inline-buffer pattern (Phase 2 CreateStep::NameInput).
//! Per CONTEXT.md Open Questions §3 (resolved): plain text-entry only; no
//! ratatui-textarea dependency, no file-browser widget.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_modpack_import_path_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::ModpackImportPathInput { buffer, error } = &state.active_view else {
        return;
    };

    let palette = &state.config.colors;
    let modal_area = centered_rect(60, 30, area);
    f.render_widget(Clear, modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(modal_area);

    let mut lines = vec![
        Line::from("Import .mrpack \u{2014} enter file path:"),
        Line::from(""),
        Line::from(Span::styled(
            format!("> {buffer}_"),
            Style::default().fg(palette.accent.to_color()),
        )),
    ];
    if let Some(err) = error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(palette.error.to_color()),
        )));
    }

    let para = Paragraph::new(lines).alignment(Alignment::Left).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Import Modpack (Enter to import / Esc to cancel)"),
    );
    f.render_widget(para, chunks[0]);
}

pub fn map_modpack_import_path_event(ev: ratatui::crossterm::event::Event) -> Option<Action> {
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::ImportPathCancel),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => Some(Action::ImportPathSubmit),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) => Some(Action::ImportPathBackspaceSearch),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::ImportPathTypeSearch(c)),
        CtEvent::Paste(s) => Some(Action::ImportPathPasteSearch(s)),
        _ => None,
    }
}

/// Return a Rect centered in `r` with the given percentage width/height.
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
