//! Pack browser -- full-screen Modrinth browser parameterised by PackKind.
//!
//! Cloned-and-parameterised from `mod_browser.rs` per Phase 11 plan 04.
//! D-LOCK: NO loader or MC filter chips (packs not loader-specific).
//! D-LOCK: `D` (uppercase) opens drop-from-path modal with current slug+kind.
//!
//! Layout mirrors mod_browser.rs:
//!  - Length(3) header -- block title (kind-aware)
//!  - Length(3) search bar -- buffer rendered inline
//!  - Min(1)   body -- results list (40%) / detail pane (60%)
//!  - Length(1) footer -- DIM keybind hint

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::mods::types::{ModBrowserFetchState, ModrinthSearchHit};
use crate::packs::kind::PackKind;
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_pack_browser(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::PackBrowser {
        slug,
        kind,
        search,
        fetch_state,
        results,
        selected,
    } = &state.active_view
    else {
        return;
    };

    // ---- Vertical layout: header / search / body / footer ----
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    // ---- Header (kind-aware title, no loader/MC filter chips per D-LOCK) ----
    let kind_label = match kind {
        PackKind::Resource => "Resource Packs",
        PackKind::Shader => "Shader Packs",
    };
    let header_para = Paragraph::new(format!("{kind_label} -- {slug}")).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {kind_label} -- {slug} ")),
    );
    f.render_widget(header_para, chunks[0]);

    // ---- Search bar ----
    let search_display = if search.is_empty() {
        format!("search: type to search Modrinth {kind_label}...")
    } else {
        format!("search: {search}_")
    };
    let search_style = if search.is_empty() {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let search_para = Paragraph::new(search_display).style(search_style).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Search")
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(search_para, chunks[1]);

    // ---- Body: 40/60 horizontal split (results / detail) ----
    let body_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[2]);

    render_results_pane(f, body_split[0], results, *selected, fetch_state);
    render_detail_pane(f, body_split[1], results.get(*selected), fetch_state);

    // ---- Footer hint ----
    let footer_text = if search.is_empty() {
        "↑/k ↓/j  Enter install  D drop-from-path  Esc back"
    } else {
        "↑/k ↓/j  Enter install  D drop-from-path  Esc back  Backspace clear"
    };
    let footer = Paragraph::new(footer_text).style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(footer, chunks[3]);
}

