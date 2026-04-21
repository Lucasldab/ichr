//! Elm-style app state for Phase 2 + Phase 3.
//!
//! Phase 1 variants (Quit, Task, Tick) are preserved. Phase 2 adds view
//! navigation (OpenCreateModal, CloseModal, ConfirmDelete, ...), instance
//! lifecycle (SelectVersion, RenameInstance, CloneInstance), and background
//! completions (ManifestLoaded, InstancesLoaded, VersionInstalled, ...).
//! Phase 3 adds the launch lifecycle (LaunchInstance, LaunchJobStarted,
//! InstanceLaunched, InstanceExited, LaunchFailed, StopInstance), the
//! running_instances map (slug to CancellationToken), and LaunchFailedModal.

use std::collections::HashMap;

use tokio_util::sync::CancellationToken;

use crate::domain::platform::{Arch, OsName};
use crate::domain::InstanceManifest;
use crate::mojang::types::VersionEntry;
use crate::tasks::{JobId, TaskEvent};

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum VersionFilter {
    #[default]
    Releases,
    All,
}

#[derive(Debug, Clone)]
pub enum CreateStep {
    NameInput { current: String, error: Option<String> },
    VersionPicker { name: String, filter: VersionFilter, search: String, error: Option<String> },
}

#[derive(Debug, Clone)]
pub enum ActiveView {
    InstanceList { selected: usize },
    CreateModal(CreateStep),
    DeleteConfirm { slug: String, display_name: String },
    RenameInline { slug: String, current: String, original: String },
    /// Inline group editor (INST-06 — mirrors RenameInline).
    GroupInline { slug: String, buffer: String, original: Option<String> },
    /// Launch-failed modal — shown when Action::LaunchFailed is dispatched.
    /// Esc dismisses and returns to InstanceList.
    LaunchFailedModal { slug: String, error: String, log_tail: String },
}

impl Default for ActiveView {
    fn default() -> Self {
        ActiveView::InstanceList { selected: 0 }
    }
}

#[derive(Default)]
pub struct AppState {
    pub should_quit: bool,
    pub active_view: ActiveView,
    pub instances: Vec<InstanceManifest>,
    pub versions: Vec<VersionEntry>,
    pub versions_filter: VersionFilter,
    pub active_jobs: Vec<(JobId, u8, String)>,
    pub arch: Option<Arch>,
    pub os: Option<OsName>,
    pub last_error: Option<String>,
    /// Tracks slug → CancellationToken for every currently-running launch job.
    /// Populated by Action::LaunchJobStarted; cleared by InstanceExited / LaunchFailed.
    pub running_instances: HashMap<String, CancellationToken>,
}


#[derive(Debug, Clone)]
pub enum Action {
    // Phase 1
    Quit,
    Task(TaskEvent),
    Tick,

    // Navigation
    OpenCreateModal,
    CloseModal,
    OpenDeleteConfirm { slug: String, display_name: String },
    OpenRenameInline { slug: String, current: String },
    MoveSelection(isize),

    // Create flow
    TypeName(char),
    BackspaceName,
    SubmitInstanceName(String),
    SetVersionFilter(VersionFilter),
    TypeSearch(char),
    BackspaceSearch,
    ClearSearch,
    SelectVersion(String),

    // Rename / clone / delete
    TypeRename(char),
    BackspaceRename,
    SubmitRename(String),
    CloneSelected,
    ConfirmDelete,
    CancelDelete,

    // Group editor (INST-06 — mirrors rename pattern)
    OpenGroupInput { slug: String, current: String },
    TypeGroup(char),
    BackspaceGroup,
    SubmitGroup,
    CancelGroupInput,

    // Phase 3: launch lifecycle
    /// Dispatch when Enter is pressed on a non-running instance row.
    LaunchInstance { slug: String },
    /// Internal — emitted by execute_effects inside the spawned task body BEFORE
    /// calling launch_instance. Inserts the CancellationToken into running_instances.
    LaunchJobStarted { slug: String, token: CancellationToken },
    /// Tracing signal: launch_instance has started executing (after token is stored).
    InstanceLaunched { slug: String },
    /// Emitted when the game process exits (cleanly or via cancellation).
    InstanceExited { slug: String, duration_ms: u64 },
    /// Emitted when launch_instance returns a non-cancellation error.
    LaunchFailed { slug: String, error: String, log_tail: String },
    /// Dispatch when `s` is pressed on a running instance row.
    StopInstance { slug: String },

    // Background completions
    ManifestLoaded(Vec<VersionEntry>),
    InstancesLoaded(Vec<InstanceManifest>),
    VersionInstalled { slug: String },
    VersionInstallFailed { slug: String, error: String },
    InstanceDeleted(String),
    InstanceRenamed { slug: String, new_name: String },
    InstanceCloned { source_slug: String, new_slug: String },
    ServiceErrored(String),
}

