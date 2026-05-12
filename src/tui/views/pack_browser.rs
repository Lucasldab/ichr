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
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use ratatui_image::Image;

use crate::config::Palette;
use crate::icons::IconSource;
use crate::mods::types::{ModBrowserFetchState, ModrinthSearchHit};
use crate::packs::kind::PackKind;
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_pack_browser(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::PackBrowser {
        slug,
        kind,
        search,
        is_searching,
        fetch_state,
        results,
        selected,
        scroll_offset,
    } = &state.active_view
    else {
        return;
    };

    let palette = &state.config.colors;

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
        crate::tui::theme::block(palette)
            .title(format!(" {kind_label} -- {slug} ")),
    );
    f.render_widget(header_para, chunks[0]);

    // ---- Search bar (vim-style focus indicator; mirrors mod_browser.rs) ----
    // Color slots and hint text both consult the user config, so a
    // rebound `browser_begin_search` shows up in the placeholder and
    // title automatically.
    let search_label = state
        .config
        .keybinds
        .label(crate::config::ActionKey::BrowserBeginSearch);
    let search_display = if *is_searching {
        format!("search: {search}_")
    } else if search.is_empty() {
        format!("press {search_label} to search Modrinth {kind_label}")
    } else {
        format!("search: {search}")
    };
    let search_style = if *is_searching {
        Style::default().fg(palette.accent.to_color())
    } else if search.is_empty() {
        Style::default()
            .fg(palette.dim.to_color())
            .add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(palette.text.to_color())
    };
    let border_color = if *is_searching {
        palette.accent.to_color()
    } else {
        palette.dim.to_color()
    };
    let title_str = if *is_searching {
        "Search [Esc]".to_string()
    } else {
        format!("Search [{search_label}]")
    };
    let search_para = Paragraph::new(search_display).style(search_style).block(
        crate::tui::theme::block(palette)
            .title(title_str)
            .border_style(Style::default().fg(border_color)),
    );
    f.render_widget(search_para, chunks[1]);

    // ---- Body: 40/60 horizontal split (results / detail) ----
    let body_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[2]);

    render_results_pane(
        f,
        body_split[0],
        results,
        *selected,
        *scroll_offset,
        fetch_state,
        state,
    );
    render_detail_pane(f, body_split[1], results.get(*selected), fetch_state, state);

    // ---- Footer hint ----
    let footer_text = if search.is_empty() {
        "↑/k ↓/j  Enter install  D drop-from-path  Esc back"
    } else {
        "↑/k ↓/j  Enter install  D drop-from-path  Esc back  Backspace clear"
    };
    let footer = Paragraph::new(footer_text).style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(footer, chunks[3]);
}

/// Phase 14 dispatch: rich path on terminals with image-protocol support,
/// else the existing `List` + `ListState` fallback.
fn render_results_pane(
    f: &mut Frame,
    area: Rect,
    results: &[ModrinthSearchHit],
    selected: usize,
    scroll_offset: usize,
    fetch_state: &ModBrowserFetchState,
    state: &AppState,
) {
    if state.icon_rendering_enabled {
        render_results_pane_rich(
            f,
            area,
            results,
            selected,
            scroll_offset,
            fetch_state,
            state,
        );
    } else {
        render_results_pane_table(f, area, results, selected, fetch_state, &state.config.colors);
    }
}

