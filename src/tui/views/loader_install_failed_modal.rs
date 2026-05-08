//! Loader install failure modal — mirrors `launch_failed_modal.rs`.
//!
//! Shown when LoaderInstallFailed is dispatched. Esc dismisses.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::loader::types::LoaderType;
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_loader_install_failed_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::LoaderInstallFailedModal {
        slug,
        loader,
        version,
        error,
        log_tail,
    } = &state.active_view
    else {
        return;
    };
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
    let kind = match loader {
        LoaderType::Fabric => "Fabric",
        LoaderType::Quilt => "Quilt",
        LoaderType::Forge => "Forge",
        LoaderType::NeoForge => "NeoForge",
    };
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(format!("Install failed: {slug}   (Esc to dismiss)"));
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

    let headline = format!("{kind} {version} installation failed: {error}");
    let err_p = Paragraph::new(headline)
        .style(Style::default().add_modifier(Modifier::BOLD))
        .wrap(Wrap { trim: false });
    f.render_widget(err_p, split[0]);

    let tail_p = Paragraph::new(log_tail.as_str()).wrap(Wrap { trim: false });
    f.render_widget(tail_p, split[1]);
}

pub fn map_loader_install_failed_event(ev: ratatui::crossterm::event::Event) -> Option<Action> {
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::DismissLoaderInstallFailed),
        _ => None,
    }
}
