//! Dep-confirm modal -- listing required/optional/incompatible deps before install.
//!
//! Source: 08-UI-SPEC.md §"Dependency-Confirm Modal" lines 277-323.
//! Mirrors `loader_switch_confirm.rs` (centered confirm with optional Red+BOLD
//! warning) and `launch_failed_modal.rs` (header/body Length(3)/Min(1) split).

use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::mods::types::{DepKind, ResolvedDep};
use crate::tui::app::{Action, ActiveView, AppState};

pub fn render_dep_confirm_modal(f: &mut Frame, area: Rect, state: &AppState) {
    let ActiveView::DepConfirmModal {
        slug: _,
        project_id: _,
        project_title,
        version_id: _,
        version_label,
        deps,
        total_bytes,
        total_files,
        has_conflict,
        root_version: _,
    } = &state.active_view
    else {
        return;
    };

    let palette = &state.config.colors;
    let dim_style = Style::default()
        .fg(palette.dim.to_color())
        .add_modifier(Modifier::DIM);
    let error_bold = Style::default()
        .fg(palette.error.to_color())
        .add_modifier(Modifier::BOLD);

    // ---- Modal centering: 70 × 20 cap (UI-SPEC line 295) ----
    let w = area.width.min(70);
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
        .title(format!("Install {project_title} {version_label}"));
    f.render_widget(outer, rect);

    let inner = Rect {
        x: rect.x + 1,
        y: rect.y + 1,
        width: rect.width.saturating_sub(2),
        height: rect.height.saturating_sub(2),
    };

    // Vertical chunks per UI-SPEC §"Dependency-Confirm Modal" Layout (lines 305-312):
    //   Length(1) headline / Length(1) blank / Min(2) deps / Length(1) summary
    //   / Length(1) divider / Length(1) footer.
    // (Header is rendered by the outer block title; we drop the Length(3) header
    //  chunk that's reserved for the title bar -- no separate header content needed.)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // headline
            Constraint::Length(1), // blank
            Constraint::Min(2),    // dep list
            Constraint::Length(1), // summary
            Constraint::Length(1), // divider OR conflict warning
            Constraint::Length(1), // footer hint
        ])
        .split(inner);

    // ---- Headline (UI-SPEC lines 652-653) ----
    // Filter `embedded` deps out before counting (UI-SPEC line 657: "embedded
    // hidden by default; not rendered").
    let visible_deps: Vec<&ResolvedDep> = deps
        .iter()
        .filter(|d| !matches!(d.kind, DepKind::Embedded))
        .collect();
    let n_deps = visible_deps.len();
    let headline_text = if n_deps == 0 {
        "This mod has no dependencies.".to_string()
    } else if n_deps == 1 {
        "1 dependency will also be installed:".to_string()
    } else {
        format!("{n_deps} dependencies will also be installed:")
    };
    let headline_para = Paragraph::new(Line::from(Span::styled(
        headline_text,
        Style::default().add_modifier(Modifier::BOLD),
    )));
    f.render_widget(headline_para, chunks[0]);

    // ---- Dep list (UI-SPEC lines 654-658) ----
    let dep_lines: Vec<Line> = visible_deps
        .iter()
        .map(|d| {
            let (prefix_text, prefix_style) = match d.kind {
                DepKind::Required => ("  required  ", dim_style),
                DepKind::Optional => ("  optional  ", dim_style),
                DepKind::Incompatible => ("  incompatible", error_bold),
                DepKind::Embedded => unreachable!("filtered above"),
            };
            // Body: "{title} {version}" -- version may be None for Optional / Incompatible.
            let version_label = d
                .version
                .as_ref()
                .map(|v| v.version_number.clone())
                .unwrap_or_default();
            let body_text = if version_label.is_empty() {
                format!(" {}", d.project_title)
            } else {
                format!(" {} {}", d.project_title, version_label)
            };
            let mut spans = vec![
                Span::styled(prefix_text.to_string(), prefix_style),
                Span::raw(body_text),
            ];
            if d.already_satisfied {
                spans.push(Span::styled(" (already satisfied)".to_string(), dim_style));
            }
            Line::from(spans)
        })
        .collect();
    let deps_para = Paragraph::new(if dep_lines.is_empty() {
        // 0-deps fallthrough (UI-SPEC line 314): nothing to render in the body.
        vec![Line::raw("")]
    } else {
        dep_lines
    });
    f.render_widget(deps_para, chunks[2]);

    // ---- Summary (UI-SPEC line 659) ----
    let file_word = if *total_files == 1 { "file" } else { "files" };
    let summary_text = format!(
        "Total: {total_files} {file_word} to download (~{})",
        format_bytes(*total_bytes)
    );
    let summary_para = Paragraph::new(summary_text);
    f.render_widget(summary_para, chunks[3]);

    // ---- Conflict warning (UI-SPEC lines 660, 321-323) OR divider ----
    if *has_conflict {
        // Find the first incompatible dep for the warning copy.
        let conflict_name = deps
            .iter()
            .find(|d| matches!(d.kind, DepKind::Incompatible))
            .map(|d| d.project_title.clone())
            .unwrap_or_else(|| "(unknown)".to_string());
        let warn = Paragraph::new(Line::from(Span::styled(
            format!("WARNING: {conflict_name} conflicts with installed mod"),
            error_bold,
        )));
        f.render_widget(warn, chunks[4]);
    } else {
        // DIM horizontal divider.
        let n = chunks[4].width.max(1) as usize;
        let divider = Paragraph::new(Line::from(Span::styled("─".repeat(n), dim_style)));
        f.render_widget(divider, chunks[4]);
    }

    // ---- Footer hint (UI-SPEC lines 661-662) ----
    let footer_text = if *has_conflict {
        "Esc cancel -- resolve conflict first"
    } else {
        "y to install  n/Esc to cancel"
    };
    let footer = Paragraph::new(footer_text).style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(footer, chunks[5]);
}

