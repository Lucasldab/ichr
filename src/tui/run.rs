//! Async event loop. Reads crossterm events, `TaskEvent`s, and a render tick,
//! dispatches them through `update`, executes any returned `Effect`s, and
//! redraws.
//!
//! Invariants:
//!   * No blocking I/O inside `.await` bodies here.
//!   * `action_tx.send(..)` in background arms uses `let _ =` — receiver
//!     being dropped is a valid shutdown signal.
//!   * `execute_effects` match is EXHAUSTIVE over Effect variants — no dead
//!     branches, no SpawnVersionInstall arm.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use ratatui::crossterm::event::{Event as CtEvent, EventStream, KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use tokio::time::interval;

use super::app::{update, Action, ActiveView, AppState, CreateStep, Effect, JavaPickerRow, VersionFilter};
use super::terminal::Tui;
use super::view::view;
use crate::auth::service::{AccountAuthEvent, AccountService};
use crate::java::service::JavaService;
use crate::loader::service::LoaderService;
use crate::loader::types::LoaderType;
use crate::launcher;
use crate::mojang::client::MojangClient;
use crate::mojang::types::VersionEntry;
use crate::persistence::paths::AppPaths;
use crate::services::{
    clone_instance, create_instance, delete_instance, list_instances, rename_instance, set_group,
};
use crate::install::install_version;
use crate::tasks::{TaskEvent, TaskManager, DEFAULT_MAX_CONCURRENT};

/// Apply the version filter + case-insensitive substring search. Always
/// excludes `old_beta` and `old_alpha` — those are out of Phase 2 scope per
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
pub async fn run(mut terminal: Tui) -> anyhow::Result<()> {
    // Resolve platform paths.
    let paths = AppPaths::resolve()
        .ok_or_else(|| anyhow::anyhow!("cannot resolve platform paths"))?;

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
    let java_service = Arc::new(JavaService::new().map_err(|e| anyhow::anyhow!("JavaService::new: {e}"))?);
    let loader_service = Arc::new(LoaderService::new().map_err(|e| anyhow::anyhow!("LoaderService::new: {e}"))?);

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

    let mut state = AppState::default();

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
                            execute_effects(effects, &state, &paths, Arc::clone(&mojang), Arc::clone(&account_service), Arc::clone(&java_service), Arc::clone(&loader_service), &task_manager, &action_tx).await;
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
                        execute_effects(effects, &state, &paths, Arc::clone(&mojang), Arc::clone(&account_service), Arc::clone(&java_service), Arc::clone(&loader_service), &task_manager, &action_tx).await;
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
    if let CtEvent::Key(KeyEvent { code: KeyCode::Char('c'), modifiers, .. }) = &ev {
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
        // Phase 6: loader modals — event mappers wired in 06-08.
        ActiveView::LoaderPickerModal { .. } => {
            super::views::loader_picker_modal::map_loader_picker_event(ev)
        }
        ActiveView::LoaderVersionPickerModal { .. } => {
            super::views::loader_version_picker_modal::map_loader_version_picker_event(ev)
        }
        ActiveView::LoaderInstallProgressModal { .. } => {
            super::views::loader_install_progress_modal::map_loader_install_progress_event(ev, state)
        }
        ActiveView::LoaderInstallFailedModal { .. } => {
            super::views::loader_install_failed_modal::map_loader_install_failed_event(ev)
        }
        ActiveView::LoaderSwitchConfirm { .. } => {
            super::views::loader_switch_confirm::map_loader_switch_confirm_event(ev)
        }
        // Phase 8 (08-07): event mappers for the new mod views land in 08-08.
        // Until then, swallow events for these states so the build is green.
        ActiveView::ModBrowser { .. }
        | ActiveView::ModVersionPickerModal { .. }
        | ActiveView::DepConfirmModal { .. }
        | ActiveView::InstalledModsList { .. }
        | ActiveView::UninstallModConfirm { .. }
        | ActiveView::ModInstallFailedModal { .. } => None,
    }
}

