//! Loader version picker -- scrollable filtered list with stable-only toggle.
//!
//! Mirrors `version_picker.rs`. Filter bar + scrollable list + Yellow/DarkGray.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::loader::types::LoaderType;
use crate::tui::app::{loader_versions_visible_indices, Action, ActiveView, AppState};

pub fn render_loader_version_picker_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::LoaderVersionPickerModal {
        slug,
        loader,
        versions,
        filter_stable_only,
        search,
        selected,
        current_version,
    } = &state.active_view
    else {
        return;
    };

    let palette = &state.config.colors;
    let dim_style = Style::default()
        .fg(palette.dim.to_color())
        .add_modifier(Modifier::DIM);
    let kind = match loader {
        LoaderType::Fabric => "Fabric",
        LoaderType::Quilt => "Quilt",
        LoaderType::Forge => "Forge",
        LoaderType::NeoForge => "NeoForge",
    };

    let modal_w = area.width.min(70);
    let modal_h = (area.height.saturating_sub(4)).min(20);
    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal_area = Rect {
        x,
        y,
        width: modal_w,
        height: modal_h,
    };

    f.render_widget(Clear, modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(modal_area);

    // Header
    let filter_label = match loader {
        LoaderType::Quilt => "(all versions are pre-release)",
        LoaderType::Fabric | LoaderType::Forge | LoaderType::NeoForge => {
            if *filter_stable_only {
                "stable only (s for all)"
            } else {
                "all (s for stable only)"
            }
        }
    };
    let header = Paragraph::new(vec![
        Line::from(format!("Instance: {slug}")),
        Line::from(Span::styled(
            filter_label,
            Style::default().fg(palette.dim.to_color()),
        )),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {kind} Loader versions -- {slug} ")),
    );
    f.render_widget(header, chunks[0]);

    // Filter bar
    let search_display = if search.is_empty() {
        "/ to filter...".to_string()
    } else {
        format!("/{search}_")
    };
    let search_para = Paragraph::new(search_display)
        .style(Style::default().fg(if search.is_empty() {
            palette.dim.to_color()
        } else {
            palette.accent.to_color()
        }))
        .block(Block::default().borders(Borders::ALL).title("Filter"));
    f.render_widget(search_para, chunks[1]);

    // Version list
    let visible: Vec<usize> =
        loader_versions_visible_indices(versions, *loader, *filter_stable_only, search);

    // MC-incompatibility empty-state copy (D-05): show a precise message for
    // Forge/NeoForge when the instance's MC version is below the supported floor.
    // Otherwise fall back to the generic "check network" message.
    let empty_msg: String = if visible.is_empty() {
        let mc_str: Option<&str> = state
            .instances
            .iter()
            .find(|i| i.slug == *slug)
            .map(|i| i.mc_version_id.as_str());
        match (loader, mc_str) {
            (LoaderType::Forge, Some(mc)) if !crate::loader::types::forge_supported_for_mc(mc) => {
                format!("No Forge available for MC {mc} (Forge requires 1.13+)")
            }
            (LoaderType::NeoForge, Some(mc))
                if !crate::loader::types::neoforge_supported_for_mc(mc) =>
            {
                format!("No NeoForge available for MC {mc} (NeoForge requires 1.20.1+)")
            }
            _ => "No versions found -- check network".to_string(),
        }
    } else {
        String::new()
    };

    let items: Vec<ListItem> = if visible.is_empty() {
        vec![ListItem::new(empty_msg).style(dim_style)]
    } else {
        visible
            .iter()
            .enumerate()
            .map(|(row_i, &orig_i)| {
                let v = &versions[orig_i];
                let stab = if v.stable { "stable" } else { "beta" };
                let mut label = format!("{} ({stab})", v.version);
                if let Some(cur) = current_version {
                    if cur == &v.version {
                        label.push_str("   \u{2190} currently installed");
                    }
                }
                let style = if row_i == *selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default().add_modifier(Modifier::DIM)
                };
                ListItem::new(label).style(style)
            })
            .collect()
    };

    let list_block = Block::default()
        .borders(Borders::ALL)
        .title("Versions (Enter / Esc)");
    let list = List::new(items).block(list_block);
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(*selected));
    f.render_stateful_widget(list, chunks[2], &mut list_state);
}

pub fn map_loader_version_picker_event(ev: ratatui::crossterm::event::Event) -> Option<Action> {
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Up, ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            ..
        }) => Some(Action::LoaderVersionPickerMove(-1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Down,
            ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            ..
        }) => Some(Action::LoaderVersionPickerMove(1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('s'),
            ..
        }) => Some(Action::ToggleStableFilter),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) => Some(Action::LoaderVersionBackspaceSearch),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => Some(Action::LoaderVersionSelect),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::LoaderVersionPickerCancel),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) && c != 'k' && c != 'j' && c != 's' => {
            Some(Action::LoaderVersionTypeSearch(c))
        }
        _ => None,
    }
}