/// Format bytes as `{whole}.{tenths} {KB|MB|GB}` per UI-SPEC line 687.
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        let v = bytes as f64 / KB as f64;
        format!("{v:.1} KB")
    } else if bytes < GB {
        let v = bytes as f64 / MB as f64;
        format!("{v:.1} MB")
    } else {
        let v = bytes as f64 / GB as f64;
        format!("{v:.1} GB")
    }
}

pub fn map_dep_confirm_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    let has_conflict = matches!(
        &state.active_view,
        ActiveView::DepConfirmModal {
            has_conflict: true,
            ..
        }
    );
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('y'),
            ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('Y'),
            ..
        }) => {
            // y is a no-op when there is a conflict (UI-SPEC line 596).
            if has_conflict {
                Some(Action::CancelModInstall)
            } else {
                Some(Action::ConfirmModInstall)
            }
        }
        CtEvent::Key(_) => Some(Action::CancelModInstall),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::types::ModrinthVersion;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> CtEvent {
        CtEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn fake_v() -> Box<ModrinthVersion> {
        Box::new(ModrinthVersion {
            id: "v".into(),
            project_id: "p".into(),
            name: "n".into(),
            version_number: "1".into(),
            version_type: "release".into(),
            game_versions: vec!["1.20.4".into()],
            loaders: vec!["fabric".into()],
            downloads: 0,
            date_published: "2026-01-01T00:00:00Z".into(),
            dependencies: vec![],
            files: vec![],
        })
    }

    fn state_no_conflict() -> AppState {
        AppState {
            active_view: ActiveView::DepConfirmModal {
                slug: "foo".into(),
                project_id: "P".into(),
                project_title: "Sodium".into(),
                version_id: "V".into(),
                version_label: "1.0".into(),
                deps: vec![],
                total_bytes: 1024,
                total_files: 1,
                has_conflict: false,
                root_version: fake_v(),
            },
            ..AppState::default()
        }
    }

    fn state_with_conflict() -> AppState {
        let mut s = state_no_conflict();
        if let ActiveView::DepConfirmModal { has_conflict, .. } = &mut s.active_view {
            *has_conflict = true;
        }
        s
    }

    #[test]
    fn y_confirms_when_no_conflict() {
        let s = state_no_conflict();
        assert!(matches!(
            map_dep_confirm_event(key(KeyCode::Char('y')), &s),
            Some(Action::ConfirmModInstall)
        ));
        assert!(matches!(
            map_dep_confirm_event(key(KeyCode::Char('Y')), &s),
            Some(Action::ConfirmModInstall)
        ));
    }

    #[test]
    fn y_is_cancel_when_has_conflict() {
        let s = state_with_conflict();
        assert!(matches!(
            map_dep_confirm_event(key(KeyCode::Char('y')), &s),
            Some(Action::CancelModInstall)
        ));
    }

    #[test]
    fn esc_or_n_cancel() {
        let s = state_no_conflict();
        assert!(matches!(
            map_dep_confirm_event(key(KeyCode::Esc), &s),
            Some(Action::CancelModInstall)
        ));
        assert!(matches!(
            map_dep_confirm_event(key(KeyCode::Char('n')), &s),
            Some(Action::CancelModInstall)
        ));
    }

    #[test]
    fn format_bytes_works() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }
}