fn map_instance_list_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Char('q'), .. }) => Some(Action::Quit),
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::Quit),
        CtEvent::Key(KeyEvent { code: KeyCode::Char('c'), .. }) => Some(Action::OpenCreateModal),
        CtEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
            if let ActiveView::InstanceList { selected } = &state.active_view {
                if let Some(m) = state.instances.get(*selected) {
                    if state.running_instances.contains_key(&m.slug) {
                        // Already running — no-op (T-03-05-01 belt-and-suspenders).
                        return None;
                    }
                    return Some(Action::LaunchInstance { slug: m.slug.clone() });
                }
            }
            None
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char('s'), .. }) => {
            if let ActiveView::InstanceList { selected } = &state.active_view {
                if let Some(m) = state.instances.get(*selected) {
                    if state.running_instances.contains_key(&m.slug) {
                        return Some(Action::StopInstance { slug: m.slug.clone() });
                    }
                }
            }
            None
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char('r'), .. }) => {
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
        CtEvent::Key(KeyEvent { code: KeyCode::Char('x'), .. }) => Some(Action::CloneSelected),
        CtEvent::Key(KeyEvent { code: KeyCode::Char('d'), .. }) => {
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
        CtEvent::Key(KeyEvent { code: KeyCode::Char('g'), .. }) => {
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
        CtEvent::Key(KeyEvent { code: KeyCode::Char('A'), modifiers, .. })
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(Action::OpenAccounts)
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char('j'), .. }) => {
            // `j` on a non-running instance opens the Java picker.
            // On a running instance it is a no-op (can't change java mid-game).
            if let ActiveView::InstanceList { selected } = &state.active_view {
                if let Some(m) = state.instances.get(*selected) {
                    if !state.running_instances.contains_key(&m.slug) {
                        return Some(Action::OpenJavaPicker { slug: m.slug.clone() });
                    }
                }
            }
            None
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char('L'), modifiers, .. })
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            // `L` (uppercase) opens the loader picker for a non-running instance
            // with no install already in flight (T-06-20).
            if let ActiveView::InstanceList { selected } = &state.active_view {
                if let Some(m) = state.instances.get(*selected) {
                    if !state.running_instances.contains_key(&m.slug)
                        && !state.running_loader_installs.contains_key(&m.slug)
                    {
                        return Some(Action::OpenLoaderPicker { slug: m.slug.clone() });
                    }
                }
            }
            None
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Down, .. }) => Some(Action::MoveSelection(1)),
        CtEvent::Key(KeyEvent { code: KeyCode::Up, .. }) => Some(Action::MoveSelection(-1)),
        CtEvent::Key(KeyEvent { code: KeyCode::Char('k'), .. }) => Some(Action::MoveSelection(-1)),
        _ => None,
    }
}

fn map_accounts_list_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::CloseAccounts),
        CtEvent::Key(KeyEvent { code: KeyCode::Char('a'), .. }) => Some(Action::AddAccount),
        CtEvent::Key(KeyEvent { code: KeyCode::Char('x'), .. }) => {
            if let ActiveView::AccountsList { selected } = &state.active_view {
                if let Some(account) = state.accounts.get(*selected) {
                    return Some(Action::RemoveAccount { id: account.id.clone() });
                }
            }
            None
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
            if let ActiveView::AccountsList { selected } = &state.active_view {
                if let Some(account) = state.accounts.get(*selected) {
                    return Some(Action::ActivateAccount { id: account.id.clone() });
                }
            }
            None
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Down, .. })
        | CtEvent::Key(KeyEvent { code: KeyCode::Char('j'), .. }) => {
            Some(Action::MoveSelection(1))
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Up, .. })
        | CtEvent::Key(KeyEvent { code: KeyCode::Char('k'), .. }) => {
            Some(Action::MoveSelection(-1))
        }
        _ => None,
    }
}

