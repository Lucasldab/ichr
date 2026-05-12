//! Top-level view dispatcher. Pure function of `&AppState`.

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::Frame;

use super::app::{ActiveView, AppState, CreateStep};
use super::views::{
    account_auth_failed::render_account_auth_failed,
    accounts_list::render_accounts_list,
    add_account_device_code::render_add_account_device_code,
    cf_browser::render_cf_browser,
    cf_file_picker_modal::render_cf_file_picker_modal,
    cf_install_failed_modal::render_cf_install_failed_modal,
    create_modal::render_create_modal,
    delete_confirm::render_delete_confirm,
    dep_confirm_modal::render_dep_confirm_modal,
    download_pane::render_download_pane,
    installed_mods_list::render_installed_mods_list,
    installed_packs_list::render_installed_packs_list,
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
    modpack_import_failed_modal::render_modpack_import_failed_modal,
    modpack_import_path_modal::render_modpack_import_path_modal,
    modpack_import_progress_modal::render_modpack_import_progress_modal,
    pack_browser::render_pack_browser,
    pack_drop_path_modal::render_pack_drop_path_modal,
    pack_install_failed_modal::render_pack_install_failed_modal,
    uninstall_mod_confirm::render_uninstall_mod_confirm,
    uninstall_pack_confirm::render_uninstall_pack_confirm,
    version_picker::render_version_picker,
};

pub fn view(state: &AppState, f: &mut Frame) {
    let area = f.area();

    // Paint base palette.text fg under everything. Widgets that don't set
    // their own fg inherit this (ratatui Style::patch leaves the existing
    // fg when the new fg is None), so terminal-default white stops leaking
    // into views that only use Modifier::BOLD/DIM/REVERSED.
    f.render_widget(
        Block::default().style(Style::default().fg(state.config.colors.text.to_color())),
        area,
    );

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

    // Phase 8 (08-08) + Phase 9 (09-07): full-screen views (ModBrowser,
    // InstalledModsList, CfBrowser) own the entire `main` rect -- suppress the
    // instance-list background render so the modless view can claim the full
    // body width without bleed-through. The CurseForge file-picker and
    // install-failed modals overlay normally.
    // Phase 11 (11-04): PackBrowser and InstalledPacksList also claim full screen.
    let full_screen = matches!(
        state.active_view,
        ActiveView::ModBrowser { .. }
            | ActiveView::InstalledModsList { .. }
            | ActiveView::CfBrowser { .. }
            | ActiveView::PackBrowser { .. }
            | ActiveView::InstalledPacksList { .. }
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
        ActiveView::ModVersionPickerModal { .. } => render_mod_version_picker_modal(f, main, state),
        ActiveView::DepConfirmModal { .. } => render_dep_confirm_modal(f, main, state),
        ActiveView::InstalledModsList { .. } => render_installed_mods_list(f, main, state),
        ActiveView::UninstallModConfirm { .. } => render_uninstall_mod_confirm(f, main, state),
        ActiveView::ModInstallFailedModal { .. } => render_mod_install_failed_modal(f, main, state),
        // Phase 9 (09-07): CurseForge views.
        ActiveView::CfBrowser { .. } => render_cf_browser(f, main, state),
        ActiveView::CfFilePickerModal { .. } => render_cf_file_picker_modal(f, main, state),
        ActiveView::CfInstallFailedModal { .. } => render_cf_install_failed_modal(f, main, state),
        // Phase 10 (10-06): Modpack import views.
        ActiveView::ModpackImportPathInput { .. } => {
            render_modpack_import_path_modal(f, main, state)
        }
        ActiveView::ModpackImportProgressModal { .. } => {
            render_modpack_import_progress_modal(f, main, state)
        }
        ActiveView::ModpackImportFailedModal { .. } => {
            render_modpack_import_failed_modal(f, main, state)
        }
        // Phase 11 (11-04): pack browser + installed packs list + drop-path modal + confirm.
        ActiveView::PackBrowser { .. } => render_pack_browser(f, main, state),
        ActiveView::InstalledPacksList { .. } => render_installed_packs_list(f, main, state),
        ActiveView::PackDropPathInput { .. } => render_pack_drop_path_modal(f, main, state),
        ActiveView::PackInstallFailedModal { .. } => {
            render_pack_install_failed_modal(f, main, state)
        }
        ActiveView::UninstallPackConfirm { .. } => render_uninstall_pack_confirm(f, main, state),
    }

    render_download_pane(f, dl, state);
}