fn render_results_pane(
    f: &mut Frame,
    area: Rect,
    results: &[ModrinthSearchHit],
    selected: usize,
    fetch_state: &ModBrowserFetchState,
) {
    let block = Block::default().borders(Borders::ALL).title(" Results ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let placeholder = match fetch_state {
        ModBrowserFetchState::Loading => Some("Searching Modrinth..."),
        ModBrowserFetchState::Error(_) => Some("Failed to reach Modrinth -- check network"),
        ModBrowserFetchState::Ready if results.is_empty() => Some("No packs found"),
        ModBrowserFetchState::Ready => None,
    };
    if let Some(text) = placeholder {
        let style = match fetch_state {
            ModBrowserFetchState::Loading | ModBrowserFetchState::Ready => Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
            ModBrowserFetchState::Error(_) => Style::default(),
        };
        f.render_widget(Paragraph::new(text).style(style), inner);
        return;
    }

    let width = inner.width as usize;
    let items: Vec<ListItem> = results
        .iter()
        .enumerate()
        .map(|(i, hit)| {
            let installed_suffix = if hit.already_installed {
                "   ✓ installed"
            } else {
                ""
            };
            let cursor_glyph = if i == selected { " ▶" } else { "" };
            let max_name_w = width.saturating_sub(installed_suffix.len() + cursor_glyph.len() + 1);
            let name = truncate(&hit.title, max_name_w);
            let line1 = if hit.already_installed {
                Line::from(vec![
                    Span::raw(name),
                    Span::styled(
                        installed_suffix.to_string(),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw(cursor_glyph.to_string()),
                ])
            } else {
                Line::from(vec![Span::raw(name), Span::raw(cursor_glyph.to_string())])
            };
            let desc = truncate(&hit.description, width.saturating_sub(3));
            let line2 = Line::from(Span::styled(
                format!("  {desc}"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ));
            let style = if i == selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(vec![line1, line2]).style(style)
        })
        .collect();

    let list = List::new(items);
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(selected));
    f.render_stateful_widget(list, inner, &mut list_state);
}

fn render_detail_pane(
    f: &mut Frame,
    area: Rect,
    selected_hit: Option<&ModrinthSearchHit>,
    fetch_state: &ModBrowserFetchState,
) {
    let block = Block::default().borders(Borders::ALL).title(" Detail ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    if selected_hit.is_none() {
        let p = Paragraph::new("Select a pack to see details").style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        );
        f.render_widget(p, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let hit = selected_hit.expect("checked above");
    lines.push(Line::from(Span::styled(
        hit.title.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    let divider: String = "─".repeat(inner.width.max(1) as usize);
    lines.push(Line::from(Span::styled(
        divider,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    )));
    lines.push(Line::raw(hit.description.clone()));
    lines.push(Line::raw(""));
    lines.push(Line::raw(format!(
        "Downloads: {}",
        thousands(hit.downloads)
    )));
    if let ModBrowserFetchState::Error(_) = fetch_state {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Could not load details -- check network".to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )));
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

fn truncate(s: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_w {
        return s.to_string();
    }
    if max_w == 1 {
        return "…".to_string();
    }
    let cut: String = chars[..max_w.saturating_sub(1)].iter().collect();
    format!("{cut}…")
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 3);
    for (i, b) in bytes.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(b',');
        }
        out.push(*b);
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// Map crossterm events to Actions for the PackBrowser view.
///
/// j/k disambiguation (same as mod_browser.rs):
///  - When search is empty, j/k navigate.
///  - When search is non-empty, j/k type into the buffer.
///  - Up/Down arrows always navigate.
///  - `D` (uppercase) → PackDropPathOpen with current slug+kind.
///  - Esc → PackBrowserClose.
pub fn map_pack_browser_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    let (search_empty, slug, kind) = match &state.active_view {
        ActiveView::PackBrowser {
            search, slug, kind, ..
        } => (search.is_empty(), slug.clone(), *kind),
        _ => return None,
    };

    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Up, ..
        }) => Some(Action::PackBrowserMove(-1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Down,
            ..
        }) => Some(Action::PackBrowserMove(1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::PackBrowserClose),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) => Some(Action::PackBrowserBackspaceSearch),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => Some(Action::InstallPackFromBrowser { slug, kind }),
        // D (uppercase) opens the drop-from-path modal -- kind inherited.
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('D'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::PackDropPathOpen { slug, kind })
        }
        // j/k disambiguation.
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => {
            if search_empty {
                Some(Action::PackBrowserMove(-1))
            } else {
                Some(Action::PackBrowserTypeSearch('k'))
            }
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => {
            if search_empty {
                Some(Action::PackBrowserMove(1))
            } else {
                Some(Action::PackBrowserTypeSearch('j'))
            }
        }
        // All other printable chars → search input.
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::PackBrowserTypeSearch(c)),
        CtEvent::Paste(s) => Some(Action::PackBrowserPasteSearch(s)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::types::ModBrowserFetchState;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> CtEvent {
        CtEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn key_shift(code: KeyCode) -> CtEvent {
        CtEvent::Key(KeyEvent::new(code, KeyModifiers::SHIFT))
    }

    fn browser_state(slug: &str, kind: PackKind, search: &str) -> AppState {
        AppState {
            active_view: ActiveView::PackBrowser {
                slug: slug.into(),
                kind,
                search: search.to_string(),
                fetch_state: ModBrowserFetchState::Ready,
                results: Vec::new(),
                selected: 0,
            },
            ..AppState::default()
        }
    }

    #[test]
    fn arrows_always_navigate() {
        let s = browser_state("foo", PackKind::Resource, "");
        assert!(matches!(
            map_pack_browser_event(key(KeyCode::Up), &s),
            Some(Action::PackBrowserMove(-1))
        ));
        assert!(matches!(
            map_pack_browser_event(key(KeyCode::Down), &s),
            Some(Action::PackBrowserMove(1))
        ));
    }

    #[test]
    fn jk_navigate_when_search_empty() {
        let s = browser_state("foo", PackKind::Resource, "");
        assert!(matches!(
            map_pack_browser_event(key(KeyCode::Char('j')), &s),
            Some(Action::PackBrowserMove(1))
        ));
        assert!(matches!(
            map_pack_browser_event(key(KeyCode::Char('k')), &s),
            Some(Action::PackBrowserMove(-1))
        ));
    }

    #[test]
    fn jk_type_when_search_nonempty() {
        let s = browser_state("foo", PackKind::Resource, "hi");
        assert!(matches!(
            map_pack_browser_event(key(KeyCode::Char('j')), &s),
            Some(Action::PackBrowserTypeSearch('j'))
        ));
        assert!(matches!(
            map_pack_browser_event(key(KeyCode::Char('k')), &s),
            Some(Action::PackBrowserTypeSearch('k'))
        ));
    }

    #[test]
    fn esc_closes_browser() {
        let s = browser_state("foo", PackKind::Resource, "");
        assert!(matches!(
            map_pack_browser_event(key(KeyCode::Esc), &s),
            Some(Action::PackBrowserClose)
        ));
    }

    #[test]
    fn uppercase_d_opens_drop_modal_with_resource_kind() {
        let s = browser_state("foo", PackKind::Resource, "");
        let result = map_pack_browser_event(key_shift(KeyCode::Char('D')), &s);
        assert!(
            matches!(
                result,
                Some(Action::PackDropPathOpen {
                    kind: PackKind::Resource,
                    ..
                })
            ),
            "expected PackDropPathOpen(Resource); got {result:?}"
        );
    }

    #[test]
    fn uppercase_d_opens_drop_modal_with_shader_kind() {
        let s = browser_state("foo", PackKind::Shader, "");
        let result = map_pack_browser_event(key_shift(KeyCode::Char('D')), &s);
        assert!(
            matches!(
                result,
                Some(Action::PackDropPathOpen {
                    kind: PackKind::Shader,
                    ..
                })
            ),
            "expected PackDropPathOpen(Shader); got {result:?}"
        );
    }
}
