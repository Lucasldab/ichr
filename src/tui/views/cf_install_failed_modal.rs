//! CurseForge install-failed modal -- mirrors `mod_install_failed_modal.rs`
//! plus the load-bearing `web_url` block per 09-RESEARCH.md §"downloadUrl
//! null UX" lines 254-271.
//!
//! When `web_url.is_some()` the modal renders the RESEARCH-specified copy
//! (BOLD headline + body explanation + "Open in browser:" line + URL line).
//! When `web_url.is_none()` the modal falls back to the Phase 8 generic
//! "Failed to install ..." headline.

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_cf_install_failed_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::CfInstallFailedModal {
        slug: _,
        mod_title,
        file_label,
        error_message,
        web_url,
    } = &state.active_view
    else {
        return;
    };

    let palette = &state.config.colors;
    let dim_style = Style::default()
        .fg(palette.dim.to_color())
        .add_modifier(Modifier::DIM);

    // 80 × 20 cap (mirrors mod_install_failed_modal.rs).
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

    // The split layout adapts to whether web_url is present:
    //  - With web_url:    Length(3) headline / Length(1) divider / Length(2) link block / Min(1) body
    //  - Without web_url: Length(3) headline / Min(1) body  (Phase 8 shape)
    let constraints: Vec<Constraint> = if web_url.is_some() {
        vec![
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(1),
        ]
    } else {
        vec![Constraint::Length(3), Constraint::Min(1)]
    };
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    // ---- Headline (BOLD) ----
    let headline = if web_url.is_some() {
        // RESEARCH-specified copy for the FileNotDownloadable case.
        format!("Cannot download \"{mod_title}\" from CurseForge.")
    } else {
        // Phase 8-style headline for non-restricted errors.
        format!("Failed to install {mod_title} {file_label}: {error_message}")
    };
    let head_p = Paragraph::new(headline)
        .style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(palette.accent.to_color()),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(head_p, split[0]);

    if let Some(url) = web_url.as_deref() {
        // Spacer / divider row (intentionally blank).
        let divider = Paragraph::new("");
        f.render_widget(divider, split[1]);

        // Link block: "Open in browser:" (DIM) above "  {url}" (accent).
        let link_lines = vec![
            Line::from(Span::styled("Open in browser:".to_string(), dim_style)),
            Line::from(Span::styled(
                format!("  {url}"),
                Style::default().fg(palette.accent.to_color()),
            )),
        ];
        let link_p = Paragraph::new(link_lines).wrap(Wrap { trim: false });
        f.render_widget(link_p, split[2]);

        // Body explanation (DIM, wrapped).
        let body_text = "The mod author has disabled third-party downloads. \
You can download the file from CurseForge in your browser, then drop it into the instance mods folder.";
        let body_p = Paragraph::new(body_text)
            .style(dim_style)
            .wrap(Wrap { trim: false });
        f.render_widget(body_p, split[3]);
    } else {
        // Body for non-restricted errors -- repeat error_message for context.
        let body_p = Paragraph::new(error_message.as_str())
            .style(dim_style)
            .wrap(Wrap { trim: false });
        f.render_widget(body_p, split[1]);
    }
}

pub fn map_cf_install_failed_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CfDismissInstallFailed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;

    fn key(code: KeyCode) -> CtEvent {
        CtEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn render_to_string(state: &AppState) -> String {
        let backend = TestBackend::new(80, 20);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            let area = f.area();
            render_cf_install_failed_modal(f, area, state);
        })
        .unwrap();
        let buf = term.backend().buffer();
        let area = buf.area();
        let mut s = String::with_capacity((area.width as usize + 1) * area.height as usize);
        for y in 0..area.height {
            for x in 0..area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn esc_dismisses() {
        assert!(matches!(
            map_cf_install_failed_event(key(KeyCode::Esc)),
            Some(Action::CfDismissInstallFailed)
        ));
    }

    #[test]
    fn other_keys_noop() {
        assert!(map_cf_install_failed_event(key(KeyCode::Char('q'))).is_none());
        assert!(map_cf_install_failed_event(key(KeyCode::Enter)).is_none());
    }

    #[test]
    fn render_with_web_url_shows_url_and_research_copy() {
        let state = AppState {
            active_view: ActiveView::CfInstallFailedModal {
                slug: "inst".into(),
                mod_title: "Wonderful World".into(),
                file_label: "1.5.0".into(),
                error_message: "Author has disabled third-party downloads".into(),
                web_url: Some(
                    "https://www.curseforge.com/minecraft/mc-mods/wwm/files/4567890".into(),
                ),
            },
            ..AppState::default()
        };
        let text = render_to_string(&state);
        assert!(
            text.contains("Cannot download"),
            "RESEARCH headline missing:\n{text}"
        );
        assert!(
            text.contains("Wonderful World"),
            "mod_title missing:\n{text}"
        );
        assert!(
            text.contains("Open in browser"),
            "browser-link label missing:\n{text}"
        );
        assert!(
            text.contains("curseforge.com/minecraft/mc-mods/wwm"),
            "web_url missing:\n{text}"
        );
        assert!(
            text.contains("third-party"),
            "body explanation missing:\n{text}"
        );
    }

    #[test]
    fn render_without_web_url_shows_generic_failure() {
        let state = AppState {
            active_view: ActiveView::CfInstallFailedModal {
                slug: "inst".into(),
                mod_title: "Sodium".into(),
                file_label: "0.5.8".into(),
                error_message: "checksum mismatch".into(),
                web_url: None,
            },
            ..AppState::default()
        };
        let text = render_to_string(&state);
        assert!(
            text.contains("Failed to install Sodium"),
            "generic headline missing:\n{text}"
        );
        assert!(
            text.contains("checksum mismatch"),
            "error_message missing:\n{text}"
        );
        assert!(
            !text.contains("Open in browser"),
            "browser-link label must NOT appear without web_url:\n{text}"
        );
    }
}
