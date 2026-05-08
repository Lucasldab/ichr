//! Loader switch / remove inline confirm overlay (mirrors delete_confirm.rs).
//!
//! Shown when the user selects a loader/version that differs from the
//! currently-installed one. y/Y confirms; any other key cancels.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_loader_switch_confirm(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::LoaderSwitchConfirm {
        slug,
        from_loader,
        to_loader,
        type_switch,
    } = &state.active_view
    else {
        return;
    };

    let modal_area = centered_rect(60, 25, area);
    f.render_widget(Clear, modal_area);

    let mut lines: Vec<Line> = Vec::new();
    if *type_switch {
        lines.push(Line::from(Span::styled(
            "WARNING: switching loader type may break installed mods.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
    }

    let body = if to_loader == "none" {
        format!("Remove loader from {slug}?")
    } else {
        match from_loader.as_deref() {
            Some(prev) => format!("Switch {slug} from {prev} to {to_loader}?"),
            None => format!("Install {to_loader} on {slug}?"),
        }
    };
    lines.push(Line::from(body));
    lines.push(Line::from(Span::styled(
        "Mods are not affected — only the loader version changes.",
        Style::default().add_modifier(Modifier::DIM),
    )));
    lines.push(Line::from(Span::styled(
        "y to confirm  n/Esc to cancel",
        Style::default().add_modifier(Modifier::DIM),
    )));

    let title = if to_loader == "none" {
        " Remove loader? "
    } else {
        " Switch loader? "
    };
    let para = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(para, modal_area);
}

pub fn map_loader_switch_confirm_event(ev: ratatui::crossterm::event::Event) -> Option<Action> {
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('y'),
            ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('Y'),
            ..
        }) => Some(Action::ConfirmLoaderSwitch),
        CtEvent::Key(_) => Some(Action::CancelLoaderSwitch),
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
    fn test_y_or_upper_y_confirms() {
        assert!(matches!(
            map_loader_switch_confirm_event(key(KeyCode::Char('y'))),
            Some(Action::ConfirmLoaderSwitch)
        ));
        assert!(matches!(
            map_loader_switch_confirm_event(key(KeyCode::Char('Y'))),
            Some(Action::ConfirmLoaderSwitch)
        ));
    }

    #[test]
    fn test_n_cancels() {
        assert!(matches!(
            map_loader_switch_confirm_event(key(KeyCode::Char('n'))),
            Some(Action::CancelLoaderSwitch)
        ));
        assert!(matches!(
            map_loader_switch_confirm_event(key(KeyCode::Char('N'))),
            Some(Action::CancelLoaderSwitch)
        ));
    }

    #[test]
    fn test_esc_cancels() {
        assert!(matches!(
            map_loader_switch_confirm_event(key(KeyCode::Esc)),
            Some(Action::CancelLoaderSwitch)
        ));
    }

    #[test]
    fn test_anything_else_cancels() {
        // Any key event that isn't y/Y maps to CancelLoaderSwitch (defensive default).
        assert!(matches!(
            map_loader_switch_confirm_event(key(KeyCode::Char('q'))),
            Some(Action::CancelLoaderSwitch)
        ));
        assert!(matches!(
            map_loader_switch_confirm_event(key(KeyCode::Enter)),
            Some(Action::CancelLoaderSwitch)
        ));
    }
}
