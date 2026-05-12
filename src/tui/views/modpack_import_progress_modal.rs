//! Modpack import progress modal -- step status + LineGauge + log-tail + cancel hint.
//!
//! Verbatim adaptation of `loader_install_progress_modal.rs` with field-name
//! substitutions: `modpack_name` replaces `slug+loader+version`, same
//! LineGauge + log_tail layout (PATTERNS.md §9).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Clear, LineGauge, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_modpack_import_progress_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::ModpackImportProgressModal {
        modpack_name,
        step_label,
        step_index,
        step_total,
        bytes_done,
        bytes_total,
        log_tail,
        ..
    } = &state.active_view
    else {
        return;
    };

    let palette = &state.config.colors;
    let modal_w = area.width.min(70);
    let modal_h = 22u16.min(area.height.saturating_sub(4));
    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal_area = Rect {
        x,
        y,
        width: modal_w,
        height: modal_h,
    };

    f.render_widget(Clear, modal_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Importing {modpack_name} "));
    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let chunks = Layout::vertical([
        Constraint::Length(1), // [0] step status text
        Constraint::Length(1), // [1] blank
        Constraint::Length(1), // [2] LineGauge
        Constraint::Length(1), // [3] blank
        Constraint::Length(1), // [4] step counter
        Constraint::Length(1), // [5] divider
        Constraint::Length(1), // [6] log-tail header
        Constraint::Min(1),    // [7] log-tail Paragraph
        Constraint::Length(1), // [8] footer hint
    ])
    .split(inner);

    // Row 0: step status text
    let p_status = Paragraph::new(step_label.as_str());
    f.render_widget(p_status, chunks[0]);

    // Row 2: LineGauge
    let ratio = if *bytes_total > 0 {
        (*bytes_done as f64) / (*bytes_total as f64)
    } else if *step_index > 0 {
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
        .filled_style(Style::default().fg(palette.success.to_color()));
    f.render_widget(gauge, chunks[2]);

    // Row 4: step counter
    let p_counter = Paragraph::new(format!("Step {step_index} of {step_total}: {step_label}"));
    f.render_widget(p_counter, chunks[4]);

    // Row 5: divider
    let div = "\u{2500}".repeat(inner.width as usize);
    let divider = Paragraph::new(div).style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(divider, chunks[5]);

    // Row 6: log-tail header
    let header = Paragraph::new("\u{2500} Import output \u{2500}")
        .style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(header, chunks[6]);

    // Row 7: log_tail Paragraph
    let tail_p = Paragraph::new(log_tail.as_str())
        .style(Style::default().add_modifier(Modifier::DIM))
        .wrap(Wrap { trim: false });
    f.render_widget(tail_p, chunks[7]);

    // Row 8: footer hint
    let hint =
        Paragraph::new("Esc cancel import").style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(hint, chunks[8]);
}

pub fn map_modpack_import_progress_event(ev: ratatui::crossterm::event::Event) -> Option<Action> {
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CancelModpackImport),
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
