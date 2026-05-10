//! Async event loop. Reads crossterm events, `TaskEvent`s, and a render tick,
//! dispatches them through `update`, executes any returned `Effect`s, and
//! redraws.
//!
//! Invariants:
//!   * No blocking I/O inside `.await` bodies here.
//!   * `action_tx.send(..)` in background arms uses `let _ =` -- receiver
//!     being dropped is a valid shutdown signal.
//!   * `execute_effects` match is EXHAUSTIVE over Effect variants -- no dead
//!     branches, no SpawnVersionInstall arm.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use ratatui::crossterm::event::{Event as CtEvent, EventStream, KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use tokio::time::interval;

use super::app::{
    update, Action, ActiveView, AppState, CreateStep, Effect, JavaPickerRow, VersionFilter,
};
use super::terminal::Tui;
use super::view::view;
use crate::auth::service::{AccountAuthEvent, AccountService};
use crate::config::ActionKey;
use crate::install::install_version;
use crate::java::service::JavaService;
use crate::launcher;
use crate::loader::service::LoaderService;
use crate::loader::types::{LoaderInfo, LoaderType};
use crate::modpack::service::ModpackService;
use crate::mods::curseforge::service::CurseForgeService;
use crate::mods::service::ModrinthService;
use crate::mojang::client::MojangClient;
use crate::mojang::types::VersionEntry;
use crate::packs::kind::PackKind;
use crate::packs::service::PackService;
use crate::persistence::paths::AppPaths;
use crate::services::{
    clone_instance, create_instance, delete_instance, list_instances, rename_instance, set_group,
};
use crate::tasks::{TaskEvent, TaskManager, DEFAULT_MAX_CONCURRENT};

/// Apply the version filter + case-insensitive substring search. Always
/// excludes `old_beta` and `old_alpha` -- those are out of Phase 2 scope per
/// ROADMAP "Out of Scope".
pub fn filter_version_list<'a>(
    versions: &'a [VersionEntry],
    filter: VersionFilter,
    search: &str,
) -> Vec<&'a VersionEntry> {
    let search_lc = search.to_ascii_lowercase();
    versions
        .iter()
        .filter(|v| match v.version_type.as_str() {
            "release" => true,
            "snapshot" => filter == VersionFilter::All,
            _ => false,
        })
        .filter(|v| {
            if search_lc.is_empty() {
                true
            } else {
                v.id.to_ascii_lowercase().contains(&search_lc)
            }
        })
        .collect()
}

/// Run the TUI event loop until `Action::Quit` is dispatched.
///
/// Consumes the terminal; callers must call `tui::restore_terminal` after
/// this returns (whether Ok or Err).
///
/// `image_picker` is the Phase 13 image-protocol picker built in
/// `tui::init_terminal`. `None` means the terminal didn't respond to the
/// protocol query (or returned `Halfblocks`, which Spike 001 verified is
/// unusable at row sizes); callers downstream check `state.icon_rendering_enabled`
/// before carving icon Rects in the detail pane.
pub async fn run(
    mut terminal: Tui,
    image_picker: Option<ratatui_image::picker::Picker>,
) -> anyhow::Result<()> {
    // Resolve platform paths.
    let paths =
        AppPaths::resolve().ok_or_else(|| anyhow::anyhow!("cannot resolve platform paths"))?;

    // Build the shared Mojang HTTP client.
    let mojang = Arc::new(MojangClient::new()?);

    // Build the AccountService (uses a separate reqwest client with the same UA).
    let http_for_auth = reqwest::Client::builder()
        .user_agent(crate::mojang::client::USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest for auth: {e}"))?;
    let account_service = Arc::new(AccountService::new(&paths, http_for_auth));

    // Build the JavaService (owns Mojang JRE + Adoptium clients; constructed once).
    let java_service =
        Arc::new(JavaService::new().map_err(|e| anyhow::anyhow!("JavaService::new: {e}"))?);
    let loader_service =
        Arc::new(LoaderService::new().map_err(|e| anyhow::anyhow!("LoaderService::new: {e}"))?);
    // Phase 8 (08-08): Modrinth service backs the mod browser/install flow.
    let modrinth_service =
        Arc::new(ModrinthService::new().map_err(|e| anyhow::anyhow!("ModrinthService::new: {e}"))?);
    // Phase 9 (09-07): CurseForge service backs the F-keybind browser/install flow.
    // Pitfall 1: `CurseForgeService::new` returns `Ok` even when no API key is
    // configured (the launcher continues to function for everything else; F
    // keybind silently no-ops via api_key_present()=false). The `?` here is
    // defensive against unexpected ctor failures (e.g., malformed key string
    // tripping reqwest header parsing).
    let cf_service = Arc::new(
        CurseForgeService::new().map_err(|e| anyhow::anyhow!("CurseForgeService::new: {e}"))?,
    );
    // Phase 10 (10-06): Modpack service backs the `i`-keybind import flow.
    let modpack_service =
        Arc::new(ModpackService::new().map_err(|e| anyhow::anyhow!("ModpackService::new: {e}"))?);
    // Phase 11 (11-04): Pack service backs R/S pack browser + install flows.
    let pack_service =
        Arc::new(PackService::new().map_err(|e| anyhow::anyhow!("PackService::new: {e}"))?);

    // Build the task plumbing. TaskEvents arrive on task_rx; we convert each
    // into an Action::Task and forward to the event loop via action_tx.
    let (action_tx, mut action_rx) = mpsc::channel::<Action>(256);
    let (task_tx, mut task_rx) = mpsc::channel::<TaskEvent>(256);
    let task_manager = TaskManager::new(task_tx, DEFAULT_MAX_CONCURRENT);

    // Pump TaskEvents into the Action bus.
    {
        let action_tx = action_tx.clone();
        tokio::spawn(async move {
            while let Some(evt) = task_rx.recv().await {
                if action_tx.send(Action::Task(evt)).await.is_err() {
                    break;
                }
            }
        });
    }

    // Phase 9 (09-07): seed AppState with the CurseForge api-key flag so the
    // F keybind can no-op silently and instance_list can render the title hint.
    // Use struct-update syntax to satisfy clippy::field_reassign_with_default
    // (Phase 1 precedent -- same pattern as arch/os).
    //
    // User config: load `<config_dir>/config.toml` once at startup. Missing
    // file or parse errors fall back to defaults (see `config::Config::load`).
    let config = std::sync::Arc::new(crate::config::Config::load(
        &paths.config_dir.join("config.toml"),
    ));
    // Phase 13: derive the icon-rendering toggle from the detected
    // protocol. Predicate lives in `icons::rendering_enabled` so the
    // halfblocks-rejection rule stays unit-testable (Spike 001 verdict).
    let icon_rendering_enabled = crate::icons::rendering_enabled(image_picker.as_ref());

    // Build the icon service iff icons are enabled. The service owns its
    // own picker clone (Picker is `Clone`) so AppState's copy can be used
    // directly for any future picker introspection without locking the
    // service.
    let icon_service = if icon_rendering_enabled {
        match image_picker.clone() {
            Some(picker) => match crate::icons::IconService::new(picker) {
                Ok(svc) => Some(std::sync::Arc::new(svc)),
                Err(e) => {
                    tracing::warn!(error = %e, "icon service failed to build -- icons disabled");
                    None
                }
            },
            None => None,
        }
    } else {
        None
    };

    let mut state = AppState {
        cf_api_key_present: cf_service.api_key_present(),
        config,
        image_picker,
        icon_rendering_enabled,
        icon_service,
        ..AppState::default()
    };

    // Load instances on startup.
    {
        let paths2 = paths.clone();
        let tx = action_tx.clone();
        tokio::spawn(async move {
            match list_instances(&paths2).await {
                Ok(list) => {
                    let _ = tx.send(Action::InstancesLoaded(list)).await;
                }
                Err(e) => {
                    let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                }
            }
        });
    }

    // Load accounts on startup.
    {
        let svc = Arc::clone(&account_service);
        let tx = action_tx.clone();
        tokio::spawn(async move {
            match svc.list_accounts().await {
                Ok(list) => {
                    let _ = tx.send(Action::AccountsLoaded(list)).await;
                }
                Err(e) => {
                    let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                }
            }
        });
    }

    let mut events = EventStream::new();
    let mut render_tick = interval(Duration::from_millis(16));
    // Interval fires immediately on the first poll; consume that initial tick
    // so the first real render happens after the loop starts.
    render_tick.tick().await;

    // Initial render before entering the select loop.
    terminal.draw(|f| view(&state, f))?;

    loop {
        tokio::select! {
            biased;

            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(ev)) => {
                        if let Some(action) = map_event(ev, &state) {
                            let effects = update(&mut state, action);
                            execute_effects(effects, &state, &paths, Arc::clone(&mojang), Arc::clone(&account_service), Arc::clone(&java_service), Arc::clone(&loader_service), Arc::clone(&modrinth_service), Arc::clone(&cf_service), Arc::clone(&modpack_service), Arc::clone(&pack_service), &task_manager, &action_tx).await;
                        }
                    }
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "crossterm event stream error");
                    }
                    None => break,
                }
            }

            maybe_action = action_rx.recv() => {
                match maybe_action {
                    Some(action) => {
                        let effects = update(&mut state, action);
                        execute_effects(effects, &state, &paths, Arc::clone(&mojang), Arc::clone(&account_service), Arc::clone(&java_service), Arc::clone(&loader_service), Arc::clone(&modrinth_service), Arc::clone(&cf_service), Arc::clone(&modpack_service), Arc::clone(&pack_service), &task_manager, &action_tx).await;
                    }
                    None => break,
                }
            }

            _ = render_tick.tick() => {
                terminal.draw(|f| view(&state, f))?;
            }
        }

        if state.should_quit {
            break;
        }
    }

    Ok(())
}

