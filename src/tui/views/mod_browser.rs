//! Mod browser -- full-screen split-pane Modrinth browser.
//!
//! Source: 08-UI-SPEC.md §"Mod Browser" lines 172-251 (layout, copy,
//! palette). j/k disambiguation pattern mirrored from
//! `loader_version_picker_modal.rs:139-143`.
//!
//! Layout:
//!  - Length(3) header -- block title + filter chips
//!  - Length(3) search bar -- ratatui-textarea single-line input
//!  - Min(1)   body -- Percentage(40)/Percentage(60) horizontal split
//!  - Length(1) footer -- DIM keybind hint
//!
//! NOTE: this view OWNS the search-input rendering but DOES NOT own a
//! `TextArea` instance -- `state.active_view` carries the search String
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
use ratatui_image::Image;

use crate::icons::IconSource;
use crate::mods::types::{ModBrowserFetchState, ModrinthSearchHit};
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_mod_browser(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::ModBrowser {
        slug,
        search,
        is_searching,
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
    let chip_palette = &state.config.colors;
    let mc_chip_style = if mc_filter_override.is_none() {
        Style::default()
            .fg(chip_palette.dim.to_color())
            .add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(chip_palette.accent.to_color())
    };
    let loader_chip_text = match loader_filter_override.as_deref() {
        None => format!("Loader: {inst_loader} (l=any)"),
        Some(_) => "Loader: any (l=any)".to_string(),
    };
    let loader_chip_style = if loader_filter_override.is_none() {
        Style::default()
            .fg(chip_palette.dim.to_color())
            .add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(chip_palette.accent.to_color())
    };

    let header_line = Line::from(vec![
        Span::styled(format!("Mods -- {slug}    "), Style::default()),
        Span::styled(format!("[{mc_chip_text}]"), mc_chip_style),
        Span::raw("  "),
        Span::styled(format!("[{loader_chip_text}]"), loader_chip_style),
    ]);
    let header_para = Paragraph::new(header_line).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Mods -- {slug} ")),
    );
    f.render_widget(header_para, chunks[0]);

    // ---- Search bar ---------------------------------------------------------
    // Vim-style focus indicator: in browse mode the bar uses the `dim`
    // palette slot (border + placeholder text); in search mode it
    // switches to `accent`. Both colors are user-configurable via
    // `~/.config/ichr/config.toml -> [colors] accent / dim / text`.
    let palette = &state.config.colors;
    // Hint text uses the live keybind label so on-screen prompts track
    // user overrides (`browser_begin_search = "?"` -> "press ? to
    // search"). `Esc` is hardcoded because exit-search is not
    // configurable yet; calling out the literal key is honest.
    let search_label = state
        .config
        .keybinds
        .label(crate::config::ActionKey::BrowserBeginSearch);
    let search_display = if *is_searching {
        format!("search: {search}_")
    } else if search.is_empty() {
        format!("press {search_label} to search Modrinth")
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
        Block::default()
            .borders(Borders::ALL)
            .title(title_str)
            .border_style(Style::default().fg(border_color)),
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
        state,
    );

    // ---- Footer hint (DIM) ----
    // UI-SPEC §"Mod Browser" line 249 -- extended with `Backspace clear` when search non-empty.
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
        ModBrowserFetchState::Error(_) => Some("Failed to reach Modrinth -- check network"),
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
    state: &AppState,
) {
    let block = Block::default().borders(Borders::ALL).title(" Detail ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Empty state (no hit selected) -- UI-SPEC line 244-246.
    let Some(hit) = selected_hit else {
        let p = Paragraph::new("Select a mod to see details").style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        );
        f.render_widget(p, inner);
        return;
    };

    // Phase 13: when icons are enabled, carve a top strip for the avatar
    // and render the title / author block to its right. The body Paragraph
    // (description, downloads, license, etc.) renders below in the
    // remaining height. When icons are disabled, fall through to the
    // existing single-Paragraph layout untouched (no Rect carve, no
    // visual difference for halfblocks-only users).
    let (icon_strip, body_area) = if state.icon_rendering_enabled && inner.height >= 5 {
        let strips = Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).split(inner);
        (Some(strips[0]), strips[1])
    } else {
        (None, inner)
    };

    if let Some(strip) = icon_strip {
        let cols = Layout::horizontal([
            Constraint::Length(8), // avatar slot (matches detail_icon_target_rect)
            Constraint::Length(1), // gutter
            Constraint::Min(0),    // title + author
        ])
        .split(strip);
        let icon_rect = cols[0];
        let header_rect = cols[2];

        // Render the icon if the IconService has decoded it. Cache miss is
        // silent: render fns don't dispatch fetches; ModBrowserMove and
        // ModBrowserSearchLoaded already do. Failed fetches stay blank
        // forever within the session per Phase 13 D-06.
        if let Some(svc) = &state.icon_service {
            if let Some(proto) = svc.try_get(IconSource::Modrinth, &hit.project_id) {
                f.render_widget(Image::new(&proto), icon_rect);
            }
        }

        let mut header_lines: Vec<Line> = Vec::new();
        header_lines.push(Line::from(Span::styled(
            hit.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if let Some(d) = selected_detail {
            header_lines.push(Line::from(Span::styled(
                format!("by {}", d.author),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
        }
        f.render_widget(Paragraph::new(header_lines), header_rect);
    }

    // Body content (or full content when icon strip is absent). When the
    // icon strip is present, we omit the title/author lines from the body
    // since the strip already shows them.
    let mut lines: Vec<Line> = Vec::new();
    let header_in_strip = icon_strip.is_some();
    if !header_in_strip {
        lines.push(Line::from(Span::styled(
            hit.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
    }

    if let Some(d) = selected_detail {
        if !header_in_strip {
            lines.push(Line::from(Span::styled(
                format!("by {}", d.author),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
        }
        lines.push(divider_line(body_area.width));
        // Wrap the body across multiple lines via Paragraph::wrap() applied
        // separately below -- for now push the raw body then break out.
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
        lines.push(divider_line(body_area.width));
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
                "Could not load details -- check network".to_string(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
        }
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, body_area);
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
/// (UI-SPEC line 686 -- `312,448,221`).
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
/// Vim-style two-mode dispatch:
///
/// - **Browse mode** (`is_searching == false`, default on open):
///   - Up/Down/`j`/`k` -> nav.
///   - Enter -> ModBrowserOpenVersions.
///   - `v` / `l` -> toggle MC / loader filter chips.
///   - `/` -> ModBrowserBeginSearch (enter search mode).
///   - Esc -> ModBrowserCancel (leave browser).
///   - Other printable chars are ignored (do NOT type into search).
/// - **Search mode** (`is_searching == true`):
///   - Up/Down -> nav (lets user pick a row without leaving search).
///   - Enter -> ModBrowserOpenVersions.
///   - Esc -> ModBrowserExitSearch (back to browse, clears query).
///   - Backspace -> pop char from search.
///   - Every printable char -> type into search.
///   - Bracketed paste -> ModBrowserPasteSearch.
///
/// The previous "letters type once `search` is non-empty" rule meant
/// queries could not start with `v`/`l`/`j`/`k`; the explicit `/`
/// toggle removes that ambiguity.
pub fn map_mod_browser_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    let is_searching = matches!(
        &state.active_view,
        ActiveView::ModBrowser {
            is_searching: true,
            ..
        }
    );
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
            code: KeyCode::Enter,
            ..
        }) => Some(Action::ModBrowserOpenVersions),
        // Esc: in search mode, exit search; in browse mode, leave the browser.
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(if is_searching {
            Action::ModBrowserExitSearch
        } else {
            Action::ModBrowserCancel
        }),
        // Backspace: only meaningful in search mode (popping the buffer).
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) if is_searching => Some(Action::ModBrowserBackspaceSearch),
        // ── Search-mode dispatch ──────────────────────────────────────────
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if is_searching && !modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::ModBrowserTypeSearch(c))
        }
        // Bracketed paste works in both modes: in browse mode it auto-enters
        // search mode (handled in the update arm) so users can paste a query
        // without first pressing `/`.
        CtEvent::Paste(s) => Some(Action::ModBrowserPasteSearch(s)),
        // ── Browse-mode dispatch ──────────────────────────────────────────
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('/'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::ModBrowserBeginSearch),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('v'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::ToggleModMcFilter),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('l'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::ToggleModLoaderFilter),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::ModBrowserMove(-1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::ModBrowserMove(1)),
        // All other browse-mode keys: ignored. Pasted text in browse mode is
        // also ignored (must enter search mode first via `/`).
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
                // Tests pre-vim-mode assumed letters typed once `search` was
                // non-empty; preserve that semantic by treating any non-empty
                // fixture as already in search mode. Empty fixtures default to
                // browse mode (matches OpenModBrowser's initial state).
                is_searching: !s.is_empty(),
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