/// Halfblocks fallback -- existing v0.2.x layout, untouched.
fn render_results_pane_table(
    f: &mut Frame,
    area: Rect,
    results: &[ModrinthSearchHit],
    selected: usize,
    fetch_state: &ModBrowserFetchState,
    palette: &Palette,
) {
    let dim_style = Style::default()
        .fg(palette.dim.to_color())
        .add_modifier(Modifier::DIM);
    let block = crate::tui::theme::block(palette).title(" Results ");
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
            ModBrowserFetchState::Loading | ModBrowserFetchState::Ready => dim_style,
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
                        Style::default().fg(palette.success.to_color()),
                    ),
                    Span::raw(cursor_glyph.to_string()),
                ])
            } else {
                Line::from(vec![Span::raw(name), Span::raw(cursor_glyph.to_string())])
            };
            let desc = truncate(&hit.description, width.saturating_sub(3));
            let line2 = Line::from(Span::styled(format!("  {desc}"), dim_style));
            let style = if i == selected {
                Style::default().bg(palette.selected_bg.to_color())
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

/// Phase 14 rich-path render. Mirrors `mod_browser::render_results_pane_rich`
/// (same row layout, same scroll math via `mod_browser::next_scroll_offset`).
/// Pack source is always Modrinth in current scope.
fn render_results_pane_rich(
    f: &mut Frame,
    area: Rect,
    results: &[ModrinthSearchHit],
    selected: usize,
    scroll_offset: usize,
    fetch_state: &ModBrowserFetchState,
    state: &AppState,
) {
    let palette = &state.config.colors;
    let dim_style = Style::default()
        .fg(palette.dim.to_color())
        .add_modifier(Modifier::DIM);
    let block = crate::tui::theme::block(palette).title(" Results ");
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
            ModBrowserFetchState::Loading | ModBrowserFetchState::Ready => dim_style,
            ModBrowserFetchState::Error(_) => Style::default(),
        };
        f.render_widget(Paragraph::new(text).style(style), inner);
        return;
    }

    let row_h: u16 = 2;
    let visible_rows = (inner.height / row_h) as usize;
    if visible_rows == 0 {
        return;
    }
    let max_offset = results.len().saturating_sub(visible_rows);
    let offset = scroll_offset.min(max_offset);

    let row_constraints: Vec<Constraint> = (0..visible_rows)
        .map(|_| Constraint::Length(row_h))
        .collect();
    let row_rects = Layout::vertical(row_constraints).split(inner);

    for (visible_i, &row_rect) in row_rects.iter().enumerate() {
        let i = offset + visible_i;
        let Some(hit) = results.get(i) else {
            break;
        };
        let is_selected = i == selected;

        if is_selected {
            let highlight =
                Block::default().style(Style::default().bg(palette.selected_bg.to_color()));
            f.render_widget(highlight, row_rect);
        }

        let cols = Layout::horizontal([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(row_rect);
        let icon_rect = cols[0];
        let text_rect = cols[2];

        if let Some(svc) = &state.icon_service {
            if let Some(proto) = svc.try_get(
                IconSource::Modrinth,
                &hit.project_id,
                crate::icons::list_row_icon_target_rect(),
            ) {
                f.render_widget(Image::new(&proto), icon_rect);
            }
        }

        let width = text_rect.width as usize;
        let installed_suffix = if hit.already_installed {
            "   ✓ installed"
        } else {
            ""
        };
        let cursor_glyph = if is_selected { " ▶" } else { "" };
        let max_name_w = width.saturating_sub(installed_suffix.len() + cursor_glyph.len());
        let name = truncate(&hit.title, max_name_w);
        let line1 = if hit.already_installed {
            Line::from(vec![
                Span::raw(name),
                Span::styled(
                    installed_suffix.to_string(),
                    Style::default().fg(palette.success.to_color()),
                ),
                Span::raw(cursor_glyph.to_string()),
            ])
        } else {
            Line::from(vec![Span::raw(name), Span::raw(cursor_glyph.to_string())])
        };
        let desc = truncate(&hit.description, width);
        let line2 = Line::from(Span::styled(desc, dim_style));
        f.render_widget(Paragraph::new(vec![line1, line2]), text_rect);
    }
}

fn render_detail_pane(
    f: &mut Frame,
    area: Rect,
    selected_hit: Option<&ModrinthSearchHit>,
    fetch_state: &ModBrowserFetchState,
    state: &AppState,
) {
    let palette = &state.config.colors;
    let dim_style = Style::default()
        .fg(palette.dim.to_color())
        .add_modifier(Modifier::DIM);
    let block = crate::tui::theme::block(palette).title(" Detail ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(hit) = selected_hit else {
        let p = Paragraph::new("Select a pack to see details").style(dim_style);
        f.render_widget(p, inner);
        return;
    };

    // Phase 13: same icon-strip carve as mod_browser. When icons are
    // disabled or pane is too narrow, fall through to single-Paragraph
    // layout untouched.
    let (icon_strip, body_area) = if state.icon_rendering_enabled && inner.height >= 5 {
        let strips = Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).split(inner);
        (Some(strips[0]), strips[1])
    } else {
        (None, inner)
    };

    if let Some(strip) = icon_strip {
        let cols = Layout::horizontal([
            Constraint::Length(8),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(strip);
        let icon_rect = cols[0];
        let header_rect = cols[2];

        if let Some(svc) = &state.icon_service {
            if let Some(proto) = svc.try_get(
                IconSource::Modrinth,
                &hit.project_id,
                crate::icons::detail_icon_target_rect(),
            ) {
                f.render_widget(Image::new(&proto), icon_rect);
            }
        }

        let header_lines = vec![Line::from(Span::styled(
            hit.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ))];
        f.render_widget(Paragraph::new(header_lines), header_rect);
    }

    let mut lines: Vec<Line> = Vec::new();
    if icon_strip.is_none() {
        lines.push(Line::from(Span::styled(
            hit.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
    }
    let divider: String = "─".repeat(body_area.width.max(1) as usize);
    lines.push(Line::from(Span::styled(divider, dim_style)));
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
            dim_style,
        )));
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, body_area);
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
/// Vim-style two-mode dispatch (mirrors `mod_browser::map_mod_browser_event`).
///   - **Browse mode** (`is_searching == false`, default on open):
///       - Up/Down/`j`/`k` → nav.
///       - Enter → InstallPackFromBrowser.
///       - `D` (uppercase) → PackDropPathOpen.
///       - `/` → PackBrowserBeginSearch.
///       - Esc → PackBrowserClose.
///   - **Search mode** (`is_searching == true`):
///       - Up/Down → nav. Enter still installs.
///       - Esc → PackBrowserExitSearch.
///       - Backspace → pop char. Printable chars → type.
///       - Bracketed paste → PackBrowserPasteSearch.
pub fn map_pack_browser_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    let (is_searching, slug, kind) = match &state.active_view {
        ActiveView::PackBrowser {
            is_searching,
            slug,
            kind,
            ..
        } => (*is_searching, slug.clone(), *kind),
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
            code: KeyCode::Enter,
            ..
        }) => Some(Action::InstallPackFromBrowser { slug, kind }),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(if is_searching {
            Action::PackBrowserExitSearch
        } else {
            Action::PackBrowserClose
        }),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) if is_searching => Some(Action::PackBrowserBackspaceSearch),
        // ── Search-mode dispatch ──────────────────────────────────────────
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if is_searching && !modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::PackBrowserTypeSearch(c))
        }
        // Paste works in both modes; auto-enters search in browse mode
        // (handled in the update arm). Mirrors mod_browser behavior.
        CtEvent::Paste(s) => Some(Action::PackBrowserPasteSearch(s)),
        // ── Browse-mode dispatch ──────────────────────────────────────────
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('/'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::PackBrowserBeginSearch),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('D'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::PackDropPathOpen { slug, kind })
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::PackBrowserMove(-1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::PackBrowserMove(1)),
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
                // Mirror mod_browser fixture: non-empty `search` implies the
                // user is in vim search mode (pre-vim-mode tests assumed
                // typing semantics once `search` was non-empty).
                is_searching: !search.is_empty(),
                fetch_state: ModBrowserFetchState::Ready,
                results: Vec::new(),
                selected: 0,
                scroll_offset: 0,
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
