//! Mod install failure modal -- mirrors `loader_install_failed_modal.rs`.
//!
//! Source: 08-UI-SPEC.md §"Mod Install Failed Modal" lines 386-401.
//! Esc dispatches `Action::DismissModInstallFailed`; the update() arm uses
//! `return_to` to jump back to ModBrowser or InstalledModsList.

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_mod_install_failed_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let palette = &state.config.colors;
    let ActiveView::ModInstallFailedModal {
        slug: _,
        mod_title,
        version_label,
        error,
        log_tail,
        return_to: _,
    } = &state.active_view
    else {
        return;
    };

    // 80 × 20 cap (UI-SPEC line 387).
    let w = area.width.min(80);
    let h = area.height.min(20);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    f.render_widget(Clear, rect);
    let outer = crate::tui::theme::block(palette)
        .title(format!("Install failed: {mod_title}   (Esc to dismiss)"));
    f.render_widget(outer, rect);

    let inner = Rect {
        x: rect.x + 1,
        y: rect.y + 1,
        width: rect.width.saturating_sub(2),
        height: rect.height.saturating_sub(2),
    };
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(inner);

    // UI-SPEC line 674 / line 392: BOLD headline.
    let headline = format!("Failed to install {mod_title} {version_label}: {error}");
    let err_p = Paragraph::new(headline)
        .style(Style::default().add_modifier(Modifier::BOLD))
        .wrap(Wrap { trim: false });
    f.render_widget(err_p, split[0]);

    let tail_p = Paragraph::new(log_tail.as_str()).wrap(Wrap { trim: false });
    f.render_widget(tail_p, split[1]);
}

pub fn map_mod_install_failed_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::DismissModInstallFailed),
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
    fn esc_dismisses() {
        assert!(matches!(
            map_mod_install_failed_event(key(KeyCode::Esc)),
            Some(Action::DismissModInstallFailed)
        ));
    }

    #[test]
    fn other_keys_noop() {
        assert!(map_mod_install_failed_event(key(KeyCode::Char('q'))).is_none());
        assert!(map_mod_install_failed_event(key(KeyCode::Enter)).is_none());
    }
}
