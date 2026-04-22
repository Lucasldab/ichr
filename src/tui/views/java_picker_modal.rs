//! Java picker modal — lets the user select a per-instance Java override.
//!
//! Rows:
//!   Auto        — clears java_override (delegate to resolver default)
//!   Detected(…) — a working system Java from scan_system_javas()
//!   Manual      — escape hatch: edit instance.json by hand

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState, JavaPickerRow};

/// Render the centered Java picker modal over whatever is beneath it.
pub fn render_java_picker_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let (slug, options, selected) = match &state.active_view {
        ActiveView::JavaPickerModal { slug, options, selected } => (slug, options, *selected),
        _ => return,
    };

    // Center a modal that is 60 wide × min(options+6, area_height) tall.
    let modal_w = area.width.min(70);
    let modal_h = (options.len() as u16 + 6).min(area.height.saturating_sub(4));
    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal_area = Rect { x, y, width: modal_w, height: modal_h };

    // Clear the background so content below doesn't bleed through.
    f.render_widget(ratatui::widgets::Clear, modal_area);

    // Outer block.
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Java Runtime — {slug} "));
    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    // Split inner: rows list + footer hint.
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let label = match row {
                JavaPickerRow::Auto => "Auto-resolve (Mojang → Adoptium fallback)".to_string(),
                JavaPickerRow::Detected(sj) => {
                    format!("System Java {}: {}", sj.major_version, sj.path.display())
                }
                JavaPickerRow::Manual => "Edit instance.json manually (escape hatch)".to_string(),
            };
            let style = if i == selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            ListItem::new(Line::from(label)).style(style)
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, chunks[0]);

    let hint = Paragraph::new("↑/k up  ↓/j down  Enter select  Esc cancel")
        .style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(hint, chunks[1]);
}

/// Translate a crossterm key event into Java-picker Actions.
pub fn map_java_picker_event(
    ev: ratatui::crossterm::event::Event,
) -> Option<Action> {
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Up, .. })
        | CtEvent::Key(KeyEvent { code: KeyCode::Char('k'), .. }) => {
            Some(Action::JavaPickerMove(-1))
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Down, .. })
        | CtEvent::Key(KeyEvent { code: KeyCode::Char('j'), .. }) => {
            Some(Action::JavaPickerMove(1))
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => Some(Action::JavaPickerSelect),
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::JavaPickerCancel),
        _ => None,
    }
}
