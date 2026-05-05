//! Top-level view dispatcher. Pure function of `&AppState`.

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;

use super::app::{ActiveView, AppState, CreateStep};
use super::views::{
    account_auth_failed::render_account_auth_failed,
    accounts_list::render_accounts_list,
    add_account_device_code::render_add_account_device_code,
    create_modal::render_create_modal,
    delete_confirm::render_delete_confirm,
    dep_confirm_modal::render_dep_confirm_modal,
    download_pane::render_download_pane,
    installed_mods_list::render_installed_mods_list,
    instance_list::{render_group_inline_overlay, render_instance_list},
    java_picker_modal::render_java_picker_modal,
    launch_failed_modal::render_launch_failed_modal,
    loader_install_failed_modal::render_loader_install_failed_modal,
    loader_install_progress_modal::render_loader_install_progress_modal,
    loader_picker_modal::render_loader_picker_modal,
    loader_switch_confirm::render_loader_switch_confirm,
    loader_version_picker_modal::render_loader_version_picker_modal,
    mod_browser::render_mod_browser,
    mod_install_failed_modal::render_mod_install_failed_modal,
    mod_version_picker_modal::render_mod_version_picker_modal,
    uninstall_mod_confirm::render_uninstall_mod_confirm,
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

    // Phase 8 (08-08): full-screen views (ModBrowser, InstalledModsList) own
    // the entire `main` rect — suppress the instance-list background render so
    // the modless view can claim the full body width without bleed-through.
    let full_screen = matches!(
        state.active_view,
        ActiveView::ModBrowser { .. } | ActiveView::InstalledModsList { .. }
    );
    if !full_screen {
        render_instance_list(f, main, state);
    }

    match &state.active_view {
        ActiveView::InstanceList { .. } => {}
        ActiveView::CreateModal(step) => match step {
            CreateStep::NameInput { .. } => render_create_modal(f, main, state),
            CreateStep::VersionPicker { .. } => render_version_picker(f, main, state),
        },
        ActiveView::DeleteConfirm { .. } => render_delete_confirm(f, main, state),
        ActiveView::RenameInline { .. } => render_create_modal(f, main, state),
        ActiveView::GroupInline { .. } => render_group_inline_overlay(f, main, state),
        ActiveView::LaunchFailedModal { .. } => render_launch_failed_modal(f, main, state),
        ActiveView::AccountsList { .. } => render_accounts_list(f, main, state),
        ActiveView::AddAccountDeviceCode { .. } => render_add_account_device_code(f, main, state),
        ActiveView::AccountAuthFailed { .. } => render_account_auth_failed(f, main, state),
        ActiveView::JavaPickerModal { .. } => render_java_picker_modal(f, main, state),
        ActiveView::LoaderPickerModal { .. } => render_loader_picker_modal(f, main, state),
        ActiveView::LoaderVersionPickerModal { .. } => {
            render_loader_version_picker_modal(f, main, state)
        }
        ActiveView::LoaderInstallProgressModal { .. } => {
            render_loader_install_progress_modal(f, main, state)
        }
        ActiveView::LoaderInstallFailedModal { .. } => {
            render_loader_install_failed_modal(f, main, state)
        }
        ActiveView::LoaderSwitchConfirm { .. } => render_loader_switch_confirm(f, main, state),
        // Phase 8 (08-08): Modrinth views.
        ActiveView::ModBrowser { .. } => render_mod_browser(f, main, state),
        ActiveView::ModVersionPickerModal { .. } => {
            render_mod_version_picker_modal(f, main, state)
        }
        ActiveView::DepConfirmModal { .. } => render_dep_confirm_modal(f, main, state),
        ActiveView::InstalledModsList { .. } => render_installed_mods_list(f, main, state),
        ActiveView::UninstallModConfirm { .. } => render_uninstall_mod_confirm(f, main, state),
        ActiveView::ModInstallFailedModal { .. } => {
            render_mod_install_failed_modal(f, main, state)
        }
    }

    render_download_pane(f, dl, state);
}
