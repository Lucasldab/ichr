//! Top-level view dispatcher. Pure function of `&AppState`.

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;

use super::app::{ActiveView, AppState, CreateStep};
use super::views::{
    create_modal::render_create_modal,
    delete_confirm::render_delete_confirm,
    download_pane::render_download_pane,
    instance_list::{render_group_inline_overlay, render_instance_list},
    version_picker::render_version_picker,
};

pub fn view(state: &AppState, f: &mut Frame) {
    let area = f.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);

    let main = layout[0];
    let dl = layout[1];

    render_instance_list(f, main, state);

    match &state.active_view {
        ActiveView::InstanceList { .. } => {}
        ActiveView::CreateModal(step) => match step {
            CreateStep::NameInput { .. } => render_create_modal(f, main, state),
            CreateStep::VersionPicker { .. } => render_version_picker(f, main, state),
        },
        ActiveView::DeleteConfirm { .. } => render_delete_confirm(f, main, state),
        ActiveView::RenameInline { .. } => render_create_modal(f, main, state),
        ActiveView::GroupInline { .. } => render_group_inline_overlay(f, main, state),
    }

    render_download_pane(f, dl, state);
}