fn map_add_account_device_code_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::CancelAddAccount),
        _ => None,
    }
}

fn map_account_auth_failed_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::CloseModal),
        _ => None,
    }
}

fn map_launch_failed_modal_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::CloseModal),
        _ => None,
    }
}

fn map_name_input_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::CloseModal),
        CtEvent::Key(KeyEvent { code: KeyCode::Backspace, .. }) => Some(Action::BackspaceName),
        CtEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
            if let ActiveView::CreateModal(CreateStep::NameInput { current, .. }) =
                &state.active_view
            {
                return Some(Action::SubmitInstanceName(current.clone()));
            }
            None
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char(c), modifiers, .. })
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(Action::TypeName(c))
        }
        _ => None,
    }
}

fn map_version_picker_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::CloseModal),
        CtEvent::Key(KeyEvent { code: KeyCode::Char('t'), .. }) => {
            let new_filter = match state.versions_filter {
                VersionFilter::Releases => VersionFilter::All,
                VersionFilter::All => VersionFilter::Releases,
            };
            Some(Action::SetVersionFilter(new_filter))
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Backspace, .. }) => Some(Action::BackspaceSearch),
        CtEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
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
        CtEvent::Key(KeyEvent { code: KeyCode::Char(c), modifiers, .. })
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(Action::TypeSearch(c))
        }
        _ => None,
    }
}

fn map_delete_confirm_event(ev: CtEvent) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Char('y'), .. })
        | CtEvent::Key(KeyEvent { code: KeyCode::Char('Y'), .. }) => Some(Action::ConfirmDelete),
        CtEvent::Key(_) => Some(Action::CancelDelete),
        _ => None,
    }
}

fn map_rename_inline_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::CloseModal),
        CtEvent::Key(KeyEvent { code: KeyCode::Backspace, .. }) => Some(Action::BackspaceRename),
        CtEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
            if let ActiveView::RenameInline { current, .. } = &state.active_view {
                return Some(Action::SubmitRename(current.clone()));
            }
            None
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char(c), modifiers, .. })
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(Action::TypeRename(c))
        }
        _ => None,
    }
}

fn map_group_inline_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::CancelGroupInput),
        CtEvent::Key(KeyEvent { code: KeyCode::Backspace, .. }) => Some(Action::BackspaceGroup),
        CtEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
            if matches!(state.active_view, ActiveView::GroupInline { .. }) {
                return Some(Action::SubmitGroup);
            }
            None
        }
        CtEvent::Key(KeyEvent { code: KeyCode::Char(c), modifiers, .. })
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(Action::TypeGroup(c))
        }
        _ => None,
    }
}

