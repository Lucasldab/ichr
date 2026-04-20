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

use super::app::{update, Action, ActiveView, AppState, CreateStep, Effect, VersionFilter};
use super::terminal::Tui;
use super::view::view;
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
                            execute_effects(effects, &paths, Arc::clone(&mojang), &task_manager, &action_tx).await;
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
                        execute_effects(effects, &paths, Arc::clone(&mojang), &task_manager, &action_tx).await;
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
    }
}

fn map_instance_list_event(ev: CtEvent, state: &AppState) -> Option<Action> {
    match ev {
        CtEvent::Key(KeyEvent { code: KeyCode::Char('q'), .. }) => Some(Action::Quit),
        CtEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) => Some(Action::Quit),
        CtEvent::Key(KeyEvent { code: KeyCode::Char('c'), .. }) => Some(Action::OpenCreateModal),
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
async fn execute_effects(
    effects: Vec<Effect>,
    paths: &AppPaths,
    mojang: Arc<MojangClient>,
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
        }
    }
}
