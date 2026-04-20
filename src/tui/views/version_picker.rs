use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::app::{ActiveView, AppState, CreateStep, VersionFilter};

pub fn render_version_picker(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::CreateModal(CreateStep::VersionPicker { name, filter, search, error }) =
        &state.active_view
    else {
        return;
    };

    // Center a modal box.
    let modal_area = centered_rect(70, 80, area);
    f.render_widget(Clear, modal_area);

    let filter_label = match filter {
        VersionFilter::Releases => "releases only (t to show all)",
        VersionFilter::All => "releases + snapshots (t to filter)",
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(modal_area);

    // Header
    let header_text = vec![
        Line::from(format!("Instance: {name}")),
        Line::from(Span::styled(filter_label, Style::default().fg(Color::DarkGray))),
    ];
    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).title("Select Version"));
    f.render_widget(header, chunks[0]);

    // Search bar
    let search_display = if search.is_empty() {
        "/ to search...".to_string()
    } else {
        format!("/{search}_")
    };
    let search_para = Paragraph::new(search_display)
        .style(Style::default().fg(if search.is_empty() {
            Color::DarkGray
        } else {
            Color::Yellow
        }))
        .block(Block::default().borders(Borders::ALL).title("Filter"));
    f.render_widget(search_para, chunks[1]);

    // Version list filtered by filter + search
    let search_lc = search.to_ascii_lowercase();
    let filtered: Vec<&_> = state
        .versions
        .iter()
        .filter(|v| match v.version_type.as_str() {
            "release" => true,
            "snapshot" => *filter == VersionFilter::All,
            _ => false,
        })
        .filter(|v| {
            if search_lc.is_empty() {
                true
            } else {
                v.id.to_ascii_lowercase().contains(&search_lc)
            }
        })
        .collect();

    let mut error_block = Block::default().borders(Borders::ALL).title("Versions (Enter / Esc)");
    if let Some(err) = error {
        error_block = error_block.title(format!("Versions — {err}"));
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let style = if i == 0 {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(format!("{} ({})", v.id, v.version_type)).style(style)
        })
        .collect();

    let list = List::new(items).block(error_block);
    f.render_widget(list, chunks[2]);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