/// Execute declarative side-effects. Keeps `update` pure.
/// Match is exhaustive — no SpawnVersionInstall arm, no dead-code branches.
///
/// `state` is the just-updated state (after `update()` ran) — read-only.
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
    task_manager: &TaskManager,
    action_tx: &mpsc::Sender<Action>,
) {
    for eff in effects {
        match eff {
            Effect::Quit => {
                // No additional work — should_quit flag drives loop exit.
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

            Effect::CreateInstance { display_name, mc_version_id, version_url, version_sha1 } => {
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
                        task_manager.spawn_task(
                            job_id,
                            move |task_tx, token| async move {
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
                            },
                        );
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

            Effect::CloneInstance { source_slug, new_name } => {
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
                        Ok(_m) => {
                            match list_instances(&paths).await {
                                Ok(list) => {
                                    let _ = tx.send(Action::InstancesLoaded(list)).await;
                                }
                                Err(e) => {
                                    let _ = tx.send(Action::ServiceErrored(e.to_string())).await;
                                }
                            }
                        }
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
                        .send(Action::LaunchJobStarted { slug: slug.clone(), token: token.clone() })
                        .await;
                    let _ = tx.send(Action::InstanceLaunched { slug: slug.clone() }).await;
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
                                .send(Action::InstanceExited { slug, duration_ms: 0 })
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
                            AccountAuthEvent::Started { user_code, verification_uri, expires_in } => {
                                Action::AccountAuthStarted {
                                    user_code,
                                    verification_uri,
                                    expires_at: std::time::Instant::now()
                                        + std::time::Duration::from_secs(expires_in),
                                }
                            }
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
                            let _ = tx.send(Action::AccountAdded { account: out.account }).await;
                        }
                        Err(crate::auth::AuthError::UserCancelled) => {
                            // Cancellation is expected — silently return to AccountsList.
                            let _ = tx.send(Action::CloseAccounts).await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::AccountAuthFailed { reason: e.to_string() })
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
                    let _ = tx.send(Action::JavaPickerOptionsLoaded { slug, options }).await;
                });
            }

            Effect::SetJavaOverride { slug, override_id } => {
                let svc = Arc::clone(&java_service);
                let paths = paths.clone();
                let tx = action_tx.clone();
                tokio::spawn(async move {
                    match svc.set_override_for_instance(&paths, &slug, override_id).await {
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
                                .send(Action::LoaderVersionsLoaded { slug, loader: loader_type, versions })
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

            Effect::InstallLoader { slug, loader_type, mc_version, loader_version } => {
                let svc = Arc::clone(&loader_service);
                let paths2 = paths.clone();
                let tx = action_tx.clone();
                let job_id = task_manager.next_job_id();
                let slug_for_task = slug.clone();
                let loader_version_for_task = loader_version.clone();

                // Forwarder: install_loader emits TaskEvent::Progress through `lt_tx`.
                // We translate each Progress into Action::LoaderInstallProgress.
                let (lt_tx, mut lt_rx) = mpsc::channel::<TaskEvent>(64);
                {
                    let tx_fwd = tx.clone();
                    let slug_fwd = slug.clone();
                    tokio::spawn(async move {
                        while let Some(evt) = lt_rx.recv().await {
                            if let TaskEvent::Progress { pct, msg, .. } = evt {
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
                    });
                }

                task_manager.spawn_task(job_id, move |_task_tx, token| async move {
                    let slug = slug_for_task;
                    let _ = tx
                        .send(Action::LoaderInstallStarted { slug: slug.clone(), token: token.clone() })
                        .await;

                    let res = svc
                        .install_loader(
                            &paths2,
                            &slug,
                            &mc_version,
                            loader_type,
                            &loader_version_for_task,
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
                            let _ = tx
                                .send(Action::LoaderInstallFailed {
                                    slug,
                                    loader: loader_type,
                                    version: loader_version_for_task,
                                    error: e.to_string(),
                                    log_tail: String::new(),
                                })
                                .await;
                        }
                    }
                    Ok(())
                });
            }

            Effect::CancelLoaderInstall { slug: _ } => {
                // The token.cancel() already happened in update() — this arm is a
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
                                    loader: LoaderType::Fabric, // placeholder — remove failure modal copy doesn't depend on type
                                    version: String::new(),
                                    error: e.to_string(),
                                    log_tail: String::new(),
                                })
                                .await;
                        }
                    }
                });
            }

            // Phase 8 (08-07): Modrinth integration effects are declared up-front
            // so the state machine + tui_smoke tests can land independently of the
            // run-loop wiring. The spawn arms below are scaffold no-ops; 08-08
            // replaces them with real ModrinthService calls (HTTP + ledger I/O).
            Effect::SearchModrinth { .. }
            | Effect::FetchModDetail { .. }
            | Effect::ListModVersions { .. }
            | Effect::ResolveModDependencies { .. }
            | Effect::InstallModWithDeps { .. }
            | Effect::ToggleModEnabledEff { .. }
            | Effect::UninstallMod { .. }
            | Effect::FetchInstalledMods { .. } => {
                // Scaffold no-op — wired in 08-08.
            }
        }
    }
}