/// Translate a crossterm event into an `Action` based on the current view context.
///
/// `map_event_pub` is the public alias used by integration tests.
pub fn map_event_pub(ev: CtEvent, state: &AppState) -> Option<Action> {
    map_event(ev, state)
}

fn map_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    // Global: Ctrl+C always quits.
    if let CtEvent::Key(KeyEvent {
        code: KeyCode::Char('c'),
        modifiers,
        ..
    }) = &ev
    {
        if modifiers.contains(KeyModifiers::CONTROL) {
            return Some(Action::Quit);
        }
    }

    match &state.active_view {
        ActiveView::InstanceList { .. } => map_instance_list_event(ev, state),
        ActiveView::CreateModal(CreateStep::NameInput { .. }) => map_name_input_event(ev, state),
        ActiveView::CreateModal(CreateStep::VersionPicker { .. }) => {
            map_version_picker_event(ev, state)
        }
        ActiveView::DeleteConfirm { .. } => map_delete_confirm_event(ev),
        ActiveView::RenameInline { .. } => map_rename_inline_event(ev, state),
        ActiveView::GroupInline { .. } => map_group_inline_event(ev, state),
        ActiveView::LaunchFailedModal { .. } => map_launch_failed_modal_event(ev),
        ActiveView::AccountsList { .. } => map_accounts_list_event(ev, state),
        ActiveView::AddAccountDeviceCode { .. } => map_add_account_device_code_event(ev),
        ActiveView::AccountAuthFailed { .. } => map_account_auth_failed_event(ev),
        ActiveView::JavaPickerModal { .. } => {
            super::views::java_picker_modal::map_java_picker_event(ev)
        }
        // Phase 6: loader modals -- event mappers wired in 06-08.
        ActiveView::LoaderPickerModal { .. } => {
            super::views::loader_picker_modal::map_loader_picker_event(ev)
        }
        ActiveView::LoaderVersionPickerModal { .. } => {
            super::views::loader_version_picker_modal::map_loader_version_picker_event(ev)
        }
        ActiveView::LoaderInstallProgressModal { .. } => {
            super::views::loader_install_progress_modal::map_loader_install_progress_event(
                ev, state,
            )
        }
        ActiveView::LoaderInstallFailedModal { .. } => {
            super::views::loader_install_failed_modal::map_loader_install_failed_event(ev)
        }
        ActiveView::LoaderSwitchConfirm { .. } => {
            super::views::loader_switch_confirm::map_loader_switch_confirm_event(ev)
        }
        // Phase 8 (08-08): Modrinth view event mappers.
        ActiveView::ModBrowser { .. } => {
            super::views::mod_browser::map_mod_browser_event(ev, state)
        }
        ActiveView::ModVersionPickerModal { .. } => {
            super::views::mod_version_picker_modal::map_mod_version_picker_event(ev)
        }
        ActiveView::DepConfirmModal { .. } => {
            super::views::dep_confirm_modal::map_dep_confirm_event(ev, state)
        }
        ActiveView::InstalledModsList { .. } => {
            super::views::installed_mods_list::map_installed_mods_list_event(ev)
        }
        ActiveView::UninstallModConfirm { .. } => {
            super::views::uninstall_mod_confirm::map_uninstall_mod_confirm_event(ev)
        }
        ActiveView::ModInstallFailedModal { .. } => {
            super::views::mod_install_failed_modal::map_mod_install_failed_event(ev)
        }
        // Phase 9 (09-07): CurseForge view event mappers.
        ActiveView::CfBrowser { .. } => super::views::cf_browser::map_cf_browser_event(ev, state),
        // Phase 10 (10-06): Modpack import view event mappers.
        ActiveView::ModpackImportPathInput { .. } => {
            super::views::modpack_import_path_modal::map_modpack_import_path_event(ev)
        }
        ActiveView::ModpackImportProgressModal { .. } => {
            super::views::modpack_import_progress_modal::map_modpack_import_progress_event(ev)
        }
        ActiveView::ModpackImportFailedModal { .. } => {
            super::views::modpack_import_failed_modal::map_modpack_import_failed_event(ev)
        }
        ActiveView::CfFilePickerModal { .. } => {
            super::views::cf_file_picker_modal::map_cf_file_picker_event(ev)
        }
        ActiveView::CfInstallFailedModal { .. } => {
            super::views::cf_install_failed_modal::map_cf_install_failed_event(ev)
        }
        // Phase 11 (11-04): pack browser + installed packs list + drop-path + confirm.
        ActiveView::PackBrowser { .. } => {
            super::views::pack_browser::map_pack_browser_event(ev, state)
        }
        ActiveView::InstalledPacksList { .. } => {
            super::views::installed_packs_list::map_installed_packs_list_event(ev, state)
        }
        ActiveView::PackDropPathInput { .. } => {
            super::views::pack_drop_path_modal::map_pack_drop_path_event(ev)
        }
        ActiveView::UninstallPackConfirm { .. } => {
            super::views::uninstall_pack_confirm::map_uninstall_pack_confirm_event(ev)
        }
        ActiveView::PackInstallFailedModal { .. } => {
            super::views::pack_install_failed_modal::map_pack_install_failed_event(ev)
        }
    }
}

