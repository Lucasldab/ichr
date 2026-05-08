//! Mod browser — full-screen split-pane Modrinth browser.
//!
//! Source: 08-UI-SPEC.md §"Mod Browser" lines 172-251 (layout, copy,
//! palette). j/k disambiguation pattern mirrored from
//! `loader_version_picker_modal.rs:139-143`.
//!
//! Layout:
//!  - Length(3) header — block title + filter chips
//!  - Length(3) search bar — ratatui-textarea single-line input
//!  - Min(1)   body — Percentage(40)/Percentage(60) horizontal split
//!  - Length(1) footer — DIM keybind hint
//!
//! NOTE: this view OWNS the search-input rendering but DOES NOT own a
//! `TextArea` instance — `state.active_view` carries the search String
//! (single source of truth), and we render it directly with the same
//! Yellow/DarkGray palette established by `loader_version_picker_modal.rs`.
//! Rationale: integrating ratatui-textarea would force the search String
//! out of `AppState` (the only mutation point) into a render-side cache,
//! violating the Elm-style "view receives `&AppState`" invariant. The
//! UI-SPEC's "single-line, no Enter/Tab" requirements are equivalent to
//! the existing string-buffer pattern Phase 6 already uses.

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::mods::types::{ModBrowserFetchState, ModrinthSearchHit};
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_mod_browser(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::ModBrowser {
        slug,
        search,
        mc_filter_override,
        loader_filter_override,
        results,
        selected,
        fetch_state,
        selected_detail,
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

    // ---- Header (title + filter chips) ----
    let inst_mc = state
        .instances
        .iter()
        .find(|m| m.slug == *slug)
        .map(|m| m.mc_version_id.clone())
        .unwrap_or_else(|| "?".to_string());
    let inst_loader = state
        .instances
        .iter()
        .find(|m| m.slug == *slug)
        .and_then(|m| {
            m.loader
                .as_ref()
                .map(|l| loader_kind_str(l.kind).to_string())
        })
        .unwrap_or_else(|| "vanilla".to_string());

    // Chip text + style depend on whether the user has overridden the default.
    let mc_chip_text = match mc_filter_override.as_deref() {
        None => format!("MC: {inst_mc} (v=any)"),
        Some(_) => "MC: any (v=any)".to_string(),
    };
    let mc_chip_style = if mc_filter_override.is_none() {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let loader_chip_text = match loader_filter_override.as_deref() {
        None => format!("Loader: {inst_loader} (l=any)"),
        Some(_) => "Loader: any (l=any)".to_string(),
    };
    let loader_chip_style = if loader_filter_override.is_none() {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let header_line = Line::from(vec![
        Span::styled(format!("Mods — {slug}    "), Style::default()),
        Span::styled(format!("[{mc_chip_text}]"), mc_chip_style),
        Span::raw("  "),
        Span::styled(format!("[{loader_chip_text}]"), loader_chip_style),
    ]);
    let header_para = Paragraph::new(header_line).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Mods — {slug} ")),
    );
    f.render_widget(header_para, chunks[0]);

    // ---- Search bar (always-focused, single-line, Yellow when non-empty) ----
    // UI-SPEC §"Mod Browser" lines 215-219: empty → DarkGray placeholder;
    // non-empty → Yellow + trailing cursor underscore.
    let search_display = if search.is_empty() {
        "search: type to search Modrinth...".to_string()
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
    // GAP-FOCUS-INDICATOR-08 (Phase 8.2): Yellow border signals the search
    // input is the focused widget — distinguishes the input from passive
    // surrounding panes (header / results / detail / footer) regardless of
    // whether the buffer is empty. Mirrors the established Yellow=active
    // palette already used by the chip styles and the inner Paragraph.
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
    let results_area = body_split[0];
    let detail_area = body_split[1];

    render_results_pane(f, results_area, results, *selected, fetch_state);
    render_detail_pane(
        f,
        detail_area,
        results.get(*selected),
        selected_detail.as_ref(),
        fetch_state,
    );

    // ---- Footer hint (DIM) ----
    // UI-SPEC §"Mod Browser" line 249 — extended with `Backspace clear` when search non-empty.
    let footer_text = if search.is_empty() {
        "↑/k ↓/j  Enter install  v MC-filter  l loader-filter  Esc back".to_string()
    } else {
        "↑/k ↓/j  Enter install  v MC-filter  l loader-filter  Esc back  Backspace clear"
            .to_string()
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

    // Loading / error / empty single-line states (UI-SPEC lines 226-231).
    let placeholder = match fetch_state {
        ModBrowserFetchState::Loading => Some("Searching Modrinth..."),
        ModBrowserFetchState::Error(_) => Some("Failed to reach Modrinth — check network"),
        ModBrowserFetchState::Ready if results.is_empty() => Some("No mods found"),
        ModBrowserFetchState::Ready => None,
    };
    if let Some(text) = placeholder {
        let style = match fetch_state {
            ModBrowserFetchState::Loading | ModBrowserFetchState::Ready => Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
            // Network-error per UI-SPEC line 231: body style, no Red.
            ModBrowserFetchState::Error(_) => Style::default(),
        };
        let p = Paragraph::new(text).style(style);
        f.render_widget(p, inner);
        return;
    }

    // Two-line rows per result (UI-SPEC lines 222-225).
    // Row width budget: inner.width − 1 (for left padding).
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
            // Truncate name to width − suffix-length − cursor-glyph-length − 1 (left pad).
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
            // Description line (DIM, indent 2, truncated to width − 3).
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
    selected_detail: Option<&crate::mods::types::ModrinthProjectDetail>,
    fetch_state: &ModBrowserFetchState,
) {
    let block = Block::default().borders(Borders::ALL).title(" Detail ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Empty state (no hit selected) — UI-SPEC line 244-246.
    if selected_hit.is_none() {
        let p = Paragraph::new("Select a mod to see details").style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        );
        f.render_widget(p, inner);
        return;
    }

    // While the lazy detail fetch is in flight (08-UI-SPEC line 645).
    // Q4 lock-in (08-RESEARCH.md): detail is fetched on Enter into the version
    // picker, not on row dwell. So selected_detail is generally None here and
    // the detail pane shows the search-hit summary only.

    // Build vertical metadata stack from search-hit + optional detail.
    let mut lines: Vec<Line> = Vec::new();
    let hit = selected_hit.expect("checked above");
    lines.push(Line::from(Span::styled(
        hit.title.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));

    if let Some(d) = selected_detail {
        lines.push(Line::from(Span::styled(
            format!("by {}", d.author),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )));
        lines.push(divider_line(inner.width));
        // Wrap the body across multiple lines via Paragraph::wrap() applied
        // separately below — for now push the raw body then break out.
        lines.push(Line::raw(d.body.clone()));
        lines.push(Line::raw(""));
        lines.push(Line::raw(format!("Downloads: {}", thousands(d.downloads))));
        lines.push(Line::raw(format!(
            "Latest: {} ({})",
            d.latest_version_label, d.latest_version_channel
        )));
        lines.push(Line::raw(format!("License: {}", d.license_id)));
        lines.push(Line::raw(format!(
            "Categories: {}",
            d.categories.join(", ")
        )));
    } else {
        // Summary-only fallback (Q4 lock-in: detail pane stays summary-only).
        lines.push(divider_line(inner.width));
        lines.push(Line::raw(hit.description.clone()));
        lines.push(Line::raw(""));
        lines.push(Line::raw(format!(
            "Downloads: {}",
            thousands(hit.downloads)
        )));
        // Network-error state surfaced if any (UI-SPEC line 646).
        if let ModBrowserFetchState::Error(_) = fetch_state {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "Could not load details — check network".to_string(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
        }
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

/// Truncate `s` to at most `max_w` columns, appending `…` if truncated.
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

/// Format a number with thousands separators using comma grouping
/// (UI-SPEC line 686 — `312,448,221`).
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

fn divider_line(width: u16) -> Line<'static> {
    let n = width.max(1) as usize;
    let s: String = "─".repeat(n);
    Line::from(Span::styled(
        s,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    ))
}

fn loader_kind_str(kind: crate::domain::instance::ModloaderKind) -> &'static str {
    use crate::domain::instance::ModloaderKind;
    match kind {
        ModloaderKind::Fabric => "fabric",
        ModloaderKind::Quilt => "quilt",
        ModloaderKind::Forge => "forge",
        ModloaderKind::NeoForge => "neoforge",
        ModloaderKind::Vanilla => "vanilla",
    }
}

/// Translate a crossterm event to an Action for the ModBrowser view.
///
/// j/k disambiguation (UI-SPEC §Keybind Contract lines 564-581):
///  - When `state.active_view.search` is empty, `j`/`k` navigate.
///  - When `search` is non-empty, `j`/`k` go into the search input.
///  - Up/Down arrows always navigate.
///  - Enter → ModBrowserOpenVersions.
///  - Esc → ModBrowserCancel.
///  - `v` / `l` → toggle MC / loader filter chips.
///  - Backspace → pop char from search.
///  - All other printable chars (incl. space, `/`, digits) → search input.
pub fn map_mod_browser_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    let search_empty = match &state.active_view {
        ActiveView::ModBrowser { search, .. } => search.is_empty(),
        _ => true,
    };
    match ev {
        // Up/Down arrows always navigate (per UI-SPEC line 742).
        CtEvent::Key(KeyEvent {
            code: KeyCode::Up, ..
        }) => Some(Action::ModBrowserMove(-1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Down,
            ..
        }) => Some(Action::ModBrowserMove(1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::ModBrowserCancel),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) => Some(Action::ModBrowserBackspaceSearch),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => Some(Action::ModBrowserOpenVersions),
        // Filter chip toggles (only fire when search is empty, otherwise letters
        // type into the search input — matches UI-SPEC §Keybind Contract).
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('v'),
            modifiers,
            ..
        }) if search_empty && !modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::ToggleModMcFilter)
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('l'),
            modifiers,
            ..
        }) if search_empty && !modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::ToggleModLoaderFilter)
        }
        // j/k disambiguation: navigate when search empty, otherwise type.
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => {
            if search_empty {
                Some(Action::ModBrowserMove(-1))
            } else {
                Some(Action::ModBrowserTypeSearch('k'))
            }
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => {
            if search_empty {
                Some(Action::ModBrowserMove(1))
            } else {
                Some(Action::ModBrowserTypeSearch('j'))
            }
        }
        // All other printable chars → search input.
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::ModBrowserTypeSearch(c)),
        // Bracketed-paste payload (08.1-04 / GAP-8-C): the terminal delivers
        // pasted text as a single `Event::Paste(String)` when bracketed paste
        // is enabled at terminal init. Route the whole payload through one
        // action dispatch instead of a stream of synthetic key events.
        CtEvent::Paste(s) => Some(Action::ModBrowserPasteSearch(s)),
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

    fn state_with_search(s: &str) -> AppState {
        AppState {
            active_view: ActiveView::ModBrowser {
                slug: "foo".into(),
                search: s.to_string(),
                mc_filter_override: None,
                loader_filter_override: None,
                results: Vec::new(),
                selected: 0,
                fetch_state: ModBrowserFetchState::Ready,
                selected_detail: None,
            },
            ..AppState::default()
        }
    }

    #[test]
    fn jk_navigate_when_search_empty() {
        let s = state_with_search("");
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Char('j')), &s),
            Some(Action::ModBrowserMove(1))
        ));
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Char('k')), &s),
            Some(Action::ModBrowserMove(-1))
        ));
    }

    #[test]
    fn jk_type_when_search_nonempty() {
        let s = state_with_search("fa");
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Char('j')), &s),
            Some(Action::ModBrowserTypeSearch('j'))
        ));
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Char('k')), &s),
            Some(Action::ModBrowserTypeSearch('k'))
        ));
    }

    #[test]
    fn arrows_always_navigate_even_with_search() {
        let s = state_with_search("fabric");
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Up), &s),
            Some(Action::ModBrowserMove(-1))
        ));
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Down), &s),
            Some(Action::ModBrowserMove(1))
        ));
    }

    #[test]
    fn enter_opens_versions() {
        let s = state_with_search("");
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Enter), &s),
            Some(Action::ModBrowserOpenVersions)
        ));
    }

    #[test]
    fn esc_cancels() {
        let s = state_with_search("");
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Esc), &s),
            Some(Action::ModBrowserCancel)
        ));
    }

    #[test]
    fn backspace_pops_search() {
        let s = state_with_search("fa");
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Backspace), &s),
            Some(Action::ModBrowserBackspaceSearch)
        ));
    }

    #[test]
    fn v_toggles_mc_filter_when_search_empty() {
        let s = state_with_search("");
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Char('v')), &s),
            Some(Action::ToggleModMcFilter)
        ));
    }

    #[test]
    fn v_types_when_search_nonempty() {
        let s = state_with_search("xy");
        assert!(matches!(
            map_mod_browser_event(key(KeyCode::Char('v')), &s),
            Some(Action::ModBrowserTypeSearch('v'))
        ));
    }

    #[test]
    fn truncate_appends_ellipsis() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("ab", 4), "ab");
        assert_eq!(truncate("abcdef", 1), "…");
        assert_eq!(truncate("abcdef", 0), "");
    }

    #[test]
    fn thousands_groups_correctly() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(312_448_221), "312,448,221");
    }

    /// GAP-8-C / 08.1-04: bracketed-paste payload from the terminal must map
    /// to a single `ModBrowserPasteSearch` action carrying the whole pasted
    /// string. Without this arm pasted text would silently fall through to
    /// `_ => None` and the user would have to type their query character by
    /// character.
    #[test]
    fn paste_event_emits_paste_search_action() {
        let s = state_with_search("");
        let pasted = "fabric api".to_string();
        let result = map_mod_browser_event(CtEvent::Paste(pasted.clone()), &s);
        match result {
            Some(Action::ModBrowserPasteSearch(got)) => assert_eq!(got, pasted),
            other => panic!("expected ModBrowserPasteSearch, got {other:?}"),
        }
    }
}
