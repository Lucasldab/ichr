//! Loader install progress modal — step status + LineGauge + cancel hint.
//!
//! Mirrors `download_pane.rs` LineGauge pattern with the modal centering
//! shape from `java_picker_modal.rs`.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Clear, LineGauge, Paragraph};
use ratatui::Frame;

use crate::loader::types::LoaderType;
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_loader_install_progress_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::LoaderInstallProgressModal {
        slug,
        loader,
        version,
        step_label,
        step_index,
        step_total,
        bytes_done,
        bytes_total,
        ..
    } = &state.active_view
    else {
        return;
    };

    let kind = match loader {
        LoaderType::Fabric => "Fabric",
        LoaderType::Quilt => "Quilt",
    };

    let modal_w = area.width.min(70);
    let modal_h = 12u16.min(area.height.saturating_sub(4));
    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal_area = Rect { x, y, width: modal_w, height: modal_h };

    f.render_widget(Clear, modal_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Installing {kind} {version} — {slug} "));
    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let chunks = Layout::vertical([
        Constraint::Length(1), // step status text
        Constraint::Length(1), // blank
        Constraint::Length(1), // LineGauge
        Constraint::Length(1), // blank
        Constraint::Length(1), // step counter
        Constraint::Length(1), // divider
        Constraint::Length(1), // footer hint
    ])
    .split(inner);

    // Row 0: step status text
    let p_status = Paragraph::new(step_label.as_str());
    f.render_widget(p_status, chunks[0]);

    // Row 2: LineGauge
    let ratio = if *bytes_total > 0 {
        (*bytes_done as f64) / (*bytes_total as f64)
    } else if *step_index > 0 {
        // Pre-byte-info: estimate via step progress.
        (*step_index as f64) / (*step_total).max(1) as f64
    } else {
        0.0
    };
    let gauge_label = if *bytes_total > 0 {
        format!(
            "{} / {}  ({}%)",
            fmt_bytes(*bytes_done),
            fmt_bytes(*bytes_total),
            (ratio * 100.0).round() as u32,
        )
    } else {
        format!("{}%", (ratio * 100.0).round() as u32)
    };
    let gauge = LineGauge::default()
        .ratio(ratio.clamp(0.0, 1.0))
        .label(Span::raw(gauge_label))
        .filled_style(Style::default().fg(Color::Green));
    f.render_widget(gauge, chunks[2]);

    // Row 4: step counter
    let p_counter =
        Paragraph::new(format!("Step {step_index} of {step_total}: {step_label}"));
    f.render_widget(p_counter, chunks[4]);

    // Row 5: divider
    let div = "\u{2500}".repeat(inner.width as usize);
    let divider = Paragraph::new(div).style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(divider, chunks[5]);

    // Row 6: footer hint
    let hint =
        Paragraph::new("Esc cancel install").style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(hint, chunks[6]);
}

pub fn map_loader_install_progress_event(
    ev: ratatui::crossterm::event::Event,
    state: &AppState,
) -> Option<Action> {
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => {
            if let ActiveView::LoaderInstallProgressModal { slug, .. } = &state.active_view {
                Some(Action::CancelLoaderInstall { slug: slug.clone() })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn fmt_bytes(n: u64) -> String {
    const MB: f64 = 1_048_576.0;
    const KB: f64 = 1024.0;
    let f = n as f64;
    if f >= MB {
        format!("{:.1} MB", f / MB)
    } else if f >= KB {
        format!("{:.1} KB", f / KB)
    } else {
        format!("{n} B")
    }
}