/// Effects requested by `update()`. NOTE: there is deliberately NO
/// `SpawnVersionInstall` variant — creating an instance and installing its
/// version are performed by a single `CreateInstance` effect, handled
/// atomically by the runtime (02-07-03 `execute_effects`). See 02-07 plan
/// header for the rationale (checker blocker B2).
#[derive(Debug, Clone)]
pub enum Effect {
    Quit,
    FetchManifest,
    FetchInstances,
    DeleteInstance(String),
    RenameInstance { slug: String, new_name: String },
    CloneInstance { source_slug: String, new_name: String },
    CreateInstance {
        display_name: String,
        mc_version_id: String,
        version_url: String,
        version_sha1: String,
    },
    SetGroup { slug: String, group: Option<String> },
    /// Spawn a launch_instance task for the given slug.
    LaunchInstance { slug: String },
    /// Cancel the running launch task for the given slug.
    KillProcess { slug: String },
}

/// Apply an `Action`, mutate `state`, and return the side-effects to execute.
///
/// This is the single mutation point for all UI state. Views receive `&AppState`
/// (immutable) — no other code path mutates `AppState`.
pub fn update(state: &mut AppState, action: Action) -> Vec<Effect> {
    match action {
        Action::Quit => {
            state.should_quit = true;
            vec![Effect::Quit]
        }
        Action::Task(TaskEvent::Progress { id, pct, msg }) => {
            match state.active_jobs.iter_mut().find(|(jid, _, _)| *jid == id) {
                Some(slot) => {
                    slot.1 = pct;
                    slot.2 = msg;
                }
                None => state.active_jobs.push((id, pct, msg)),
            }
            vec![]
        }
        Action::Task(TaskEvent::Completed { id, result }) => {
            state.active_jobs.retain(|(jid, _, _)| *jid != id);
            if let Err(e) = result {
                state.last_error = Some(e);
            }
            vec![Effect::FetchInstances]
        }
        Action::Tick => vec![],

        Action::OpenCreateModal => {
            state.active_view = ActiveView::CreateModal(CreateStep::NameInput {
                current: String::new(),
                error: None,
            });
            vec![Effect::FetchManifest]
        }
        Action::CloseModal => {
            state.active_view = ActiveView::default();
            vec![]
        }
        Action::OpenDeleteConfirm { slug, display_name } => {
            state.active_view = ActiveView::DeleteConfirm { slug, display_name };
            vec![]
        }
        Action::OpenRenameInline { slug, current } => {
            state.active_view = ActiveView::RenameInline {
                slug,
                current: current.clone(),
                original: current,
            };
            vec![]
        }
        Action::MoveSelection(delta) => {
            if let ActiveView::InstanceList { selected } = &mut state.active_view {
                let len = state.instances.len() as isize;
                if len > 0 {
                    let new_idx = (*selected as isize + delta).rem_euclid(len);
                    *selected = new_idx as usize;
                }
            }
            vec![]
        }

        Action::TypeName(c) => {
            if let ActiveView::CreateModal(CreateStep::NameInput { current, error }) =
                &mut state.active_view
            {
                current.push(c);
                *error = None;
            }
            vec![]
        }
        Action::BackspaceName => {
            if let ActiveView::CreateModal(CreateStep::NameInput { current, .. }) =
                &mut state.active_view
            {
                current.pop();
            }
            vec![]
        }
        Action::SubmitInstanceName(name) => {
            let trimmed = name.trim().to_string();
            if trimmed.is_empty() {
                if let ActiveView::CreateModal(CreateStep::NameInput { error, .. }) =
                    &mut state.active_view
                {
                    *error = Some("name cannot be empty".into());
                }
                return vec![];
            }
            state.active_view = ActiveView::CreateModal(CreateStep::VersionPicker {
                name: trimmed,
                filter: state.versions_filter,
                search: String::new(),
                error: None,
            });
            vec![]
        }
        Action::SetVersionFilter(f) => {
            state.versions_filter = f;
            if let ActiveView::CreateModal(CreateStep::VersionPicker { filter, .. }) =
                &mut state.active_view
            {
                *filter = f;
            }
            vec![]
        }
        Action::TypeSearch(c) => {
            if let ActiveView::CreateModal(CreateStep::VersionPicker { search, .. }) =
                &mut state.active_view
            {
                search.push(c);
            }
            vec![]
        }
        Action::BackspaceSearch => {
            if let ActiveView::CreateModal(CreateStep::VersionPicker { search, .. }) =
                &mut state.active_view
            {
                search.pop();
            }
            vec![]
        }
        Action::ClearSearch => {
            if let ActiveView::CreateModal(CreateStep::VersionPicker { search, .. }) =
                &mut state.active_view
            {
                search.clear();
            }
            vec![]
        }
        Action::SelectVersion(id) => {
            // Look up the full VersionEntry to pass URL + SHA1 to the Effect.
            let entry = state.versions.iter().find(|v| v.id == id).cloned();
            let eff = match (&state.active_view, entry) {
                (ActiveView::CreateModal(CreateStep::VersionPicker { name, .. }), Some(v)) => {
                    Effect::CreateInstance {
                        display_name: name.clone(),
                        mc_version_id: v.id.clone(),
                        version_url: v.url.clone(),
                        version_sha1: v.sha1.clone(),
                    }
                }
                _ => return vec![],
            };
            state.active_view = ActiveView::default();
            vec![eff]
        }

        Action::TypeRename(c) => {
            if let ActiveView::RenameInline { current, .. } = &mut state.active_view {
                current.push(c);
            }
            vec![]
        }
        Action::BackspaceRename => {
            if let ActiveView::RenameInline { current, .. } = &mut state.active_view {
                current.pop();
            }
            vec![]
        }
        Action::SubmitRename(new_name) => {
            let trimmed = new_name.trim().to_string();
            if let ActiveView::RenameInline { slug, .. } = state.active_view.clone() {
                state.active_view = ActiveView::default();
                if trimmed.is_empty() {
                    return vec![];
                }
                return vec![Effect::RenameInstance { slug, new_name: trimmed }];
            }
            vec![]
        }
        Action::CloneSelected => {
            if let ActiveView::InstanceList { selected } = &state.active_view {
                if let Some(m) = state.instances.get(*selected) {
                    let new_name = format!("{} (Copy)", m.display_name);
                    return vec![Effect::CloneInstance {
                        source_slug: m.slug.clone(),
                        new_name,
                    }];
                }
            }
            vec![]
        }
        Action::ConfirmDelete => {
            if let ActiveView::DeleteConfirm { slug, .. } = state.active_view.clone() {
                state.active_view = ActiveView::default();
                return vec![Effect::DeleteInstance(slug)];
            }
            vec![]
        }
        Action::CancelDelete => {
            state.active_view = ActiveView::default();
            vec![]
        }

        Action::OpenGroupInput { slug, current } => {
            state.active_view = ActiveView::GroupInline {
                slug,
                buffer: current.clone(),
                original: if current.is_empty() { None } else { Some(current) },
            };
            vec![]
        }
        Action::TypeGroup(c) => {
            if let ActiveView::GroupInline { buffer, .. } = &mut state.active_view {
                buffer.push(c);
            }
            vec![]
        }
        Action::BackspaceGroup => {
            if let ActiveView::GroupInline { buffer, .. } = &mut state.active_view {
                buffer.pop();
            }
            vec![]
        }
        Action::SubmitGroup => {
            if let ActiveView::GroupInline { slug, buffer, .. } = state.active_view.clone() {
                state.active_view = ActiveView::default();
                let trimmed = buffer.trim().to_string();
                let group = if trimmed.is_empty() { None } else { Some(trimmed) };
                return vec![Effect::SetGroup { slug, group }];
            }
            vec![]
        }
        Action::CancelGroupInput => {
            state.active_view = ActiveView::default();
            vec![]
        }

        Action::ManifestLoaded(versions) => {
            state.versions = versions;
            vec![]
        }
        Action::InstancesLoaded(list) => {
            state.instances = list;
            vec![]
        }
        Action::VersionInstalled { .. }
        | Action::InstanceDeleted(_)
        | Action::InstanceRenamed { .. }
        | Action::InstanceCloned { .. } => vec![Effect::FetchInstances],
        Action::VersionInstallFailed { error, .. } => {
            state.last_error = Some(error);
            vec![Effect::FetchInstances]
        }
        Action::ServiceErrored(e) => {
            state.last_error = Some(e);
            vec![]
        }

        // Phase 3: launch lifecycle
        Action::LaunchInstance { slug } => {
            if state.running_instances.contains_key(&slug) {
                // Belt-and-suspenders: already running, no-op (T-03-05-01).
                vec![]
            } else {
                vec![Effect::LaunchInstance { slug }]
            }
        }
        Action::LaunchJobStarted { slug, token } => {
            state.running_instances.insert(slug, token);
            vec![]
        }
        Action::InstanceLaunched { slug: _ } => {
            // Tracing signal only — token already inserted via LaunchJobStarted.
            vec![]
        }
        Action::InstanceExited { slug, duration_ms: _ } => {
            state.running_instances.remove(&slug);
            vec![Effect::FetchInstances]
        }
        Action::LaunchFailed { slug, error, log_tail } => {
            state.running_instances.remove(&slug);
            state.active_view = ActiveView::LaunchFailedModal { slug, error, log_tail };
            vec![]
        }
        Action::StopInstance { slug } => {
            // Cancel and clear immediately so the running badge disappears on the
            // next render. The async launch task will later dispatch
            // Action::InstanceExited; the arm is idempotent (remove of an absent
            // slug is a no-op), so the double-path is safe.
            if let Some(token) = state.running_instances.remove(&slug) {
                token.cancel();
            }
            vec![Effect::KillProcess { slug }]
        }
    }
}
