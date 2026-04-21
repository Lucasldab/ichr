//! Elm-style app state for Phase 2 + Phase 3 + Phase 4.
//!
//! Phase 1 variants (Quit, Task, Tick) are preserved. Phase 2 adds view
//! navigation (OpenCreateModal, CloseModal, ConfirmDelete, ...), instance
//! lifecycle (SelectVersion, RenameInstance, CloneInstance), and background
//! completions (ManifestLoaded, InstancesLoaded, VersionInstalled, ...).
//! Phase 3 adds the launch lifecycle (LaunchInstance, LaunchJobStarted,
//! InstanceLaunched, InstanceExited, LaunchFailed, StopInstance), the
//! running_instances map (slug to CancellationToken), and LaunchFailedModal.
//! Phase 4 adds Microsoft account management (AccountsList, AddAccountDeviceCode,
//! AccountAuthFailed views) and the MSA launch integration via AuthContext.

use std::collections::HashMap;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::auth::{Account, AuthContext};
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
    /// AUTH-06 account list view. Entered via `A` from InstanceList.
    AccountsList { selected: usize },
    /// AUTH-01 device-code modal. `expires_at` drives the countdown;
    /// the render loop recomputes "seconds remaining" each frame.
    AddAccountDeviceCode {
        user_code: String,
        verification_uri: String,
        expires_at: Instant,
        stage: String,
    },
    /// AUTH-02 error modal (e.g., "No Xbox profile — visit xbox.com/profile").
    AccountAuthFailed { reason: String },
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
    /// Persisted list of Microsoft accounts.
    pub accounts: Vec<Account>,
    /// id of the currently-active account, if any. Drives whether
    /// Effect::LaunchInstance builds AuthContext::Msa or Offline.
    pub active_account_id: Option<String>,
    /// Cancellation token for the currently-active
    /// start_device_code_auth job. Cleared on Completion/Failure/Cancel.
    pub add_account_cancel: Option<CancellationToken>,
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

    // Phase 4: accounts
    OpenAccounts,
    CloseAccounts,
    AccountsLoaded(Vec<Account>),
    AddAccount,
    AccountAuthStarted { user_code: String, verification_uri: String, expires_at: Instant },
    AccountAuthProgress { stage: String },
    AccountAdded { account: Account },
    AccountAuthFailed { reason: String },
    RemoveAccount { id: String },
    ActivateAccount { id: String },
    CancelAddAccount,
    /// Internal — stores the CancellationToken created by execute_effects for
    /// the device-code auth task into state.add_account_cancel.
    AddAccountTokenCreated(CancellationToken),

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
    /// Spawn a launch_instance task. auth_ctx built by update() from
    /// state.active_account_id + instance display_name.
    LaunchInstance { slug: String, auth_ctx: AuthContext },
    /// Cancel the running launch task for the given slug.
    KillProcess { slug: String },
    /// Spawn the device-code auth task (AccountService::start_device_code_auth).
    StartDeviceCodeAuth,
    /// Remove account via AccountService::remove_account, then reload list.
    RemoveAccount { id: String },
    /// Activate account via AccountService::activate_account, then reload list.
    ActivateAccount { id: String },
    /// Reload the account list from store (AccountService::list_accounts).
    FetchAccounts,
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
            match &mut state.active_view {
                ActiveView::InstanceList { selected } => {
                    let len = state.instances.len() as isize;
                    if len > 0 {
                        let new_idx = (*selected as isize + delta).rem_euclid(len);
                        *selected = new_idx as usize;
                    }
                }
                ActiveView::AccountsList { selected } => {
                    let len = state.accounts.len() as isize;
                    if len > 0 {
                        let new_idx = (*selected as isize + delta).rem_euclid(len);
                        *selected = new_idx as usize;
                    }
                }
                _ => {}
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

        // Phase 4: accounts
        Action::OpenAccounts => {
            state.active_view = ActiveView::AccountsList { selected: 0 };
            vec![Effect::FetchAccounts]
        }
        Action::CloseAccounts => {
            if let Some(tok) = state.add_account_cancel.take() {
                tok.cancel();
            }
            state.active_view = ActiveView::default();
            vec![]
        }
        Action::AccountsLoaded(list) => {
            state.active_account_id = list
                .iter()
                .find(|a| a.is_active)
                .map(|a| a.id.clone());
            state.accounts = list;
            vec![]
        }
        Action::AddAccount => {
            vec![Effect::StartDeviceCodeAuth]
        }
        Action::AccountAuthStarted { user_code, verification_uri, expires_at } => {
            state.active_view = ActiveView::AddAccountDeviceCode {
                user_code,
                verification_uri,
                expires_at,
                stage: "waiting for user".into(),
            };
            vec![]
        }
        Action::AccountAuthProgress { stage } => {
            if let ActiveView::AddAccountDeviceCode { stage: s, .. } = &mut state.active_view {
                *s = stage;
            }
            vec![]
        }
        Action::AccountAdded { account } => {
            state.add_account_cancel = None;
            if account.is_active {
                state.active_account_id = Some(account.id.clone());
            }
            if let Some(existing) = state.accounts.iter_mut().find(|a| a.id == account.id) {
                *existing = account;
            } else {
                state.accounts.push(account);
            }
            state.active_view = ActiveView::AccountsList { selected: 0 };
            vec![Effect::FetchAccounts]
        }
        Action::AccountAuthFailed { reason } => {
            state.add_account_cancel = None;
            state.active_view = ActiveView::AccountAuthFailed { reason };
            vec![]
        }
        Action::RemoveAccount { id } => {
            state.accounts.retain(|a| a.id != id);
            if state.active_account_id.as_deref() == Some(id.as_str()) {
                state.active_account_id = None;
            }
            vec![Effect::RemoveAccount { id }]
        }
        Action::ActivateAccount { id } => {
            for a in state.accounts.iter_mut() {
                a.is_active = a.id == id;
            }
            state.active_account_id = Some(id.clone());
            vec![Effect::ActivateAccount { id }]
        }
        Action::CancelAddAccount => {
            if let Some(tok) = state.add_account_cancel.take() {
                tok.cancel();
            }
            state.active_view = ActiveView::AccountsList { selected: 0 };
            vec![]
        }
        Action::AddAccountTokenCreated(token) => {
            state.add_account_cancel = Some(token);
            vec![]
        }

        // Phase 3: launch lifecycle
        Action::LaunchInstance { slug } => {
            if state.running_instances.contains_key(&slug) {
                // Belt-and-suspenders: already running, no-op (T-03-05-01).
                vec![]
            } else {
                let display_name = state
                    .instances
                    .iter()
                    .find(|m| m.slug == slug)
                    .map(|m| m.display_name.clone())
                    .unwrap_or_else(|| slug.clone());
                let auth_ctx = if let Some(aid) = state.active_account_id.clone() {
                    AuthContext::Msa { account_id: aid }
                } else {
                    AuthContext::Offline { username: display_name }
                };
                vec![Effect::LaunchInstance { slug, auth_ctx }]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::StorageBackend;

    fn sample_account(id: &str, active: bool) -> Account {
        Account {
            id: id.into(),
            mc_username: format!("P{id}"),
            mc_uuid: format!("{id:0>8}-0000-4000-8000-000000000000"),
            mc_token_expires_at: 0,
            msa_token_expires_at: 0,
            added_at: 0,
            last_refreshed_at: 0,
            is_active: active,
            storage: StorageBackend::EncryptedFile,
        }
    }

    #[test]
    fn test_open_accounts_fetches_list() {
        let mut state = AppState::default();
        let effects = update(&mut state, Action::OpenAccounts);
        assert!(matches!(state.active_view, ActiveView::AccountsList { selected: 0 }));
        assert!(matches!(effects.as_slice(), [Effect::FetchAccounts]));
    }

    #[test]
    fn test_accounts_loaded_derives_active_id() {
        let mut state = AppState::default();
        let list = vec![sample_account("A", false), sample_account("B", true)];
        let _ = update(&mut state, Action::AccountsLoaded(list));
        assert_eq!(state.active_account_id.as_deref(), Some("B"));
        assert_eq!(state.accounts.len(), 2);
    }

    #[test]
    fn test_activate_account_sets_active_id_exclusively() {
        let mut state = AppState::default();
        state.accounts = vec![sample_account("A", true), sample_account("B", false)];
        state.active_account_id = Some("A".into());
        let effects = update(&mut state, Action::ActivateAccount { id: "B".into() });
        assert_eq!(state.active_account_id.as_deref(), Some("B"));
        assert!(state.accounts.iter().find(|a| a.id == "B").unwrap().is_active);
        assert!(!state.accounts.iter().find(|a| a.id == "A").unwrap().is_active);
        assert!(matches!(effects.as_slice(), [Effect::ActivateAccount { .. }]));
    }

    #[test]
    fn test_remove_active_account_clears_active_id() {
        let mut state = AppState::default();
        state.accounts = vec![sample_account("A", true)];
        state.active_account_id = Some("A".into());
        let _ = update(&mut state, Action::RemoveAccount { id: "A".into() });
        assert!(state.active_account_id.is_none());
        assert!(state.accounts.is_empty());
    }

    #[test]
    fn test_launch_instance_with_active_account_builds_msa_context() {
        let mut state = AppState::default();
        state.instances.push(crate::domain::InstanceManifest::new(
            "s".into(),
            "s".into(),
            "1.21.4".into(),
        ));
        state.active_account_id = Some("id-1".into());
        let effects = update(&mut state, Action::LaunchInstance { slug: "s".into() });
        match effects.as_slice() {
            [Effect::LaunchInstance { auth_ctx: AuthContext::Msa { account_id }, .. }] => {
                assert_eq!(account_id, "id-1");
            }
            other => panic!("expected Msa LaunchInstance; got {other:?}"),
        }
    }

    #[test]
    fn test_launch_instance_without_active_account_builds_offline_context() {
        let mut state = AppState::default();
        // new(display_name, slug, mc_version_id)
        let m = crate::domain::InstanceManifest::new("Pretty".into(), "s".into(), "1.21.4".into());
        state.instances.push(m);
        assert!(state.active_account_id.is_none());
        let effects = update(&mut state, Action::LaunchInstance { slug: "s".into() });
        match effects.as_slice() {
            [Effect::LaunchInstance { auth_ctx: AuthContext::Offline { username }, .. }] => {
                assert_eq!(username, "Pretty");
            }
            other => panic!("expected Offline LaunchInstance; got {other:?}"),
        }
    }

    #[test]
    fn test_account_auth_started_transitions_to_modal() {
        let mut state = AppState::default();
        let _ = update(&mut state, Action::AccountAuthStarted {
            user_code: "ABCD".into(),
            verification_uri: "https://ms/link".into(),
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(900),
        });
        assert!(matches!(state.active_view, ActiveView::AddAccountDeviceCode { .. }));
    }

    #[test]
    fn test_account_auth_failed_transitions_to_failed_modal() {
        let mut state = AppState::default();
        let _ = update(&mut state, Action::AccountAuthFailed { reason: "no license".into() });
        match &state.active_view {
            ActiveView::AccountAuthFailed { reason } => assert_eq!(reason, "no license"),
            other => panic!("expected AccountAuthFailed modal; got {other:?}"),
        }
    }

    #[test]
    fn test_cancel_add_account_cancels_token_and_returns_to_list() {
        let mut state = AppState::default();
        let t = CancellationToken::new();
        state.add_account_cancel = Some(t.clone());
        let _ = update(&mut state, Action::CancelAddAccount);
        assert!(t.is_cancelled());
        assert!(state.add_account_cancel.is_none());
        assert!(matches!(state.active_view, ActiveView::AccountsList { selected: 0 }));
    }
}
