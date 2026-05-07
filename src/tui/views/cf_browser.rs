//! CurseForge mod browser — full-screen split-pane CurseForge browser.
//!
//! Source: 09-RESEARCH.md §"TUI Integration Plumbing" + 09-PATTERNS.md
//! §`src/tui/views/cf_browser.rs` (deltas off `mod_browser.rs`).
//!
//! Layout mirrors `mod_browser.rs` (Phase 8 analog) verbatim:
//!  - Length(3) header — block title + filter chips
//!  - Length(3) search bar — string-buffer input (no ratatui-textarea)
//!  - Min(1)   body — Percentage(40)/Percentage(60) horizontal split
//!  - Length(1) footer — DIM keybind hint
//!
//! Differences from Phase 8 ModBrowser:
//!  - Wire types are CurseForge (`CurseForgeSearchHit`, `CurseForgeProjectDetail`).
//!  - `loader_filter` is `Option<i32>` (CurseForge ModLoaderType enum).
//!  - Search placeholder mentions CurseForge.
//!  - Detail pane uses `authors` list (CurseForge bundles authors with the
//!    project response — no separate fetch).

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::mods::curseforge::types::{CurseForgeProjectDetail, CurseForgeSearchHit};
use crate::mods::types::ModBrowserFetchState;
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_cf_browser(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::CfBrowser {
        slug,
        search_input,
        results,
        selected,
        fetch_state,
        mc_filter,
        loader_filter,
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
        .and_then(|m| m.loader.as_ref().map(|l| loader_kind_str(l.kind).to_string()))
        .unwrap_or_else(|| "vanilla".to_string());

    // Chip text + style depend on whether the user has overridden the default.
    let mc_chip_text = match mc_filter.as_deref() {
        None => format!("MC: {inst_mc} (v=any)"),
        Some(_) => "MC: any (v=any)".to_string(),
    };
    let mc_chip_style = if mc_filter.is_none() {
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let loader_chip_text = match loader_filter {
        None => format!("Loader: {inst_loader} (l=any)"),
        Some(_) => "Loader: any (l=any)".to_string(),
    };
    let loader_chip_style = if loader_filter.is_none() {
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let header_line = Line::from(vec![
        Span::styled(format!("CurseForge Mods — {slug}    "), Style::default()),
        Span::styled(format!("[{mc_chip_text}]"), mc_chip_style),
        Span::raw("  "),
        Span::styled(format!("[{loader_chip_text}]"), loader_chip_style),
    ]);
    let header_para = Paragraph::new(header_line).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" CurseForge Mods — {slug} ")),
    );
    f.render_widget(header_para, chunks[0]);

    // ---- Search bar (always-focused, single-line, Yellow when non-empty) ----
    let search_display = if search_input.is_empty() {
        "search: type to search CurseForge...".to_string()
    } else {
        format!("search: {search_input}_")
    };
    let search_style = if search_input.is_empty() {
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(Color::Yellow)
    };
    // GAP-FOCUS-INDICATOR-08 (Phase 8.2): symmetric mirror of mod_browser
    // focus indicator. Both browsers must communicate focus identically
    // for visual consistency across mod sources.
    let search_para = Paragraph::new(search_display)
        .style(search_style)
        .block(
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
    let footer_text = if search_input.is_empty() {
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
    results: &[CurseForgeSearchHit],
    selected: usize,
    fetch_state: &ModBrowserFetchState,
) {
    let block = Block::default().borders(Borders::ALL).title(" Results ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let placeholder = match fetch_state {
        ModBrowserFetchState::Loading => Some("Searching CurseForge..."),
        ModBrowserFetchState::Error(_) => Some("Failed to reach CurseForge — check network"),
        ModBrowserFetchState::Ready if results.is_empty() => Some("No mods found"),
        ModBrowserFetchState::Ready => None,
    };
    if let Some(text) = placeholder {
        let style = match fetch_state {
            ModBrowserFetchState::Loading | ModBrowserFetchState::Ready => {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)
            }
            ModBrowserFetchState::Error(_) => Style::default(),
        };
        let p = Paragraph::new(text).style(style);
        f.render_widget(p, inner);
        return;
    }

    let width = inner.width as usize;
    let items: Vec<ListItem> = results
        .iter()
        .enumerate()
        .map(|(i, hit)| {
            let installed_suffix = if hit.already_installed { "   ✓ installed" } else { "" };
            let cursor_glyph = if i == selected { " ▶" } else { "" };
            let max_name_w =
                width.saturating_sub(installed_suffix.len() + cursor_glyph.len() + 1);
            let name = truncate(&hit.name, max_name_w);
            let line1 = if hit.already_installed {
                Line::from(vec![
                    Span::raw(name),
                    Span::styled(installed_suffix.to_string(), Style::default().fg(Color::Green)),
                    Span::raw(cursor_glyph.to_string()),
                ])
            } else {
                Line::from(vec![Span::raw(name), Span::raw(cursor_glyph.to_string())])
            };
            let desc = truncate(&hit.summary, width.saturating_sub(3));
            let line2 = Line::from(Span::styled(
                format!("  {desc}"),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
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
    selected_hit: Option<&CurseForgeSearchHit>,
    selected_detail: Option<&CurseForgeProjectDetail>,
    fetch_state: &ModBrowserFetchState,
) {
    let block = Block::default().borders(Borders::ALL).title(" Detail ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    if selected_hit.is_none() {
        let p = Paragraph::new("Select a mod to see details")
            .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM));
        f.render_widget(p, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let hit = selected_hit.expect("checked above");
    lines.push(Line::from(Span::styled(
        hit.name.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));

    if let Some(d) = selected_detail {
        let authors_joined = if d.authors.is_empty() {
            String::new()
        } else {
            d.authors
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !authors_joined.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("by {authors_joined}"),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            )));
        }
        lines.push(divider_line(inner.width));
        lines.push(Line::raw(d.summary.clone()));
        lines.push(Line::raw(""));
        lines.push(Line::raw(format!("Downloads: {}", thousands(d.download_count))));
        // CurseForge mod response does not carry "latest version" inline —
        // direct the user to the file picker.
        lines.push(Line::raw("Latest: see Files (Enter)"));
        if !d.links.website_url.is_empty() {
            lines.push(Line::raw(format!("Website: {}", d.links.website_url)));
        }
    } else {
        lines.push(divider_line(inner.width));
        lines.push(Line::raw(hit.summary.clone()));
        lines.push(Line::raw(""));
        lines.push(Line::raw(format!("Downloads: {}", thousands(hit.download_count))));
        if let ModBrowserFetchState::Error(_) = fetch_state {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "Could not load details — check network".to_string(),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
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

/// Format a number with thousands separators using comma grouping.
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
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
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

/// Translate a crossterm event to an Action for the CfBrowser view.
///
/// Keymap mirrors `map_mod_browser_event` (Phase 8 analog) verbatim:
///  - When `state.active_view.search_input` is empty, `j`/`k` navigate.
///  - When non-empty, `j`/`k` go into the search input.
///  - Up/Down arrows always navigate.
///  - Enter on the highlighted row → `Action::CfBrowserOpenDetail { slug, mod_id }`.
///  - Esc → `Action::CloseModal` (returns to the InstanceList, mirroring Phase 8).
///  - `v` / `l` (when search empty) → toggle MC / loader filter chips.
///  - Backspace → pop char from search.
///  - All other printable chars → search input.
pub fn map_cf_browser_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    let (search_empty, selected_mod_id, selected_slug): (bool, Option<u64>, Option<String>) =
        match &state.active_view {
            ActiveView::CfBrowser {
                slug,
                search_input,
                results,
                selected,
                ..
            } => (
                search_input.is_empty(),
                results.get(*selected).map(|h| h.id),
                Some(slug.clone()),
            ),
            _ => (true, None, None),
        };
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Up, .. }) => Some(Action::CfBrowserMoveSelection(-1)),
        CtEvent::Key(KeyEvent { code: KeyCode::Down, .. }) => Some(Action::CfBrowserMoveSelection(1)),
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::CloseModal),
        CtEvent::Key(KeyEvent { code: KeyCode::Backspace, .. }) => {
            Some(Action::CfBrowserBackspaceSearch)
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
            let slug = selected_slug?;
            let mod_id = selected_mod_id?;
            Some(Action::CfBrowserOpenDetail { slug, mod_id })
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char('v'), modifiers, .. })
            if search_empty && !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(Action::CfBrowserToggleMcFilter)
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char('l'), modifiers, .. })
            if search_empty && !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(Action::CfBrowserToggleLoaderFilter)
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char('k'), modifiers, .. })
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if search_empty {
                Some(Action::CfBrowserMoveSelection(-1))
            } else {
                Some(Action::CfBrowserTypeSearch('k'))
            }
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char('j'), modifiers, .. })
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if search_empty {
                Some(Action::CfBrowserMoveSelection(1))
            } else {
                Some(Action::CfBrowserTypeSearch('j'))
            }
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char(c), modifiers, .. })
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(Action::CfBrowserTypeSearch(c))
        }
        // Bracketed-paste payload (08.1-04 / GAP-8-C): the terminal delivers
        // pasted text as a single `Event::Paste(String)` when bracketed paste
        // is enabled at terminal init. Route the whole payload through one
        // action dispatch instead of a stream of synthetic key events.
        CtEvent::Paste(s) => Some(Action::CfBrowserPasteSearch(s)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::curseforge::types::CurseForgeSearchHit;
    use crate::mods::types::ModBrowserFetchState;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> CtEvent {
        CtEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn state_with_search(s: &str) -> AppState {
        state_with_search_and_results(s, Vec::new())
    }

    fn state_with_search_and_results(s: &str, results: Vec<CurseForgeSearchHit>) -> AppState {
        AppState {
            active_view: ActiveView::CfBrowser {
                slug: "foo".into(),
                search_input: s.to_string(),
                results,
                selected: 0,
                fetch_state: ModBrowserFetchState::Ready,
                mc_filter: None,
                loader_filter: None,
                selected_detail: None,
            },
            ..AppState::default()
        }
    }

    fn fx_hit(id: u64, name: &str) -> CurseForgeSearchHit {
        CurseForgeSearchHit {
            id,
            slug: name.into(),
            name: name.into(),
            summary: String::new(),
            download_count: 0,
            categories: Vec::new(),
            already_installed: false,
        }
    }

    #[test]
    fn jk_navigate_when_search_empty() {
        let s = state_with_search("");
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Char('j')), &s),
            Some(Action::CfBrowserMoveSelection(1))
        ));
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Char('k')), &s),
            Some(Action::CfBrowserMoveSelection(-1))
        ));
    }

    #[test]
    fn jk_type_when_search_nonempty() {
        let s = state_with_search("fa");
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Char('j')), &s),
            Some(Action::CfBrowserTypeSearch('j'))
        ));
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Char('k')), &s),
            Some(Action::CfBrowserTypeSearch('k'))
        ));
    }

    #[test]
    fn arrows_always_navigate_even_with_search() {
        let s = state_with_search("fabric");
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Up), &s),
            Some(Action::CfBrowserMoveSelection(-1))
        ));
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Down), &s),
            Some(Action::CfBrowserMoveSelection(1))
        ));
    }

    #[test]
    fn enter_opens_detail_when_row_present() {
        let s = state_with_search_and_results("", vec![fx_hit(443959, "Sodium")]);
        match map_cf_browser_event(key(KeyCode::Enter), &s) {
            Some(Action::CfBrowserOpenDetail { slug, mod_id }) => {
                assert_eq!(slug, "foo");
                assert_eq!(mod_id, 443959);
            }
            other => panic!("expected CfBrowserOpenDetail, got {other:?}"),
        }
    }

    #[test]
    fn enter_noop_when_results_empty() {
        let s = state_with_search("");
        assert!(map_cf_browser_event(key(KeyCode::Enter), &s).is_none());
    }

    #[test]
    fn esc_returns_close_modal() {
        let s = state_with_search("");
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Esc), &s),
            Some(Action::CloseModal)
        ));
    }

    #[test]
    fn backspace_pops_search() {
        let s = state_with_search("fa");
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Backspace), &s),
            Some(Action::CfBrowserBackspaceSearch)
        ));
    }

    #[test]
    fn v_toggles_mc_filter_when_search_empty() {
        let s = state_with_search("");
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Char('v')), &s),
            Some(Action::CfBrowserToggleMcFilter)
        ));
    }

    #[test]
    fn l_toggles_loader_filter_when_search_empty() {
        let s = state_with_search("");
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Char('l')), &s),
            Some(Action::CfBrowserToggleLoaderFilter)
        ));
    }

    #[test]
    fn v_types_when_search_nonempty() {
        let s = state_with_search("xy");
        assert!(matches!(
            map_cf_browser_event(key(KeyCode::Char('v')), &s),
            Some(Action::CfBrowserTypeSearch('v'))
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
    /// to a single `CfBrowserPasteSearch` action carrying the whole pasted
    /// string — same contract as the Modrinth analog.
    #[test]
    fn paste_event_emits_paste_search_action() {
        let s = state_with_search("");
        let pasted = "create big cannons".to_string();
        let result = map_cf_browser_event(CtEvent::Paste(pasted.clone()), &s);
        match result {
            Some(Action::CfBrowserPasteSearch(got)) => assert_eq!(got, pasted),
            other => panic!("expected CfBrowserPasteSearch, got {other:?}"),
        }
    }
}
