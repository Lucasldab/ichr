//! Uninstall mod confirm — inline overlay.
//!
//! Source: 08-UI-SPEC.md §"Uninstall Confirm" lines 404-419.
//! Mirrors `delete_confirm.rs` BUT drops the Red text per UI-SPEC line 419
//! ("uninstalling a mod is reversible — Red is reserved for type-switch
//! warnings and incompatible-dep warnings only").

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_uninstall_mod_confirm(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::UninstallModConfirm { slug, mod_id: _, display_name } = &state.active_view
    else {
        return;
    };

    // Modal dimensions per UI-SPEC line 416: min(70, area.width) × 5.
    let w = area.width.min(70);
    let h = 5u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let modal_area = Rect { x, y, width: w, height: h };

    f.render_widget(Clear, modal_area);

    // Body lines per UI-SPEC lines 411-413. NO Red anywhere — body style only.
    let lines = vec![
        Line::from(format!("Uninstall {display_name} from {slug}?")),
        Line::from("The mod file will be deleted from .minecraft/mods/."),
        Line::from(ratatui::text::Span::styled(
            "y to confirm  n/Esc to cancel",
            Style::default().add_modifier(Modifier::DIM),
        )),
    ];

    let para = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Uninstall mod?"));
    f.render_widget(para, modal_area);
}

pub fn map_uninstall_mod_confirm_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Char('y'), .. })
        | CtEvent::Key(KeyEvent { code: KeyCode::Char('Y'), .. }) => {
            Some(Action::ConfirmUninstallMod)
        }
        CtEvent::Key(_) => Some(Action::CancelUninstallMod),
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
    fn y_confirms() {
        assert!(matches!(
            map_uninstall_mod_confirm_event(key(KeyCode::Char('y'))),
            Some(Action::ConfirmUninstallMod)
        ));
        assert!(matches!(
            map_uninstall_mod_confirm_event(key(KeyCode::Char('Y'))),
            Some(Action::ConfirmUninstallMod)
        ));
    }

    #[test]
    fn esc_n_cancel() {
        assert!(matches!(
            map_uninstall_mod_confirm_event(key(KeyCode::Esc)),
            Some(Action::CancelUninstallMod)
        ));
        assert!(matches!(
            map_uninstall_mod_confirm_event(key(KeyCode::Char('n'))),
            Some(Action::CancelUninstallMod)
        ));
    }
}