fn map_instance_list_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    // Configurable-keybind dispatch (`~/.config/ichr/config.toml ->
    // [keybinds]`). Runs before the hardcoded match arms so user
    // overrides win. Actions whose slot is not yet rebindable
    // (s/r/x/d/Esc/arrow-nav/k) keep their hardcoded handlers below.
    if let CtEvent::Key(kev) = ev {
        let kb = &state.config.keybinds;
        if kb.matches(ActionKey::Quit, &kev) {
            return Some(Action::Quit);
        }
        if kb.matches(ActionKey::OpenCreateInstance, &kev) {
            return Some(Action::OpenCreateModal);
        }
        if kb.matches(ActionKey::OpenModpackImport, &kev) {
            return Some(Action::OpenModpackImport);
        }
        if kb.matches(ActionKey::OpenAccountsList, &kev) {
            return Some(Action::OpenAccounts);
        }
        // Per-instance actions: need a selected, non-running target.
        if let ActiveView::InstanceList { selected } = &state.active_view {
            if let Some(m) = state.instances.get(*selected) {
                let slug = m.slug.clone();
                let running = state.running_instances.contains_key(&slug);
                if kb.matches(ActionKey::LaunchInstance, &kev) {
                    return if running {
                        // T-03-05-01 belt-and-suspenders: ignore launch
                        // attempts on a running instance.
                        None
                    } else {
                        Some(Action::LaunchInstance { slug })
                    };
                }
                if kb.matches(ActionKey::OpenJavaPicker, &kev) {
                    // Cannot change Java mid-run.
                    return if running {
                        None
                    } else {
                        Some(Action::OpenJavaPicker { slug })
                    };
                }
                if kb.matches(ActionKey::OpenLoaderPicker, &kev) {
                    let installing = state.running_loader_installs.contains_key(&slug);
                    return if running || installing {
                        None
                    } else {
                        Some(Action::OpenLoaderPicker { slug })
                    };
                }
                if kb.matches(ActionKey::OpenModBrowser, &kev) {
                    // Pitfall 8 (08-RESEARCH.md): silent no-op while a mod
                    // install is in flight.
                    return if state.running_mod_jobs.contains_key(&slug) {
                        None
                    } else {
                        Some(Action::OpenModBrowser { slug })
                    };
                }
                if kb.matches(ActionKey::OpenInstalledMods, &kev) {
                    return Some(Action::OpenInstalledMods { slug });
                }
                if kb.matches(ActionKey::OpenCfBrowser, &kev) {
                    // Pitfall 1 + 8 inheritance.
                    return if state.cf_api_key_present
                        && !state.running_mod_jobs.contains_key(&slug)
                    {
                        Some(Action::OpenCfBrowser { slug })
                    } else {
                        None
                    };
                }
                if kb.matches(ActionKey::OpenPackResourceBrowser, &kev) {
                    let in_flight = state
                        .running_pack_jobs
                        .contains_key(&(slug.clone(), PackKind::Resource));
                    return if in_flight {
                        None
                    } else {
                        Some(Action::OpenPackBrowser {
                            slug,
                            kind: PackKind::Resource,
                        })
                    };
                }
                if kb.matches(ActionKey::OpenPackShaderBrowser, &kev) {
                    let in_flight = state
                        .running_pack_jobs
                        .contains_key(&(slug.clone(), PackKind::Shader));
                    return if in_flight {
                        None
                    } else {
                        Some(Action::OpenPackBrowser {
                            slug,
                            kind: PackKind::Shader,
                        })
                    };
                }
            }
        }
    }
    // Hardcoded fallback for keys that don't (yet) have a configurable
    // slot: Esc-as-quit, lifecycle keys (s/r/x/d), and arrow / hjk-style
    // navigation. Migrating these to `[keybinds]` is a follow-up.
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::Quit),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('s'),
            ..
        }) => {
            if let ActiveView::InstanceList { selected } = &state.active_view {
                if let Some(m) = state.instances.get(*selected) {
                    if state.running_instances.contains_key(&m.slug) {
                        return Some(Action::StopInstance {
                            slug: m.slug.clone(),
                        });
                    }
                }
            }
            None
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('r'),
            ..
        }) => {
            if let ActiveView::InstanceList { selected } = &state.active_view {
                if let Some(m) = state.instances.get(*selected) {
                    return Some(Action::OpenRenameInline {
                        slug: m.slug.clone(),
                        current: m.display_name.clone(),
                    });
                }
            }
            None
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('x'),
            ..
        }) => Some(Action::CloneSelected),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('d'),
            ..
        }) => {
            if let ActiveView::InstanceList { selected } = &state.active_view {
                if let Some(m) = state.instances.get(*selected) {
                    // T-03-05-02: block deletion of a running instance.
                    if state.running_instances.contains_key(&m.slug) {
                        return None;
                    }
                    return Some(Action::OpenDeleteConfirm {
                        slug: m.slug.clone(),
                        display_name: m.display_name.clone(),
                    });
                }
            }
            None
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('g'),
            ..
        }) => {
            if let ActiveView::InstanceList { selected } = &state.active_view {
                if let Some(m) = state.instances.get(*selected) {
                    return Some(Action::OpenGroupInput {
                        slug: m.slug.clone(),
                        current: m.group.clone().unwrap_or_default(),
                    });
                }
            }
            None
        }
        // ── Configurable bindings (A / j / L / M / m / F / R / S) live in
        //   `[keybinds]` and are dispatched at the top of this fn. The
        //   arms that previously matched these letters were removed when
        //   the dispatcher landed -- editing config.toml is the way to
        //   change them now. The hardcoded arms below cover keys that do
        //   not yet have a slot in the keybind enum (Down/Up/k for nav,
        //   c/i are bound but their lowercase forms still flow through
        //   the same code).
        CtEvent::Key(KeyEvent {
            code: KeyCode::Down,
            ..
        }) => Some(Action::MoveSelection(1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Up, ..
        }) => Some(Action::MoveSelection(-1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            ..
        }) => Some(Action::MoveSelection(-1)),
        _ => None,
    }
}

fn map_accounts_list_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CloseAccounts),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('a'),
            ..
        }) => Some(Action::AddAccount),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('x'),
            ..
        }) => {
            if let ActiveView::AccountsList { selected } = &state.active_view {
                if let Some(account) = state.accounts.get(*selected) {
                    return Some(Action::RemoveAccount {
                        id: account.id.clone(),
                    });
                }
            }
            None
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => {
            if let ActiveView::AccountsList { selected } = &state.active_view {
                if let Some(account) = state.accounts.get(*selected) {
                    return Some(Action::ActivateAccount {
                        id: account.id.clone(),
                    });
                }
            }
            None
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Down,
            ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            ..
        }) => Some(Action::MoveSelection(1)),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Up, ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            ..
        }) => Some(Action::MoveSelection(-1)),
        _ => None,
    }
}

fn map_add_account_device_code_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CancelAddAccount),
        _ => None,
    }
}

fn map_account_auth_failed_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CloseModal),
        _ => None,
    }
}

fn map_launch_failed_modal_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CloseModal),
        _ => None,
    }
}

fn map_name_input_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CloseModal),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) => Some(Action::BackspaceName),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => {
            if let ActiveView::CreateModal(CreateStep::NameInput { current, .. }) =
                &state.active_view
            {
                return Some(Action::SubmitInstanceName(current.clone()));
            }
            None
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::TypeName(c)),
        // Bracketed-paste payload (08.1-04 / GAP-8-C): the terminal delivers
        // pasted text as a single `Event::Paste(String)` when bracketed paste
        // is enabled at terminal init. Route the whole payload through one
        // action dispatch instead of a stream of synthetic key events.
        CtEvent::Paste(s) => Some(Action::PasteName(s)),
        _ => None,
    }
}

fn map_version_picker_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CloseModal),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('t'),
            ..
        }) => {
            let new_filter = match state.versions_filter {
                VersionFilter::Releases => VersionFilter::All,
                VersionFilter::All => VersionFilter::Releases,
            };
            Some(Action::SetVersionFilter(new_filter))
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) => Some(Action::BackspaceSearch),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => {
            // Select the first visible version.
            if let ActiveView::CreateModal(CreateStep::VersionPicker { filter, search, .. }) =
                &state.active_view
            {
                let visible = filter_version_list(&state.versions, *filter, search);
                if let Some(v) = visible.first() {
                    return Some(Action::SelectVersion(v.id.clone()));
                }
            }
            None
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::TypeSearch(c)),
        _ => None,
    }
}

fn map_delete_confirm_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char('y'),
            ..
        })
        | CtEvent::Key(KeyEvent {
            code: KeyCode::Char('Y'),
            ..
        }) => Some(Action::ConfirmDelete),
        CtEvent::Key(_) => Some(Action::CancelDelete),
        _ => None,
    }
}

fn map_rename_inline_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CloseModal),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) => Some(Action::BackspaceRename),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => {
            if let ActiveView::RenameInline { current, .. } = &state.active_view {
                return Some(Action::SubmitRename(current.clone()));
            }
            None
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::TypeRename(c)),
        _ => None,
    }
}

fn map_group_inline_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => Some(Action::CancelGroupInput),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) => Some(Action::BackspaceGroup),
        CtEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => {
            if matches!(state.active_view, ActiveView::GroupInline { .. }) {
                return Some(Action::SubmitGroup);
            }
            None
        }
        CtEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => Some(Action::TypeGroup(c)),
        _ => None,
    }
}

