//! Pack install failure modal -- mirrors `mod_install_failed_modal.rs`.
//!
//! Phase 11 follow-up: previously `Action::PackInstallFailed` only emitted
//! `tracing::warn!` and dropped the user back into the pack browser with
//! no surface indication of why the install bailed (first hit: a Modrinth
//! filename containing Minecraft formatting codes that the strict
//! allowlist refused). This modal makes failures visible.

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_pack_install_failed_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::PackInstallFailedModal {
        slug: _,
        kind,
        pack_title,
        version_label,
        error,
        return_to: _,
    } = &state.active_view
    else {
        return;
    };

    let kind_label = match kind {
        crate::packs::kind::PackKind::Resource => "resource pack",
        crate::packs::kind::PackKind::Shader => "shader pack",
    };

    // 80 × 20 cap (mirrors mod modal's UI-SPEC §"Mod Install Failed Modal").
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
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(format!("Install failed: {pack_title}   (Esc to dismiss)"));
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

    let version_part = if version_label.is_empty() {
        String::new()
    } else {
        format!(" {version_label}")
    };
    let headline = format!(
        "Failed to install {kind_label} {pack_title}{version_part}: {error}"
    );
    let err_p = Paragraph::new(headline)
        .style(Style::default().add_modifier(Modifier::BOLD))
        .wrap(Wrap { trim: false });
    f.render_widget(err_p, split[0]);

    // Pack install does not stream subprocess log output (no installer
    // process -- just a streaming HTTP download), so there's no log_tail
    // analogue. Instead show a short hint pointing the user at the file
    // log for full context.
    let hint = "See ~/.local/share/ichr/ichr.log for the full failure trace.";
    let tail_p = Paragraph::new(hint).wrap(Wrap { trim: false });
    f.render_widget(tail_p, split[1]);
}

pub fn map_pack_install_failed_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::DismissPackInstallFailed),
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
            map_pack_install_failed_event(key(KeyCode::Esc)),
            Some(Action::DismissPackInstallFailed)
        ));
    }

    #[test]
    fn other_keys_noop() {
        assert!(map_pack_install_failed_event(key(KeyCode::Char('q'))).is_none());
        assert!(map_pack_install_failed_event(key(KeyCode::Enter)).is_none());
    }
}