/// Execute declarative side-effects. Keeps `update` pure.
/// Match is exhaustive -- no SpawnVersionInstall arm, no dead-code branches.
///
/// `state` is the just-updated state (after `update()` ran) -- read-only.
/// Used by Effect::LaunchInstance to look up the instance display_name (username).
#[allow(clippy::too_many_arguments)]
async fn execute_effects(
    effects: Vec<Effect>,
    state: &AppState,
    paths: &AppPaths,
    mojang: Arc<MojangClient>,
    account_service: Arc<AccountService>,
    java_service: Arc<JavaService>,
    loader_service: Arc<LoaderService>,
    modrinth_service: Arc<ModrinthService>,
    cf_service: Arc<CurseForgeService>,
    modpack_service: Arc<ModpackService>,
    pack_service: Arc<PackService>,
    task_manager: &TaskManager,
    action_tx: &mpsc::Sender<Action>,
) {
    for eff in effects {
        match eff {
            Effect::Quit => {
                // No additional work -- should_quit flag drives loop exit.
            }

            Effect::FetchInstances => {
                let paths = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match list_instances(&paths).await {
                        Ok(list) => {
                            let _ = tx.send(Action::InstancesLoaded(list)).await;
                        }
                        Err(e) => {
                            let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                        }
                    }
                });
            }

            Effect::FetchManifest => {
                let paths = paths.clone();
                let mojang = Arc::clone(&mojang);
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    let cache = paths.cache_dir.join("manifest_v2.json");
                    match mojang.fetch_manifest(&cache).await {
                        Ok(m) => {
                            let _ = tx.send(Action::ManifestLoaded(m.versions)).await;
                        }
                        Err(e) => {
                            let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                        }
                    }
                });
            }

            Effect::CreateInstance {
                display_name,
                mc_version_id,
                version_url,
                version_sha1,
            } => {
                // Step 1: create the instance record (fast, no network).
                // Done synchronously in an async block so the compiler doesn't
                // need 'static on task_manager.
                let paths2 = paths.clone();
                let mojang2 = Arc::clone(&mojang);
                let tx = action_tx.clone();
                match create_instance(paths, &display_name, &mc_version_id).await {
                    Err(e) => {
                        let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                    }
                    Ok(manifest) => {
                        // Refresh instance list immediately.
                        if let Ok(list) = list_instances(paths).await {
                            let _ = tx.send(Action::InstancesLoaded(list)).await;
                        }
                        let slug = manifest.slug.clone();
                        let entry = VersionEntry {
                            id: mc_version_id.clone(),
                            version_type: "release".into(),
                            url: version_url,
                            time: String::new(),
                            release_time: String::new(),
                            sha1: version_sha1,
                            compliance_level: 1,
                        };
                        // Step 2: hand off the install work to TaskManager so
                        // the TUI stays responsive during the download.
                        let job_id = task_manager.next_job_id();
                        let tx2 = tx.clone();
                        task_manager.spawn_task(job_id, move |task_tx, token| async move {
                            let res = install_version(
                                job_id, &paths2, &mojang2, task_tx, token, &slug, &entry,
                            )
                            .await;
                            let action = match &res {
                                Ok(()) => Action::VersionInstalled { slug: slug.clone() },
                                Err(e) => Action::VersionInstallFailed {
                                    slug: slug.clone(),
                                    error: e.to_string(),
                                },
                            };
                            let _ = tx2.send(action).await;
                            res.map_err(|e| anyhow::anyhow!("{e}"))
                        });
                    }
                }
            }

            Effect::DeleteInstance(slug) => {
                let paths = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match delete_instance(&paths, &slug).await {
                        Ok(()) => {
                            let _ = tx.send(Action::InstanceDeleted(slug)).await;
                        }
                        Err(e) => {
                            let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                        }
                    }
                });
            }

            Effect::RenameInstance { slug, new_name } => {
                let paths = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match rename_instance(&paths, &slug, &new_name).await {
                        Ok(m) => {
                            let _ = tx
                                .send(Action::InstanceRenamed {
                                    slug,
                                    new_name: m.display_name,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                        }
                    }
                });
            }

            Effect::CloneInstance {
                source_slug,
                new_name,
            } => {
                let paths = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match clone_instance(&paths, &source_slug, &new_name).await {
                        Ok(m) => {
                            let _ = tx
                                .send(Action::InstanceCloned {
                                    source_slug,
                                    new_slug: m.slug,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                        }
                    }
                });
            }

            Effect::SetGroup { slug, group } => {
                let paths = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match set_group(&paths, &slug, group).await {
                        Ok(_m) => match list_instances(&paths).await {
                            Ok(list) => {
                                let _ = tx.send(Action::InstancesLoaded(list)).await;
                            }
                            Err(e) => {
                                let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                            }
                        },
                        Err(e) => {
                            let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                        }
                    }
                });
            }

            Effect::LaunchInstance { slug, auth_ctx } => {
                let paths2 = paths.clone();
                let svc = Arc::clone(&account_service);
                let java_svc = Arc::clone(&java_service);
                let tx = action_tx.clone();
                let job_id = task_manager.next_job_id();
                let slug_for_task = slug.clone();
                task_manager.spawn_task(job_id, move |task_tx, token| async move {
                    let slug = slug_for_task;
                    // Store the token in state via LaunchJobStarted BEFORE blocking on launch.
                    let _ = tx
                        .send(Action::LaunchJobStarted {
                            slug: slug.clone(),
                            token: token.clone(),
                        })
                        .await;
                    let _ = tx
                        .send(Action::InstanceLaunched { slug: slug.clone() })
                        .await;
                    let res = launcher::service::launch_instance(
                        &paths2,
                        &slug,
                        auth_ctx,
                        Some(svc.as_ref()),
                        java_svc.as_ref(),
                        task_tx,
                        token,
                        job_id,
                    )
                    .await;
                    match res {
                        Ok(duration_ms) => {
                            let _ = tx.send(Action::InstanceExited { slug, duration_ms }).await;
                        }
                        Err(crate::error::AppError::Cancelled) => {
                            let _ = tx
                                .send(Action::InstanceExited {
                                    slug,
                                    duration_ms: 0,
                                })
                                .await;
                        }
                        Err(crate::error::AppError::LaunchFailed { code, message }) => {
                            let _ = tx
                                .send(Action::LaunchFailed {
                                    slug,
                                    error: format!("exit code {code}"),
                                    log_tail: message,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::LaunchFailed {
                                    slug,
                                    error: e.to_string(),
                                    log_tail: String::new(),
                                })
                                .await;
                        }
                    }
                    Ok(())
                });
            }

            Effect::KillProcess { slug } => {
                if let Some(token) = state.running_instances.get(&slug) {
                    token.cancel();
                }
            }

            Effect::StartDeviceCodeAuth => {
                let svc = Arc::clone(&account_service);
                let tx = action_tx.clone();
                let token = tokio_util::sync::CancellationToken::new();
                let _ = action_tx
                    .send(Action::AddAccountTokenCreated(token.clone()))
                    .await;
                let (event_tx, mut event_rx) = mpsc::channel::<AccountAuthEvent>(16);
                // Forwarder: AccountAuthEvent -> Action
                let tx_fwd = tx.clone();
                tokio::spawn(async move {
                    while let Some(ev) = event_rx.recv().await {
                        let action = match ev {
                            AccountAuthEvent::Started {
                                user_code,
                                verification_uri,
                                expires_in,
                            } => Action::AccountAuthStarted {
                                user_code,
                                verification_uri,
                                expires_at: std::time::Instant::now()
                                    + std::time::Duration::from_secs(expires_in),
                            },
                            AccountAuthEvent::Progress { stage } => {
                                Action::AccountAuthProgress { stage }
                            }
                        };
                        if tx_fwd.send(action).await.is_err() {
                            break;
                        }
                    }
                });
                tokio::spawn(async move {
                    match svc.start_device_code_auth(token, event_tx).await {
                        Ok(out) => {
                            let _ = tx
                                .send(Action::AccountAdded {
                                    account: out.account,
                                })
                                .await;
                        }
                        Err(crate::auth::AuthError::UserCancelled) => {
                            // Cancellation is expected -- silently return to AccountsList.
                            let _ = tx.send(Action::CloseAccounts).await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::AccountAuthFailed {
                                    reason: e.to_string(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::RemoveAccount { id } => {
                let svc = Arc::clone(&account_service);
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = svc.remove_account(&id).await {
                        let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                    }
                    if let Ok(list) = svc.list_accounts().await {
                        let _ = tx.send(Action::AccountsLoaded(list)).await;
                    }
                });
            }

            Effect::ActivateAccount { id } => {
                let svc = Arc::clone(&account_service);
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = svc.activate_account(&id).await {
                        let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                    }
                    if let Ok(list) = svc.list_accounts().await {
                        let _ = tx.send(Action::AccountsLoaded(list)).await;
                    }
                });
            }

            Effect::FetchAccounts => {
                let svc = Arc::clone(&account_service);
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.list_accounts().await {
                        Ok(list) => {
                            let _ = tx.send(Action::AccountsLoaded(list)).await;
                        }
                        Err(e) => {
                            let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                        }
                    }
                });
            }

            Effect::FetchSystemJavas { slug } => {
                let svc = Arc::clone(&java_service);
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    let system_javas = svc.list_system_javas().await;
                    let mut options = vec![JavaPickerRow::Auto];
                    for sj in system_javas {
                        options.push(JavaPickerRow::Detected(sj));
                    }
                    options.push(JavaPickerRow::Manual);
                    let _ = tx
                        .send(Action::JavaPickerOptionsLoaded { slug, options })
                        .await;
                });
            }

            Effect::SetJavaOverride { slug, override_id } => {
                let svc = Arc::clone(&java_service);
                let paths = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc
                        .set_override_for_instance(&paths, &slug, override_id)
                        .await
                    {
                        Ok(()) => {
                            let _ = tx.send(Action::JavaOverrideSet { slug }).await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::JavaOverrideFailed {
                                    slug,
                                    reason: e.to_string(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::FetchLoaderVersions { slug, loader_type } => {
                let svc = Arc::clone(&loader_service);
                let mc_version = state
                    .instances
                    .iter()
                    .find(|m| m.slug == slug)
                    .map(|m| m.mc_version_id.clone())
                    .unwrap_or_default();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.list_loader_versions(loader_type, &mc_version).await {
                        Ok(versions) => {
                            let _ = tx
                                .send(Action::LoaderVersionsLoaded {
                                    slug,
                                    loader: loader_type,
                                    versions,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::LoaderInstallFailed {
                                    slug,
                                    loader: loader_type,
                                    version: String::new(),
                                    error: e.to_string(),
                                    log_tail: String::new(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::InstallLoader {
                slug,
                loader_type,
                mc_version,
                loader_version,
            } => {
                let svc = Arc::clone(&loader_service);
                let java = Arc::clone(&java_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                let job_id = task_manager.next_job_id();
                let slug_for_task = slug.clone();
                let loader_version_for_task = loader_version.clone();

                // Forwarder: install_loader emits TaskEvent::Progress through `lt_tx`.
                // Phase 7 (D-02): filter [log-tail]-prefixed messages into
                // Action::LoaderInstallLogTail (updates log_tail without clobbering gauge);
                // non-prefixed messages flow into Action::LoaderInstallProgress as before.
                let (lt_tx, mut lt_rx) = mpsc::channel::<TaskEvent>(64);
                {
                    let tx_fwd = tx.clone();
                    let slug_fwd = slug.clone();
                    tokio::spawn(async move {
                        while let Some(evt) = lt_rx.recv().await {
                            if let TaskEvent::Progress { pct, msg, .. } = evt {
                                if let Some(tail) = msg.strip_prefix("[log-tail] ") {
                                    let _ = tx_fwd
                                        .send(Action::LoaderInstallLogTail {
                                            slug: slug_fwd.clone(),
                                            tail: tail.to_string(),
                                        })
                                        .await;
                                } else {
                                    let _ = tx_fwd
                                        .send(Action::LoaderInstallProgress {
                                            slug: slug_fwd.clone(),
                                            pct,
                                            step_label: msg,
                                            bytes_done: 0,
                                            bytes_total: 0,
                                        })
                                        .await;
                                }
                            }
                        }
                    });
                }

                task_manager.spawn_task(job_id, move |_task_tx, token| async move {
                    let slug = slug_for_task;
                    let _ = tx
                        .send(Action::LoaderInstallStarted {
                            slug: slug.clone(),
                            token: token.clone(),
                        })
                        .await;

                    // Phase 7 (D-06): resolve the JRE for the installer subprocess BEFORE
                    // calling install_loader. The vanilla version JSON must already be on
                    // disk (Phase 2/3 install_version) -- if not, surface a typed error.
                    let jre_path = match java
                        .resolve_jre_for_mc_version_install(&paths2, &mc_version)
                        .await
                    {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = tx
                                .send(Action::LoaderInstallFailed {
                                    slug,
                                    loader: loader_type,
                                    version: loader_version_for_task,
                                    error: format!("JRE resolution failed: {e}"),
                                    log_tail: String::new(),
                                })
                                .await;
                            return Ok(());
                        }
                    };

                    let res = svc
                        .install_loader(
                            &paths2,
                            &slug,
                            &mc_version,
                            loader_type,
                            &loader_version_for_task,
                            &jre_path,
                            lt_tx,
                            token,
                            job_id,
                        )
                        .await;

                    match res {
                        Ok(()) => {
                            let _ = tx.send(Action::LoaderInstalled { slug }).await;
                        }
                        Err(crate::loader::error::LoaderError::Cancelled) => {
                            // Treat cancellation as a clean completion for UI purposes:
                            // the CancelLoaderInstall handler in update() already moved
                            // the active view back to InstanceList.
                            let _ = tx.send(Action::LoaderInstalled { slug }).await;
                        }
                        Err(e) => {
                            // Phase 7 (LOAD-06): extract subprocess stderr from
                            // LoaderError::SubprocessExit to surface in the failure modal.
                            let log_tail = match &e {
                                crate::loader::error::LoaderError::SubprocessExit {
                                    tail, ..
                                } => tail.clone(),
                                _ => String::new(),
                            };
                            let _ = tx
                                .send(Action::LoaderInstallFailed {
                                    slug,
                                    loader: loader_type,
                                    version: loader_version_for_task,
                                    error: e.to_string(),
                                    log_tail,
                                })
                                .await;
                        }
                    }
                    Ok(())
                });
            }

            Effect::CancelLoaderInstall { slug: _ } => {
                // The token.cancel() already happened in update() -- this arm is a
                // no-op hook for symmetry with KillProcess. The install task
                // observes the cancellation token and returns LoaderError::Cancelled.
            }

            Effect::RemoveLoader { slug } => {
                let svc = Arc::clone(&loader_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.remove_loader(&paths2, &slug).await {
                        Ok(()) => {
                            let _ = tx.send(Action::LoaderInstalled { slug }).await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::LoaderInstallFailed {
                                    slug,
                                    loader: LoaderType::Fabric, // placeholder -- remove failure modal copy doesn't depend on type
                                    version: String::new(),
                                    error: e.to_string(),
                                    log_tail: String::new(),
                                })
                                .await;
                        }
                    }
                });
            }

            // Phase 8 (08-08): Modrinth integration effect arms -- wired below.
            // Install progress (`Effect::InstallModWithDeps`) flows through the
            // existing `download_pane` via `Action::Task(TaskEvent::Progress)`,
            // NOT a blocking install modal -- UI-SPEC §11 invariant.
            Effect::SearchModrinth {
                slug,
                query,
                mc,
                loader,
            } => {
                let svc = Arc::clone(&modrinth_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc
                        .search(
                            &query,
                            mc.as_deref(),
                            loader.as_ref(),
                            Some(&paths2),
                            Some(&slug),
                        )
                        .await
                    {
                        Ok(hits) => {
                            let _ = tx.send(Action::ModBrowserSearchLoaded { slug, hits }).await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, slug = %slug, "Modrinth search failed");
                            let _ = tx
                                .send(Action::ModBrowserSearchFailed {
                                    slug,
                                    message: e.to_string(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::FetchModDetail { slug, project_id } => {
                let svc = Arc::clone(&modrinth_service);
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.get_project(&project_id).await {
                        Ok(detail) => {
                            let _ = tx.send(Action::ModDetailLoaded { slug, detail }).await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                slug = %slug,
                                project_id = %project_id,
                                "Modrinth get_project failed",
                            );
                        }
                    }
                });
            }

            Effect::ListModVersions {
                slug,
                project_id,
                project_title: _,
                mc,
                loader,
            } => {
                let svc = Arc::clone(&modrinth_service);
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc
                        .list_versions(&project_id, mc.as_deref(), loader.as_ref())
                        .await
                    {
                        Ok(versions) => {
                            let _ = tx.send(Action::ModVersionsLoaded { slug, versions }).await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                slug = %slug,
                                project_id = %project_id,
                                "Modrinth list_versions failed",
                            );
                            // Surface as empty list -- the version-picker empty-state
                            // copy ("No versions match...") will render.
                            let _ = tx
                                .send(Action::ModVersionsLoaded {
                                    slug,
                                    versions: Vec::new(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::ResolveModDependencies {
                slug,
                project_id,
                project_title,
                version_id,
                version_label,
                mc,
                loader,
            } => {
                let svc = Arc::clone(&modrinth_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc
                        .resolve_dependencies(&paths2, &slug, &version_id, &mc, loader.as_ref())
                        .await
                    {
                        Ok(graph) => {
                            let _ = tx
                                .send(Action::ModDepsResolved {
                                    slug,
                                    project_id,
                                    project_title,
                                    version_id,
                                    version_label,
                                    graph: Box::new(graph),
                                })
                                .await;
                        }
                        Err(e) => {
                            // Surface dep-resolution failures via the install-failed
                            // modal -- same UX surface as a download error.
                            let _ = tx
                                .send(Action::ModInstallFailed {
                                    slug,
                                    mod_title: project_title,
                                    version_label,
                                    error: e.to_string(),
                                    log_tail: String::new(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::InstallModWithDeps {
                slug,
                project_slug,
                project_title,
                root_version,
                graph,
            } => {
                let svc = Arc::clone(&modrinth_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                let job_id = task_manager.next_job_id();

                // Forwarder: ModrinthService emits TaskEvent::Progress through `lt_tx`.
                // Translate each event into Action::Task so it flows through the
                // existing `state.active_jobs` → `download_pane` LineGauge -- NOT
                // a blocking install modal (UI-SPEC §11).
                let (lt_tx, mut lt_rx) = mpsc::channel::<TaskEvent>(64);
                {
                    let tx_fwd = tx.clone();
                    tokio::spawn(async move {
                        while let Some(evt) = lt_rx.recv().await {
                            // Only Progress events drive the download_pane LineGauge.
                            // Completed/Failed are signalled separately via the per-arm
                            // ModInstalled / ModInstallFailed actions below.
                            if let TaskEvent::Progress { id, pct, msg } = evt {
                                let _ = tx_fwd
                                    .send(Action::Task(TaskEvent::Progress { id, pct, msg }))
                                    .await;
                            }
                        }
                    });
                }

                let slug_for_task = slug.clone();
                let project_slug_for_task = project_slug.clone();
                let project_title_for_task = project_title.clone();
                let root_version_for_task = root_version.clone();
                let graph_for_task = graph.clone();
                task_manager.spawn_task(job_id, move |_task_tx, token| async move {
                    let slug = slug_for_task;
                    let _ = tx
                        .send(Action::ModInstallStarted {
                            slug: slug.clone(),
                            project_id: root_version_for_task.project_id.clone(),
                            token: token.clone(),
                        })
                        .await;

                    let res = svc
                        .install_mod_into_instance(
                            &paths2,
                            &slug,
                            &project_slug_for_task,
                            &project_title_for_task,
                            &root_version_for_task,
                            &graph_for_task,
                            lt_tx,
                            token,
                            job_id,
                        )
                        .await;

                    match res {
                        Ok(()) => {
                            let _ = tx
                                .send(Action::ModInstalled {
                                    slug,
                                    project_id: root_version_for_task.project_id.clone(),
                                })
                                .await;
                        }
                        Err(crate::mods::error::ModrinthError::Cancelled) => {
                            // Treat cancellation as a clean completion (Phase 6 precedent --
                            // run.rs lines 962-967 for LoaderInstall). The user already
                            // returned to ModBrowser via the cancel path.
                            let _ = tx
                                .send(Action::ModInstalled {
                                    slug,
                                    project_id: root_version_for_task.project_id.clone(),
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::ModInstallFailed {
                                    slug,
                                    mod_title: project_title_for_task,
                                    version_label: root_version_for_task.version_number.clone(),
                                    error: e.to_string(),
                                    log_tail: String::new(),
                                })
                                .await;
                        }
                    }
                    Ok(())
                });
            }

            Effect::ToggleModEnabledEff {
                slug,
                mod_id,
                want_enabled,
            } => {
                let svc = Arc::clone(&modrinth_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    let res = if want_enabled {
                        svc.enable_mod(&paths2, &slug, &mod_id).await
                    } else {
                        svc.disable_mod(&paths2, &slug, &mod_id).await
                    };
                    match res {
                        Ok(()) => {
                            let _ = tx
                                .send(Action::ModToggled {
                                    slug,
                                    mod_id,
                                    enabled: want_enabled,
                                })
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                slug = %slug,
                                mod_id = %mod_id,
                                "Modrinth toggle_mod_enabled failed",
                            );
                        }
                    }
                });
            }

            Effect::UninstallMod { slug, mod_id } => {
                let svc = Arc::clone(&modrinth_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.uninstall_mod(&paths2, &slug, &mod_id).await {
                        Ok(()) => {
                            let _ = tx.send(Action::ModUninstalled { slug, mod_id }).await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                slug = %slug,
                                mod_id = %mod_id,
                                "Modrinth uninstall_mod failed",
                            );
                        }
                    }
                });
            }

            Effect::FetchInstalledMods { slug } => {
                let svc = Arc::clone(&modrinth_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.list_installed_mods(&paths2, &slug).await {
                        Ok(mods) => {
                            let _ = tx.send(Action::InstalledModsLoaded { slug, mods }).await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                slug = %slug,
                                "Modrinth list_installed_mods failed",
                            );
                            let _ = tx
                                .send(Action::InstalledModsLoaded {
                                    slug,
                                    mods: Vec::new(),
                                })
                                .await;
                        }
                    }
                });
            }

            // Phase 9 (09-07): CurseForge effect arms -- LOCKED design with 4
            // SEPARATE arms (SearchCurseForge / FetchCfMod / ListCfFiles /
            // InstallCfMod). Mirrors Phase 8's separate-fetch-then-list pattern
            // (FetchModDetail + ListModVersions). Do NOT collapse into a
            // combined `OpenCfFilePicker` effect -- the design relies on the
            // Action ping-pong: CfBrowserOpenDetail → FetchCfMod →
            // CfBrowserDetailLoaded → ListCfFiles → CfFilePickerLoaded.
            //
            // Install progress (`Effect::InstallCfMod`) flows through the
            // existing `download_pane` via `Action::Task(TaskEvent::Progress)`,
            // NOT a blocking install modal -- UI-SPEC §11 invariant inherited
            // from Phase 8.
            Effect::SearchCurseForge {
                slug,
                query,
                mc,
                loader,
            } => {
                let svc = Arc::clone(&cf_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                let loader_info = synthetic_loader_info_from_cf_type(loader);
                tokio::spawn(async move {
                    match svc
                        .search(
                            &query,
                            mc.as_deref(),
                            loader_info.as_ref(),
                            Some(&paths2),
                            Some(&slug),
                        )
                        .await
                    {
                        Ok(hits) => {
                            let _ = tx.send(Action::CfBrowserSearchLoaded { slug, hits }).await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, slug = %slug, "CurseForge search failed");
                            let _ = tx
                                .send(Action::CfBrowserSearchFailed {
                                    slug,
                                    error: e.to_string(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::FetchCfMod { slug, mod_id } => {
                let svc = Arc::clone(&cf_service);
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.get_mod(mod_id).await {
                        Ok(detail) => {
                            let _ = tx
                                .send(Action::CfBrowserDetailLoaded { slug, detail })
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e, slug = %slug, mod_id, "CurseForge get_mod failed"
                            );
                            let _ = tx
                                .send(Action::CfBrowserSearchFailed {
                                    slug,
                                    error: e.to_string(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::ListCfFiles {
                slug,
                mod_id,
                mc,
                loader,
            } => {
                // Mirrors Phase 8 ListModVersions: the spawned task fetches BOTH
                // the project detail (needed for CfFilePickerLoaded.mod_detail)
                // AND the file list. One extra get_mod round-trip per file-picker
                // open (~50ms) -- acceptable for v1; deferred caching to v2.
                let svc = Arc::clone(&cf_service);
                let tx = action_tx.clone();
                let loader_info = synthetic_loader_info_from_cf_type(loader);
                tokio::spawn(async move {
                    let res: Result<_, crate::mods::curseforge::error::CurseForgeError> = async {
                        let detail = svc.get_mod(mod_id).await?;
                        let files = svc
                            .list_files(mod_id, mc.as_deref(), loader_info.as_ref())
                            .await?;
                        Ok::<_, crate::mods::curseforge::error::CurseForgeError>((detail, files))
                    }
                    .await;
                    match res {
                        Ok((detail, files)) => {
                            let _ = tx
                                .send(Action::CfFilePickerLoaded {
                                    slug,
                                    mod_detail: detail,
                                    files,
                                })
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e, slug = %slug, mod_id, "CurseForge list_files failed"
                            );
                            let _ = tx
                                .send(Action::CfBrowserSearchFailed {
                                    slug,
                                    error: e.to_string(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::InstallCfMod {
                slug,
                mod_detail,
                file,
            } => {
                // Verbatim per 09-RESEARCH.md §"Effect arm template" lines
                // 1163-1216. Mirrors Phase 8 InstallModWithDeps mpsc-forwarder
                // pattern, plus the FileNotDownloadable → CfModInstallFailed
                // mapping that carries `web_url: Some(...)` so the modal can
                // render the actionable browser link (MOD-04 load-bearing path).
                let svc = Arc::clone(&cf_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                let job_id = task_manager.next_job_id();

                // Forwarder: cf_service emits TaskEvent::Progress through
                // `lt_tx`. Translate each event into Action::Task so it flows
                // through the existing `state.active_jobs` → `download_pane`
                // LineGauge -- NOT a blocking install modal (UI-SPEC §11).
                let (lt_tx, mut lt_rx) = mpsc::channel::<TaskEvent>(64);
                {
                    let tx_fwd = tx.clone();
                    tokio::spawn(async move {
                        while let Some(evt) = lt_rx.recv().await {
                            if let TaskEvent::Progress { id, pct, msg } = evt {
                                let _ = tx_fwd
                                    .send(Action::Task(TaskEvent::Progress { id, pct, msg }))
                                    .await;
                            }
                        }
                    });
                }

                let slug_for_task = slug.clone();
                let mod_id = mod_detail.id;
                let file_id = file.id;
                task_manager.spawn_task(job_id, move |_task_tx, token| async move {
                    let slug = slug_for_task;
                    let _ = tx
                        .send(Action::CfModInstallStarted {
                            slug: slug.clone(),
                            mod_id,
                            file_id,
                            token: token.clone(),
                        })
                        .await;

                    let res = svc
                        .install_mod_into_instance(
                            &paths2,
                            &slug,
                            mod_detail.as_ref(),
                            file.as_ref(),
                            lt_tx,
                            token,
                            job_id,
                        )
                        .await;

                    match res {
                        Ok(()) => {
                            let _ = tx.send(Action::CfModInstalled { slug, mod_id }).await;
                        }
                        Err(crate::mods::curseforge::error::CurseForgeError::Cancelled) => {
                            // Silent -- the cancel path already pruned
                            // running_mod_jobs (if applicable). Phase 6/8 precedent.
                            let _ = tx.send(Action::CfModInstalled { slug, mod_id }).await;
                        }
                        Err(
                            crate::mods::curseforge::error::CurseForgeError::FileNotDownloadable {
                                web_url,
                                ..
                            },
                        ) => {
                            let _ = tx
                                .send(Action::CfModInstallFailed {
                                    slug,
                                    mod_title: mod_detail.name.clone(),
                                    file_label: file.display_name.clone(),
                                    error: "Author has disabled third-party downloads".into(),
                                    web_url: Some(web_url),
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::CfModInstallFailed {
                                    slug,
                                    mod_title: mod_detail.name.clone(),
                                    file_label: file.display_name.clone(),
                                    error: e.to_string(),
                                    web_url: None,
                                })
                                .await;
                        }
                    }
                    Ok(())
                });
            }

            // ── Phase 10 (10-06): Modpack import effect arms ──
            Effect::ImportModpack { mrpack_path } => {
                let svc = Arc::clone(&modpack_service);
                let mojang_arc = Arc::clone(&mojang);
                let loader_svc = Arc::clone(&loader_service);
                let java_svc = Arc::clone(&java_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                let job_id = task_manager.next_job_id();

                // Derive a human-readable modpack name from the file stem for use
                // in ModpackImportStarted (before the slug is assigned by create_instance).
                let modpack_name_for_started = mrpack_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("modpack")
                    .to_string();

                // mpsc forwarder: ModpackService emits TaskEvent::Progress through lt_tx.
                // Translate each event into Action::ModpackImportProgress for gauge updates.
                let (lt_tx, mut lt_rx) = mpsc::channel::<TaskEvent>(64);
                {
                    let tx_fwd = tx.clone();
                    tokio::spawn(async move {
                        while let Some(evt) = lt_rx.recv().await {
                            if let TaskEvent::Progress { pct, msg, .. } = evt {
                                // Slug is not known at the forwarder layer (it is assigned
                                // inside import_mrpack after create_instance). Use an empty
                                // slug; the ModpackImportProgress update arm matches on the
                                // active_view shape rather than on the slug.
                                let _ = tx_fwd
                                    .send(Action::ModpackImportProgress {
                                        slug: String::new(),
                                        pct,
                                        step_label: msg,
                                        bytes_done: 0,
                                        bytes_total: 0,
                                    })
                                    .await;
                            }
                        }
                    });
                }

                let mrpack_path_for_task = mrpack_path.clone();
                let modpack_name_for_task = modpack_name_for_started.clone();

                task_manager.spawn_task(job_id, move |_task_tx, token| async move {
                    // Dispatch ModpackImportStarted BEFORE calling import_mrpack so the
                    // progress modal opens immediately. The slug is a placeholder (empty)
                    // because the real slug is not known until import_mrpack returns.
                    let _ = tx
                        .send(Action::ModpackImportStarted {
                            slug: String::new(),
                            modpack_name: modpack_name_for_task.clone(),
                            token: token.clone(),
                        })
                        .await;

                    let res = svc
                        .import_mrpack(
                            &paths2,
                            &mrpack_path_for_task,
                            &mojang_arc,
                            &loader_svc,
                            &java_svc,
                            lt_tx,
                            token,
                            job_id,
                        )
                        .await;

                    match res {
                        Ok(manifest) => {
                            let _ = tx
                                .send(Action::ModpackImported {
                                    slug: manifest.slug,
                                })
                                .await;
                        }
                        Err(crate::modpack::error::ModpackError::Cancelled) => {
                            // HIGH-2 regression fix: dispatch ModpackImportCancelled
                            // (NOT ModpackImported{slug:""}, which would call remove("")
                            // -- a no-op against the real-slug HashMap key, leaving a
                            // stale CancellationToken in running_modpack_imports).
                            // The dedicated ModpackImportCancelled arm calls clear()
                            // regardless of which slug was assigned before cancel.
                            let _ = tx.send(Action::ModpackImportCancelled).await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::ModpackImportFailed {
                                    modpack_name: modpack_name_for_task,
                                    error: e.to_string(),
                                    log_tail: String::new(),
                                })
                                .await;
                        }
                    }
                    Ok(())
                });
            }

            Effect::CancelModpackImport => {
                // The token.cancel() already happened in update() -- this arm is a
                // no-op hook for symmetry with CancelLoaderInstall. The import task
                // observes the cancellation token and returns ModpackError::Cancelled;
                // service-side cleanup (remove_dir_all) ran in import_mrpack's outer arm.
            }

            // ── Phase 11 (11-04): Pack browser + install effect arms ─────────
            Effect::SearchPacks {
                slug,
                kind,
                query,
                mc,
            } => {
                let svc = Arc::clone(&pack_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc
                        .search(&query, kind, mc.as_deref(), Some(&paths2), Some(&slug))
                        .await
                    {
                        Ok(hits) => {
                            let _ = tx
                                .send(Action::PackBrowserSearchLoaded { slug, kind, hits })
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, %slug, ?kind, "PackService search failed");
                            let _ = tx
                                .send(Action::PackBrowserSearchFailed {
                                    slug,
                                    kind,
                                    message: e.to_string(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::FetchInstalledPacks { slug, kind } => {
                let svc = Arc::clone(&pack_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.list_installed(&paths2, &slug, kind).await {
                        Ok(packs) => {
                            let _ = tx
                                .send(Action::InstalledPacksLoaded { slug, kind, packs })
                                .await;
                        }
                        Err(e) => tracing::warn!(error = %e, %slug, ?kind, "list_installed failed"),
                    }
                });
            }

            Effect::DropInstallPack { slug, kind, path } => {
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                let token = tokio_util::sync::CancellationToken::new();
                let token2 = token.clone();
                tokio::spawn(async move {
                    match crate::packs::install::drop_pack_from_path(
                        &paths2, &slug, kind, &path, &token2,
                    )
                    .await
                    {
                        Ok(_outcome) => {
                            let _ = tx.send(Action::PackInstalled { slug, kind }).await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::PackDropFailed {
                                    slug,
                                    kind,
                                    error: e.to_string(),
                                })
                                .await;
                        }
                    }
                });
                // Suppress unused token warning -- it's a handle for future cancel support.
                let _ = token;
            }

            Effect::InstallPackFromModrinth {
                slug,
                kind,
                project_id,
                project_slug,
                project_title,
                version,
            } => {
                let svc = Arc::clone(&pack_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                let job_id = task_manager.next_job_id();

                // Progress forwarder (mirrors InstallModWithDeps pattern).
                let (lt_tx, mut lt_rx) = mpsc::channel::<crate::tasks::TaskEvent>(64);
                {
                    let tx_fwd = tx.clone();
                    tokio::spawn(async move {
                        while let Some(evt) = lt_rx.recv().await {
                            if let crate::tasks::TaskEvent::Progress { id, pct, msg } = evt {
                                let _ = tx_fwd
                                    .send(Action::Task(crate::tasks::TaskEvent::Progress {
                                        id,
                                        pct,
                                        msg,
                                    }))
                                    .await;
                            }
                        }
                    });
                }

                let slug_task = slug.clone();
                let project_id_task = project_id.clone();
                let project_title_task = project_title.clone();
                let version_label_task = version.version_number.clone();
                task_manager.spawn_task(job_id, move |_task_tx, token| async move {
                    let slug = slug_task;
                    match svc
                        .install_modrinth(
                            &paths2,
                            &slug,
                            kind,
                            &version,
                            &project_slug,
                            &project_id,
                            &project_title,
                            lt_tx,
                            token,
                            job_id,
                        )
                        .await
                    {
                        Ok(_row) => {
                            let _ = tx.send(Action::PackInstalled { slug, kind }).await;
                        }
                        Err(crate::packs::error::PackError::Cancelled) => {
                            tracing::info!(%slug, ?kind, %project_id_task, "pack install cancelled");
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::PackInstallFailed {
                                    slug,
                                    kind,
                                    pack_title: project_title_task,
                                    version_label: version_label_task,
                                    error: e.to_string(),
                                })
                                .await;
                        }
                    }
                    Ok(())
                });
            }

            Effect::FetchPackVersions {
                slug,
                kind,
                project_id,
                project_slug,
                project_title,
                mc,
            } => {
                // GAP-11-A: browser Enter-key auto-pick install chain.
                // 1. list_versions for the picked project (kind-aware, MC-filtered).
                // 2. pick first `is_latest_stable=true`; fallback `versions.first()`.
                // 3. get_version to materialise the full ModrinthVersion body.
                // 4. dispatch Action::AutoStartPackInstall, which fans out to
                //    Effect::InstallPackFromModrinth (handler above).
                let svc = Arc::clone(&pack_service);
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.list_versions(&project_id, mc.as_deref(), kind).await {
                        Ok(entries) => {
                            let entry_opt = entries
                                .iter()
                                .find(|v| v.is_latest_stable)
                                .cloned()
                                .or_else(|| entries.first().cloned());
                            let Some(entry) = entry_opt else {
                                let _ = tx
                                    .send(Action::PackVersionsFailed {
                                        slug,
                                        kind,
                                        project_id,
                                        message: "no compatible versions found".to_string(),
                                    })
                                    .await;
                                return;
                            };
                            match svc.get_version(&entry.version_id).await {
                                Ok(version) => {
                                    let _ = tx
                                        .send(Action::AutoStartPackInstall {
                                            slug,
                                            kind,
                                            project_id,
                                            project_slug,
                                            project_title,
                                            version,
                                        })
                                        .await;
                                }
                                Err(e) => {
                                    let _ = tx
                                        .send(Action::PackVersionsFailed {
                                            slug,
                                            kind,
                                            project_id,
                                            message: e.to_string(),
                                        })
                                        .await;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::PackVersionsFailed {
                                    slug,
                                    kind,
                                    project_id,
                                    message: e.to_string(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::TogglePackEnabledEff { slug, kind, mod_id } => {
                let svc = Arc::clone(&pack_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.toggle_pack_enabled(&paths2, &slug, &mod_id, kind).await {
                        Ok(new_enabled) => {
                            let _ = tx
                                .send(Action::PackToggled {
                                    slug,
                                    kind,
                                    mod_id,
                                    new_enabled,
                                })
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, %slug, ?kind, "toggle_pack_enabled failed");
                            let _ = tx
                                .send(Action::PackToggleFailed {
                                    slug,
                                    kind,
                                    error: e.to_string(),
                                })
                                .await;
                        }
                    }
                });
            }

            Effect::UninstallPack { slug, kind, mod_id } => {
                let svc = Arc::clone(&pack_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.uninstall_pack(&paths2, &slug, &mod_id, kind).await {
                        Ok(()) => {
                            let _ = tx
                                .send(Action::PackUninstalled { slug, kind, mod_id })
                                .await;
                        }
                        Err(e) => tracing::warn!(error = %e, %slug, ?kind, "uninstall_pack failed"),
                    }
                });
            }

            Effect::FetchIcon {
                source,
                project_id,
                url,
                purpose,
            } => {
                // No-op when icons are disabled or the service didn't build.
                let Some(svc) = state.icon_service.as_ref().map(Arc::clone) else {
                    continue;
                };
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                let target = purpose.target_rect();
                tokio::spawn(async move {
                    match svc
                        .fetch_and_decode(&paths2, source, &project_id, &url, target)
                        .await
                    {
                        Ok(()) => {
                            let _ = tx.send(Action::IconFetched { source, project_id }).await;
                        }
                        Err(e) => {
                            // Per Phase 13 D-05/D-06: blank icon area + warn.
                            // No user-facing toast, no retry.
                            tracing::warn!(
                                error = %e,
                                ?source,
                                %project_id,
                                ?purpose,
                                "icon fetch failed -- detail pane keeps blank icon area"
                            );
                        }
                    }
                });
            }

            // Phase 14: stub. Wired with real registry calls in 14-05/14-06.
            // For now, log + emit an empty `InstalledIconsResolved` so the
            // action machinery is exercised end-to-end and downstream tests
            // can verify the dispatch path without the actual API call.
            Effect::ResolveInstalledIcons { slug, source } => {
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    tracing::debug!(
                        %slug,
                        ?source,
                        "ResolveInstalledIcons stub -- real hydration lands in 14-05/14-06"
                    );
                    let _ = tx
                        .send(Action::InstalledIconsResolved {
                            slug,
                            source,
                            hits: Vec::new(),
                        })
                        .await;
                });
            }
        }
    }
}

/// Map a CurseForge `modLoaderType` integer back to a synthetic `LoaderInfo`
/// suitable for `cf_service.search` / `cf_service.list_files`. Only the `kind`
/// field is read by the underlying filter (`curseforge_loader_type`), so the
/// `version` / `version_id` strings can be empty.
///
/// Phase 9 (09-07): the `Effect::SearchCurseForge` / `Effect::ListCfFiles`
/// variants carry `Option<i32>` (already filtered through update arm), but the
/// service signatures take `Option<&LoaderInfo>`. The 1:1 inverse is exact
/// because `curseforge_loader_type` is total over the four valid loader kinds.
fn synthetic_loader_info_from_cf_type(cf_type: Option<i32>) -> Option<LoaderInfo> {
    use crate::domain::instance::ModloaderKind;
    let kind = match cf_type? {
        1 => ModloaderKind::Forge,
        4 => ModloaderKind::Fabric,
        5 => ModloaderKind::Quilt,
        6 => ModloaderKind::NeoForge,
        _ => return None,
    };
    Some(LoaderInfo {
        kind,
        version: String::new(),
        version_id: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_input_state(current: &str) -> AppState {
        AppState {
            active_view: ActiveView::CreateModal(CreateStep::NameInput {
                current: current.to_string(),
                error: None,
            }),
            ..AppState::default()
        }
    }

    /// GAP-8-C / 08.1-04: bracketed-paste payload from the terminal must map
    /// to a single `PasteName` action carrying the whole pasted string.
    /// Mirrors the mod_browser / cf_browser paste-event tests for consistency.
    #[test]
    fn paste_event_emits_paste_name_action() {
        let state = name_input_state("");
        let pasted = "MyInstance".to_string();
        let result = map_name_input_event(CtEvent::Paste(pasted.clone()), &state);
        match result {
            Some(Action::PasteName(got)) => assert_eq!(got, pasted),
            other => panic!("expected PasteName, got {other:?}"),
        }
    }
}
