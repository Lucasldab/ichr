//! Smoke test for the TUI reducer. Does NOT start the event loop (that requires
//! a real terminal); instead exercises `update()` directly, which is the only
//! place state mutation happens.

use mineltui::mojang::types::VersionEntry;
use mineltui::tasks::{JobId, TaskEvent};
use mineltui::tui::{update, Action, ActiveView, AppState, CreateStep, Effect, VersionFilter};

// ---- Phase 1 tests (preserved) ---------------------------------------------

#[test]
fn quit_action_sets_should_quit_and_emits_quit_effect() {
    let mut state = AppState::default();
    assert!(!state.should_quit);

    let effects = update(&mut state, Action::Quit);
    assert!(state.should_quit);
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], Effect::Quit));
}

#[test]
fn task_progress_upserts_active_jobs() {
    let mut state = AppState::default();
    let id = JobId(7);

    let _ = update(
        &mut state,
        Action::Task(TaskEvent::Progress { id, pct: 10, msg: "starting".into() }),
    );
    assert_eq!(state.active_jobs.len(), 1);
    assert_eq!(state.active_jobs[0].1, 10);

    let _ = update(
        &mut state,
        Action::Task(TaskEvent::Progress { id, pct: 50, msg: "halfway".into() }),
    );
    assert_eq!(state.active_jobs.len(), 1, "should upsert, not append");
    assert_eq!(state.active_jobs[0].1, 50);
    assert_eq!(state.active_jobs[0].2, "halfway");
}

#[test]
fn task_completed_removes_from_active_jobs() {
    let mut state = AppState::default();
    let id = JobId(3);

    let _ = update(
        &mut state,
        Action::Task(TaskEvent::Progress { id, pct: 25, msg: "running".into() }),
    );
    assert_eq!(state.active_jobs.len(), 1);

    let _ = update(
        &mut state,
        Action::Task(TaskEvent::Completed { id, result: Ok(()) }),
    );
    assert!(state.active_jobs.is_empty());
}

#[test]
fn tick_action_does_not_mutate_state() {
    let mut state = AppState::default();
    let effects = update(&mut state, Action::Tick);
    assert!(!state.should_quit);
    assert!(state.active_jobs.is_empty());
    assert!(effects.is_empty());
}

// ---- Phase 2 tests (Task 2-07-01b) -----------------------------------------

#[test]
fn test_open_create_modal_sets_active_view() {
    let mut state = AppState::default();
    let _effects = update(&mut state, Action::OpenCreateModal);
    assert!(matches!(
        state.active_view,
        ActiveView::CreateModal(CreateStep::NameInput { .. })
    ));
}

#[test]
fn test_close_modal_returns_to_instance_list() {
    let mut state = AppState {
        active_view: ActiveView::CreateModal(CreateStep::NameInput {
            current: "hi".into(),
            error: None,
        }),
        ..AppState::default()
    };
    let _effects = update(&mut state, Action::CloseModal);
    assert!(matches!(state.active_view, ActiveView::InstanceList { .. }));
}

#[test]
fn test_submit_name_advances_to_version_picker() {
    let mut state = AppState {
        active_view: ActiveView::CreateModal(CreateStep::NameInput {
            current: String::new(),
            error: None,
        }),
        ..AppState::default()
    };
    let _effects = update(&mut state, Action::SubmitInstanceName("Test".into()));
    assert!(matches!(
        &state.active_view,
        ActiveView::CreateModal(CreateStep::VersionPicker { name, .. }) if name == "Test"
    ));
}

#[test]
fn test_submit_empty_name_stays_on_name_input_with_error() {
    let mut state = AppState {
        active_view: ActiveView::CreateModal(CreateStep::NameInput {
            current: String::new(),
            error: None,
        }),
        ..AppState::default()
    };
    let _effects = update(&mut state, Action::SubmitInstanceName(String::new()));
    assert!(matches!(
        &state.active_view,
        ActiveView::CreateModal(CreateStep::NameInput { error: Some(_), .. })
    ));
}

#[test]
fn test_toggle_version_filter_cycles_releases_and_all() {
    let mut state = AppState::default();
    assert_eq!(state.versions_filter, VersionFilter::Releases);
    let _effects = update(&mut state, Action::SetVersionFilter(VersionFilter::All));
    assert_eq!(state.versions_filter, VersionFilter::All);
    let _effects = update(&mut state, Action::SetVersionFilter(VersionFilter::Releases));
    assert_eq!(state.versions_filter, VersionFilter::Releases);
}

#[test]
fn test_select_version_emits_create_instance_effect() {
    let mut state = AppState {
        versions: vec![VersionEntry {
            id: "1.21.4".into(),
            version_type: "release".into(),
            url: "https://example.invalid/1.21.4.json".into(),
            time: "2024-12-01T00:00:00Z".into(),
            release_time: "2024-12-01T00:00:00Z".into(),
            sha1: "0000000000000000000000000000000000000000".into(),
            compliance_level: 1,
        }],
        active_view: ActiveView::CreateModal(CreateStep::VersionPicker {
            name: "X".into(),
            filter: VersionFilter::Releases,
            search: String::new(),
            error: None,
        }),
        ..AppState::default()
    };

    let effects = update(&mut state, Action::SelectVersion("1.21.4".into()));
    assert_eq!(effects.len(), 1);
    let Effect::CreateInstance { ref mc_version_id, .. } = effects[0] else {
        panic!("expected CreateInstance, got {:?}", effects[0]);
    };
    assert_eq!(mc_version_id, "1.21.4");
}

#[test]
fn test_progress_updates_active_jobs() {
    let mut state = AppState::default();
    let id = JobId(1);
    let _effects = update(
        &mut state,
        Action::Task(TaskEvent::Progress { id, pct: 50, msg: "libs".into() }),
    );
    assert_eq!(state.active_jobs.len(), 1);
    assert_eq!(state.active_jobs[0].0, JobId(1));
    assert_eq!(state.active_jobs[0].1, 50);
    assert_eq!(state.active_jobs[0].2, "libs");
}

#[test]
fn test_instance_installed_action_reloads_list() {
    let mut state = AppState::default();
    let effects = update(&mut state, Action::VersionInstalled { slug: "a".into() });
    assert!(
        effects.iter().any(|e| matches!(e, Effect::FetchInstances)),
        "expected FetchInstances effect"
    );
}

#[test]
fn test_confirm_delete_emits_delete_instance_effect() {
    let mut state = AppState {
        active_view: ActiveView::DeleteConfirm {
            slug: "alpha".into(),
            display_name: "Alpha".into(),
        },
        ..AppState::default()
    };
    let effects = update(&mut state, Action::ConfirmDelete);
    assert!(
        effects.iter().any(|e| matches!(e, Effect::DeleteInstance(s) if s == "alpha")),
        "expected DeleteInstance(alpha)"
    );
    assert!(matches!(state.active_view, ActiveView::InstanceList { .. }));
}

#[test]
fn test_type_search_appends_to_version_picker_search_field() {
    let mut state = AppState {
        active_view: ActiveView::CreateModal(CreateStep::VersionPicker {
            name: "Test".into(),
            filter: VersionFilter::Releases,
            search: String::new(),
            error: None,
        }),
        ..AppState::default()
    };
    let _effects = update(&mut state, Action::TypeSearch('a'));
    if let ActiveView::CreateModal(CreateStep::VersionPicker { search, .. }) = &state.active_view {
        assert!(search.ends_with('a'));
    } else {
        panic!("expected VersionPicker after TypeSearch");
    }
}

// ---- Phase 2 run.rs helper tests (Task 2-07-03) -----------------------------

#[test]
fn test_version_filter_includes_releases_and_excludes_old_beta() {
    use mineltui::tui::run::filter_version_list;

    let versions = vec![
        VersionEntry {
            id: "1.21.4".into(),
            version_type: "release".into(),
            url: String::new(),
            time: String::new(),
            release_time: String::new(),
            sha1: String::new(),
            compliance_level: 1,
        },
        VersionEntry {
            id: "24w45a".into(),
            version_type: "snapshot".into(),
            url: String::new(),
            time: String::new(),
            release_time: String::new(),
            sha1: String::new(),
            compliance_level: 1,
        },
        VersionEntry {
            id: "b1.8.1".into(),
            version_type: "old_beta".into(),
            url: String::new(),
            time: String::new(),
            release_time: String::new(),
            sha1: String::new(),
            compliance_level: 0,
        },
        VersionEntry {
            id: "a1.2.6".into(),
            version_type: "old_alpha".into(),
            url: String::new(),
            time: String::new(),
            release_time: String::new(),
            sha1: String::new(),
            compliance_level: 0,
        },
    ];

    // Releases-only: only "release" type passes; old_beta and old_alpha excluded.
    let releases = filter_version_list(&versions, VersionFilter::Releases, "");
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0].id, "1.21.4");

    // All: release + snapshot pass; old_beta and old_alpha still excluded.
    let all = filter_version_list(&versions, VersionFilter::All, "");
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|v| v.version_type != "old_beta" && v.version_type != "old_alpha"));

    // Search filters by id substring.
    let searched = filter_version_list(&versions, VersionFilter::All, "1.21");
    assert_eq!(searched.len(), 1);
    assert_eq!(searched[0].id, "1.21.4");
}

// ---- Phase 2 view smoke tests (Task 2-07-02) --------------------------------

#[test]
fn test_view_renders_empty_state_without_crash() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::default();
    terminal.draw(|f| mineltui::tui::view::view(&state, f)).unwrap();
}

#[test]
fn test_view_dispatches_without_panic() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    // InstanceList (default)
    let state = AppState::default();
    terminal.draw(|f| mineltui::tui::view::view(&state, f)).unwrap();

    // CreateModal / NameInput
    let state = AppState {
        active_view: ActiveView::CreateModal(CreateStep::NameInput {
            current: "test".into(),
            error: None,
        }),
        ..AppState::default()
    };
    terminal.draw(|f| mineltui::tui::view::view(&state, f)).unwrap();

    // CreateModal / VersionPicker
    let state = AppState {
        active_view: ActiveView::CreateModal(CreateStep::VersionPicker {
            name: "MyInstance".into(),
            filter: VersionFilter::Releases,
            search: String::new(),
            error: None,
        }),
        ..AppState::default()
    };
    terminal.draw(|f| mineltui::tui::view::view(&state, f)).unwrap();

    // DeleteConfirm
    let state = AppState {
        active_view: ActiveView::DeleteConfirm {
            slug: "my-inst".into(),
            display_name: "My Inst".into(),
        },
        ..AppState::default()
    };
    terminal.draw(|f| mineltui::tui::view::view(&state, f)).unwrap();

    // RenameInline
    let state = AppState {
        active_view: ActiveView::RenameInline {
            slug: "my-inst".into(),
            current: "My Inst".into(),
            original: "My Inst".into(),
        },
        ..AppState::default()
    };
    terminal.draw(|f| mineltui::tui::view::view(&state, f)).unwrap();

    // GroupInline
    let state = AppState {
        active_view: ActiveView::GroupInline {
            slug: "alpha".into(),
            buffer: "smp".into(),
            original: None,
        },
        instances: vec![],
        ..AppState::default()
    };
    terminal.draw(|f| mineltui::tui::view::view(&state, f)).unwrap();
}

// ---- INST-06 group-assign smoke tests (Task 2-09-01) ------------------------

use mineltui::domain::InstanceManifest;

fn make_instance(slug: &str, name: &str, group: Option<&str>) -> InstanceManifest {
    let mut m = InstanceManifest::new(name.to_string(), slug.to_string(), "1.20.4".to_string());
    m.group = group.map(|s| s.to_string());
    m
}

#[test]
fn test_group_assign_emits_set_group_effect() {
    let mut state = AppState {
        active_view: ActiveView::InstanceList { selected: 0 },
        instances: vec![make_instance("alpha", "Alpha", None)],
        ..AppState::default()
    };

    // Open the group editor for the selected row.
    let _ = update(&mut state, Action::OpenGroupInput {
        slug: "alpha".into(),
        current: String::new(),
    });
    assert!(
        matches!(&state.active_view, ActiveView::GroupInline { slug, buffer, .. } if slug == "alpha" && buffer.is_empty()),
        "expected GroupInline state with empty buffer"
    );

    // Type "smp" then backspace once -> "sm".
    let _ = update(&mut state, Action::TypeGroup('s'));
    let _ = update(&mut state, Action::TypeGroup('m'));
    let _ = update(&mut state, Action::TypeGroup('p'));
    let _ = update(&mut state, Action::BackspaceGroup);
    if let ActiveView::GroupInline { buffer, .. } = &state.active_view {
        assert_eq!(buffer, "sm");
    } else {
        panic!("expected GroupInline after typing");
    }

    // Submit -> Effect::SetGroup { slug: "alpha", group: Some("sm") } and modal closes.
    let effects = update(&mut state, Action::SubmitGroup);
    assert_eq!(effects.len(), 1, "expected exactly one Effect");
    let Effect::SetGroup { slug, group } = &effects[0] else {
        panic!("expected Effect::SetGroup, got {:?}", effects[0]);
    };
    assert_eq!(slug, "alpha");
    assert_eq!(group.as_deref(), Some("sm"));
    assert!(matches!(state.active_view, ActiveView::InstanceList { .. }));
}

#[test]
fn test_group_assign_empty_buffer_clears_group() {
    let mut state = AppState {
        active_view: ActiveView::InstanceList { selected: 0 },
        instances: vec![make_instance("beta", "Beta", Some("smp"))],
        ..AppState::default()
    };

    // Open the editor prefilled with "smp" (as run.rs would do).
    let _ = update(&mut state, Action::OpenGroupInput {
        slug: "beta".into(),
        current: "smp".into(),
    });
    // Clear the buffer with three backspaces.
    let _ = update(&mut state, Action::BackspaceGroup);
    let _ = update(&mut state, Action::BackspaceGroup);
    let _ = update(&mut state, Action::BackspaceGroup);

    let effects = update(&mut state, Action::SubmitGroup);
    assert_eq!(effects.len(), 1);
    let Effect::SetGroup { slug, group } = &effects[0] else {
        panic!("expected Effect::SetGroup, got {:?}", effects[0]);
    };
    assert_eq!(slug, "beta");
    assert!(group.is_none(), "empty submission must clear the group (pass None)");
}

#[test]
fn test_group_cancel_does_not_emit_effect() {
    let mut state = AppState {
        active_view: ActiveView::GroupInline {
            slug: "gamma".into(),
            buffer: "typed-but-not-saved".into(),
            original: None,
        },
        ..AppState::default()
    };
    let effects = update(&mut state, Action::CancelGroupInput);
    assert!(effects.is_empty(), "cancel must not emit any Effect");
    assert!(matches!(state.active_view, ActiveView::InstanceList { .. }));
}

// ---- Phase 3 launch / stop / modal smoke tests (Task 03-05-02) --------------

use tokio_util::sync::CancellationToken;

#[test]
fn test_enter_on_non_running_emits_launch_effect() {
    let mut state = AppState {
        active_view: ActiveView::InstanceList { selected: 0 },
        instances: vec![make_instance("alpha", "Alpha", None)],
        ..AppState::default()
    };
    let effects = update(&mut state, Action::LaunchInstance { slug: "alpha".into() });
    assert_eq!(effects.len(), 1);
    assert!(
        matches!(&effects[0], Effect::LaunchInstance { slug, .. } if slug == "alpha"),
        "expected Effect::LaunchInstance(alpha), got {effects:?}"
    );
}

#[test]
fn test_enter_on_running_is_noop() {
    let mut state = AppState {
        active_view: ActiveView::InstanceList { selected: 0 },
        instances: vec![make_instance("alpha", "Alpha", None)],
        ..AppState::default()
    };
    state.running_instances.insert("alpha".into(), CancellationToken::new());
    let effects = update(&mut state, Action::LaunchInstance { slug: "alpha".into() });
    assert!(effects.is_empty(), "launching an already-running instance must be a no-op");
}

#[test]
fn test_s_on_running_emits_kill_effect() {
    let mut state = AppState::default();
    let token = CancellationToken::new();
    state.running_instances.insert("beta".into(), token.clone());
    let effects = update(&mut state, Action::StopInstance { slug: "beta".into() });
    assert_eq!(effects.len(), 1);
    assert!(
        matches!(&effects[0], Effect::KillProcess { slug } if slug == "beta"),
        "expected Effect::KillProcess(beta), got {effects:?}"
    );
    // Badge-clearance guarantee: update() removes the slug from running_instances
    // immediately so the running badge disappears on the next render, even before
    // the async launch task dispatches InstanceExited.
    assert!(
        !state.running_instances.contains_key("beta"),
        "slug must be removed from running_instances on StopInstance (badge UX)"
    );
    assert!(
        token.is_cancelled(),
        "token must be cancelled on StopInstance so the async launch task unwinds"
    );
}

#[test]
fn test_launch_failed_transitions_to_modal() {
    let mut state = AppState::default();
    state.running_instances.insert("gamma".into(), CancellationToken::new());
    let effects = update(
        &mut state,
        Action::LaunchFailed {
            slug: "gamma".into(),
            error: "boom".into(),
            log_tail: "line1".into(),
        },
    );
    assert!(effects.is_empty(), "LaunchFailed should produce no effects");
    assert!(
        !state.running_instances.contains_key("gamma"),
        "slug must be removed from running_instances after LaunchFailed"
    );
    let av = &state.active_view;
    assert!(
        matches!(av, ActiveView::LaunchFailedModal { slug, error, .. }
            if slug == "gamma" && error == "boom"),
        "expected LaunchFailedModal, got {av:?}"
    );
}

#[test]
fn test_instance_exited_refreshes_list() {
    let mut state = AppState::default();
    state.running_instances.insert("delta".into(), CancellationToken::new());
    let effects = update(
        &mut state,
        Action::InstanceExited { slug: "delta".into(), duration_ms: 1234 },
    );
    assert!(
        !state.running_instances.contains_key("delta"),
        "slug must be removed from running_instances after InstanceExited"
    );
    assert!(
        effects.iter().any(|e| matches!(e, Effect::FetchInstances)),
        "expected FetchInstances effect after exit"
    );
}

#[test]
fn test_launch_job_started_inserts_token() {
    let mut state = AppState::default();
    assert!(!state.running_instances.contains_key("epsilon"));
    let token = CancellationToken::new();
    let effects = update(
        &mut state,
        Action::LaunchJobStarted { slug: "epsilon".into(), token },
    );
    assert!(effects.is_empty(), "LaunchJobStarted must produce no effects");
    assert!(
        state.running_instances.contains_key("epsilon"),
        "LaunchJobStarted must insert slug into running_instances"
    );
}

#[test]
fn test_d_on_running_is_noop() {
    use mineltui::tui::run::map_event_pub;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    let mut state = AppState {
        active_view: ActiveView::InstanceList { selected: 0 },
        instances: vec![make_instance("zeta", "Zeta", None)],
        ..AppState::default()
    };
    state.running_instances.insert("zeta".into(), CancellationToken::new());

    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &state);
    assert!(
        action.is_none(),
        "pressing d on a running instance must return None, got {action:?}"
    );
}

// ---- Phase 4 account management smoke tests (Task 04-09-03) -----------------

use mineltui::auth::{Account, AuthContext, StorageBackend};
use mineltui::tui::run::map_event_pub;
use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

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

// (1) A keybind opens Accounts view from InstanceList
#[test]
fn test_capital_a_opens_accounts_view_from_instance_list() {
    let state = AppState::default();
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &state).expect("should emit Action");
    assert!(matches!(action, Action::OpenAccounts));
}

// (2) 'a' key in AccountsList emits AddAccount
#[test]
fn test_a_in_accounts_list_emits_add_account() {
    let state = AppState {
        active_view: ActiveView::AccountsList { selected: 0 },
        ..AppState::default()
    };
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &state).unwrap();
    assert!(matches!(action, Action::AddAccount));
}

// (3) Enter in AccountsList emits ActivateAccount for selected
#[test]
fn test_enter_in_accounts_list_activates_selected() {
    let state = AppState {
        accounts: vec![sample_account("A", false), sample_account("B", false)],
        active_view: ActiveView::AccountsList { selected: 1 },
        ..AppState::default()
    };
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let action = map_event_pub(ev, &state).unwrap();
    match action {
        Action::ActivateAccount { id } => assert_eq!(id, "B"),
        other => panic!("expected ActivateAccount, got {other:?}"),
    }
}

// (4) x in AccountsList emits RemoveAccount for selected
#[test]
fn test_x_in_accounts_list_removes_selected() {
    let state = AppState {
        accounts: vec![sample_account("A", true)],
        active_view: ActiveView::AccountsList { selected: 0 },
        ..AppState::default()
    };
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &state).unwrap();
    match action {
        Action::RemoveAccount { id } => assert_eq!(id, "A"),
        other => panic!("expected RemoveAccount, got {other:?}"),
    }
}

// (5) Esc in AccountsList emits CloseAccounts
#[test]
fn test_esc_in_accounts_list_closes() {
    let state = AppState {
        active_view: ActiveView::AccountsList { selected: 0 },
        ..AppState::default()
    };
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let action = map_event_pub(ev, &state).unwrap();
    assert!(matches!(action, Action::CloseAccounts));
}

// (6) Esc in AddAccountDeviceCode emits CancelAddAccount
#[test]
fn test_esc_in_device_code_modal_cancels() {
    let state = AppState {
        active_view: ActiveView::AddAccountDeviceCode {
            user_code: "X".into(),
            verification_uri: "u".into(),
            expires_at: std::time::Instant::now(),
            stage: "s".into(),
        },
        ..AppState::default()
    };
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let action = map_event_pub(ev, &state).unwrap();
    assert!(matches!(action, Action::CancelAddAccount));
}

// (7) Esc in AccountAuthFailed emits CloseModal
#[test]
fn test_esc_in_auth_failed_modal_closes() {
    let state = AppState {
        active_view: ActiveView::AccountAuthFailed { reason: "r".into() },
        ..AppState::default()
    };
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let action = map_event_pub(ev, &state).unwrap();
    assert!(matches!(action, Action::CloseModal) || matches!(action, Action::CloseAccounts));
}

// (8) LaunchInstance with active account builds Msa AuthContext in Effect
#[test]
fn test_launch_effect_with_active_account_is_msa() {
    let mut state = AppState {
        // new(display_name, slug, mc_version_id)
        instances: vec![mineltui::domain::InstanceManifest::new(
            "s".into(),
            "s".into(),
            "1.21.4".into(),
        )],
        active_account_id: Some("acc-1".into()),
        ..AppState::default()
    };
    let effects = update(&mut state, Action::LaunchInstance { slug: "s".into() });
    match effects.as_slice() {
        [Effect::LaunchInstance { auth_ctx: AuthContext::Msa { account_id }, .. }] => {
            assert_eq!(account_id, "acc-1");
        }
        other => panic!("expected Msa launch, got {other:?}"),
    }
}

// (9) AccountsLoaded derives active_account_id from list
#[test]
fn test_accounts_loaded_sets_active_id() {
    let mut state = AppState::default();
    let list = vec![sample_account("A", true), sample_account("B", false)];
    let _ = update(&mut state, Action::AccountsLoaded(list));
    assert_eq!(state.active_account_id.as_deref(), Some("A"));
}

// ---- Phase 5 Java picker smoke tests (Task 05-08-01) ------------------------

use mineltui::java::detect::SystemJava;
use mineltui::tui::app::JavaPickerRow;
use mineltui::java::types::JavaRuntimeId;
use std::path::PathBuf;

fn make_system_java(path: &str, major: u32) -> SystemJava {
    SystemJava { path: PathBuf::from(path), major_version: major }
}

// (1) j on a running instance is a no-op
#[test]
fn test_j_on_running_instance_is_noop() {
    use mineltui::tui::run::map_event_pub;

    let mut state = AppState {
        active_view: ActiveView::InstanceList { selected: 0 },
        instances: vec![make_instance("alpha", "Alpha", None)],
        ..AppState::default()
    };
    state.running_instances.insert("alpha".into(), CancellationToken::new());

    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &state);
    assert!(
        action.is_none(),
        "j on a running instance must be no-op, got {action:?}"
    );
}

// (2) j on a non-running instance dispatches OpenJavaPicker + FetchSystemJavas effect
#[test]
fn test_j_on_non_running_opens_java_picker() {
    use mineltui::tui::run::map_event_pub;

    let state = AppState {
        active_view: ActiveView::InstanceList { selected: 0 },
        instances: vec![make_instance("beta", "Beta", None)],
        ..AppState::default()
    };

    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &state).expect("should emit Action");
    assert!(
        matches!(&action, Action::OpenJavaPicker { slug } if slug == "beta"),
        "expected OpenJavaPicker(beta), got {action:?}"
    );

    // Dispatching it transitions state and emits FetchSystemJavas
    let mut state2 = state;
    let effects = update(&mut state2, action);
    assert!(
        matches!(&state2.active_view, ActiveView::JavaPickerModal { slug, .. } if slug == "beta"),
        "expected JavaPickerModal after OpenJavaPicker"
    );
    assert!(
        effects.iter().any(|e| matches!(e, Effect::FetchSystemJavas { slug } if slug == "beta")),
        "expected FetchSystemJavas effect"
    );
}

// (3) JavaPickerOptionsLoaded populates modal and resets selected to 0
#[test]
fn test_java_picker_options_loaded_populates_modal() {
    let mut state = AppState {
        active_view: ActiveView::JavaPickerModal {
            slug: "gamma".into(),
            options: vec![JavaPickerRow::Auto, JavaPickerRow::Manual],
            selected: 1,
        },
        ..AppState::default()
    };

    let new_options = vec![
        JavaPickerRow::Auto,
        JavaPickerRow::Detected(make_system_java("/usr/bin/java", 21)),
        JavaPickerRow::Manual,
    ];
    let _ = update(&mut state, Action::JavaPickerOptionsLoaded {
        slug: "gamma".into(),
        options: new_options,
    });

    match &state.active_view {
        ActiveView::JavaPickerModal { options, selected, .. } => {
            assert_eq!(options.len(), 3, "options must be replaced");
            assert_eq!(*selected, 0, "selected must reset to 0");
        }
        other => panic!("expected JavaPickerModal, got {other:?}"),
    }
}

// (4) JavaPickerMove wraps around
#[test]
fn test_java_picker_move_wraps_around() {
    let mut state = AppState {
        active_view: ActiveView::JavaPickerModal {
            slug: "delta".into(),
            options: vec![
                JavaPickerRow::Auto,
                JavaPickerRow::Detected(make_system_java("/usr/bin/java", 21)),
                JavaPickerRow::Manual,
            ],
            selected: 0,
        },
        ..AppState::default()
    };

    // Move +1 three times: 0 -> 1 -> 2 -> 0 (wrap)
    let _ = update(&mut state, Action::JavaPickerMove(1));
    let _ = update(&mut state, Action::JavaPickerMove(1));
    let _ = update(&mut state, Action::JavaPickerMove(1));

    match &state.active_view {
        ActiveView::JavaPickerModal { selected, .. } => {
            assert_eq!(*selected, 0, "three +1 moves on 3 options must wrap back to 0");
        }
        other => panic!("expected JavaPickerModal, got {other:?}"),
    }
}

// (5) Enter on Auto row emits SetJavaOverride with override_id: None
#[test]
fn test_java_picker_enter_on_auto_dispatches_set_override_none() {
    let mut state = AppState {
        active_view: ActiveView::JavaPickerModal {
            slug: "epsilon".into(),
            options: vec![
                JavaPickerRow::Auto,
                JavaPickerRow::Detected(make_system_java("/usr/bin/java", 21)),
                JavaPickerRow::Manual,
            ],
            selected: 0, // Auto selected
        },
        ..AppState::default()
    };

    let effects = update(&mut state, Action::JavaPickerSelect);
    assert!(
        effects.iter().any(|e| matches!(e, Effect::SetJavaOverride { override_id: None, slug } if slug == "epsilon")),
        "expected SetJavaOverride with None, got {effects:?}"
    );
    assert!(
        matches!(state.active_view, ActiveView::InstanceList { .. }),
        "modal must close after select"
    );
}

// (6) Enter on Detected row emits SetJavaOverride with Some(System{...})
#[test]
fn test_java_picker_enter_on_detected_dispatches_set_override_system() {
    let mut state = AppState {
        active_view: ActiveView::JavaPickerModal {
            slug: "zeta".into(),
            options: vec![
                JavaPickerRow::Auto,
                JavaPickerRow::Detected(make_system_java("/usr/lib/jvm/java-21/bin/java", 21)),
                JavaPickerRow::Manual,
            ],
            selected: 1, // Detected selected
        },
        ..AppState::default()
    };

    let effects = update(&mut state, Action::JavaPickerSelect);
    let found = effects.iter().any(|e| {
        matches!(e, Effect::SetJavaOverride {
            slug,
            override_id: Some(JavaRuntimeId::System { path, major_version: 21 }),
        } if slug == "zeta" && path == &PathBuf::from("/usr/lib/jvm/java-21/bin/java"))
    });
    assert!(found, "expected SetJavaOverride with System{{path,21}}, got {effects:?}");
}

// (7) Esc returns to InstanceList with no Effect
#[test]
fn test_java_picker_esc_returns_to_instance_list() {
    let mut state = AppState {
        active_view: ActiveView::JavaPickerModal {
            slug: "eta".into(),
            options: vec![JavaPickerRow::Auto, JavaPickerRow::Manual],
            selected: 0,
        },
        ..AppState::default()
    };

    let effects = update(&mut state, Action::JavaPickerCancel);
    assert!(effects.is_empty(), "Esc must emit no effects");
    assert!(
        matches!(state.active_view, ActiveView::InstanceList { .. }),
        "Esc must return to InstanceList"
    );
}

// (10) MoveSelection in AccountsList updates selected index
#[test]
fn test_move_selection_in_accounts_list() {
    let mut state = AppState {
        accounts: vec![
            sample_account("A", false),
            sample_account("B", false),
            sample_account("C", false),
        ],
        active_view: ActiveView::AccountsList { selected: 0 },
        ..AppState::default()
    };
    let _ = update(&mut state, Action::MoveSelection(1));
    match state.active_view {
        ActiveView::AccountsList { selected } => assert_eq!(selected, 1),
        _ => panic!("view changed unexpectedly"),
    }
}

// ========================================================================
// Phase 6: Loader picker / install / switch
// ========================================================================

use mineltui::loader::types::LoaderType;

fn key_l() -> CtEvent {
    CtEvent::Key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE))
}

fn state_with_one_instance(slug: &str, mc: &str) -> AppState {
    let mut s = AppState::default();
    s.instances.push(InstanceManifest::new(slug.into(), slug.into(), mc.into()));
    s
}

#[test]
#[allow(non_snake_case)]
fn test_uppercase_L_on_instance_list_opens_loader_picker() {
    let s = state_with_one_instance("ti", "1.21.4");
    let action = map_event_pub(key_l(), &s);
    match action {
        Some(Action::OpenLoaderPicker { slug }) => assert_eq!(slug, "ti"),
        other => panic!("expected OpenLoaderPicker; got {other:?}"),
    }
}

#[test]
#[allow(non_snake_case)]
fn test_uppercase_L_on_running_instance_is_noop() {
    let mut s = state_with_one_instance("ti", "1.21.4");
    s.running_instances.insert("ti".into(), CancellationToken::new());
    let action = map_event_pub(key_l(), &s);
    assert!(action.is_none(), "L on a running instance should be no-op");
}

#[test]
#[allow(non_snake_case)]
fn test_uppercase_L_blocked_when_loader_install_in_flight() {
    let mut s = state_with_one_instance("ti", "1.21.4");
    s.running_loader_installs.insert("ti".into(), CancellationToken::new());
    let action = map_event_pub(key_l(), &s);
    assert!(action.is_none(), "L during in-flight install should be no-op");
}

#[test]
fn test_loader_picker_select_quilt_emits_fetch_effect() {
    let mut s = state_with_one_instance("ti", "1.21.4");
    let _ = update(&mut s, Action::OpenLoaderPicker { slug: "ti".into() });
    let _ = update(&mut s, Action::LoaderPickerMove(2)); // Quilt at index 2
    let effects = update(&mut s, Action::LoaderPickerSelect);
    match effects.as_slice() {
        [Effect::FetchLoaderVersions { loader_type: LoaderType::Quilt, .. }] => {}
        other => panic!("expected FetchLoaderVersions(Quilt); got {other:?}"),
    }
}

#[test]
fn test_loader_install_progress_action_updates_modal_fields() {
    let mut s = AppState {
        active_view: ActiveView::LoaderInstallProgressModal {
            slug: "ti".into(),
            loader: LoaderType::Fabric,
            version: "0.16.9".into(),
            step_label: "init".into(),
            step_index: 1,
            step_total: 4,
            bytes_done: 0,
            bytes_total: 0,
            cancel_token_key: "ti".into(),
        },
        ..AppState::default()
    };
    let _ = update(&mut s, Action::LoaderInstallProgress {
        slug: "ti".into(),
        pct: 42,
        step_label: "Downloading loader libraries".into(),
        bytes_done: 100,
        bytes_total: 200,
    });
    if let ActiveView::LoaderInstallProgressModal { step_label, bytes_done, bytes_total, .. } = &s.active_view {
        assert_eq!(step_label, "Downloading loader libraries");
        assert_eq!(*bytes_done, 100);
        assert_eq!(*bytes_total, 200);
    } else {
        panic!("expected progress modal")
    }
}

#[test]
fn test_loader_installed_clears_token_and_emits_fetch_instances() {
    let mut s = AppState::default();
    s.running_loader_installs.insert("ti".into(), CancellationToken::new());
    let effects = update(&mut s, Action::LoaderInstalled { slug: "ti".into() });
    assert!(s.running_loader_installs.is_empty());
    assert!(matches!(s.active_view, ActiveView::InstanceList { .. }));
    assert!(effects.iter().any(|e| matches!(e, Effect::FetchInstances)));
}

#[test]
fn test_loader_install_failed_routes_to_failed_modal() {
    let mut s = AppState::default();
    s.running_loader_installs.insert("ti".into(), CancellationToken::new());
    let _ = update(&mut s, Action::LoaderInstallFailed {
        slug: "ti".into(),
        loader: LoaderType::Quilt,
        version: "0.30.0-beta.7".into(),
        error: "no network".into(),
        log_tail: "GET ...".into(),
    });
    assert!(s.running_loader_installs.is_empty());
    assert!(matches!(s.active_view, ActiveView::LoaderInstallFailedModal { .. }));
}

#[test]
fn test_dismiss_loader_install_failed_returns_to_list() {
    let mut s = AppState {
        active_view: ActiveView::LoaderInstallFailedModal {
            slug: "ti".into(),
            loader: LoaderType::Fabric,
            version: "0.16.9".into(),
            error: "x".into(),
            log_tail: "y".into(),
        },
        ..AppState::default()
    };
    let _ = update(&mut s, Action::DismissLoaderInstallFailed);
    assert!(matches!(s.active_view, ActiveView::InstanceList { .. }));
}

#[test]
fn test_confirm_loader_switch_to_none_emits_remove_effect() {
    let mut s = state_with_one_instance("ti", "1.21.4");
    s.active_view = ActiveView::LoaderSwitchConfirm {
        slug: "ti".into(),
        from_loader: Some("fabric:0.16.9".into()),
        to_loader: "none".into(),
        type_switch: false,
    };
    let effects = update(&mut s, Action::ConfirmLoaderSwitch);
    assert!(matches!(effects.as_slice(), [Effect::RemoveLoader { .. }]));
}

#[test]
fn test_confirm_loader_switch_to_quilt_emits_install_effect() {
    let mut s = state_with_one_instance("ti", "1.21.4");
    s.active_view = ActiveView::LoaderSwitchConfirm {
        slug: "ti".into(),
        from_loader: Some("fabric:0.16.9".into()),
        to_loader: "quilt:0.30.0-beta.7".into(),
        type_switch: true,
    };
    let effects = update(&mut s, Action::ConfirmLoaderSwitch);
    match effects.as_slice() {
        [Effect::InstallLoader { loader_type: LoaderType::Quilt, loader_version, .. }] => {
            assert_eq!(loader_version, "0.30.0-beta.7");
        }
        other => panic!("expected Effect::InstallLoader(Quilt); got {other:?}"),
    }
}

#[test]
fn test_cancel_loader_install_cancels_token_and_returns_to_list() {
    let mut s = AppState::default();
    let t = CancellationToken::new();
    s.running_loader_installs.insert("ti".into(), t.clone());
    s.active_view = ActiveView::LoaderInstallProgressModal {
        slug: "ti".into(),
        loader: LoaderType::Fabric,
        version: "0.16.9".into(),
        step_label: "downloading".into(),
        step_index: 2,
        step_total: 4,
        bytes_done: 0,
        bytes_total: 0,
        cancel_token_key: "ti".into(),
    };
    let effects = update(&mut s, Action::CancelLoaderInstall { slug: "ti".into() });
    assert!(t.is_cancelled());
    assert!(s.running_loader_installs.is_empty());
    assert!(matches!(s.active_view, ActiveView::InstanceList { .. }));
    assert!(matches!(effects.as_slice(), [Effect::CancelLoaderInstall { .. }]));
}

// ========================================================================
// Phase 8 (08-07): Modrinth state machine — 15 transition tests
// ========================================================================

use mineltui::mods::types::{
    DepKind, InstalledModRow, ModBrowserFetchState, ModSource, ModrinthFile, ModrinthHashes,
    ModrinthSearchHit, ModrinthVersion, ModrinthVersionEntry, ResolvedDep,
};
use mineltui::tui::app::ModInstallFailedReturnTo;

/// Like `state_with_one_instance`, but returns an instance with no loader
/// (None). Same shape as Phase 6's helper; named distinctly so future
/// loader-bearing helpers can coexist.
fn make_state_with_one_instance(slug: &str, mc: &str) -> AppState {
    let mut s = AppState::default();
    s.instances.push(InstanceManifest::new(slug.into(), slug.into(), mc.into()));
    s
}

/// Build a minimal `ModrinthSearchHit` for tests.
fn hit(project_id: &str, slug: &str) -> ModrinthSearchHit {
    ModrinthSearchHit {
        project_id: project_id.into(),
        slug: slug.into(),
        title: slug.into(),
        description: "x".into(),
        downloads: 0,
        already_installed: false,
    }
}

/// Build a minimal `ModrinthVersion` for tests (no deps).
fn fake_version(id: &str, project_id: &str) -> ModrinthVersion {
    ModrinthVersion {
        id: id.into(),
        project_id: project_id.into(),
        name: id.into(),
        version_number: "1.0.0".into(),
        version_type: "release".into(),
        game_versions: vec!["1.20.4".into()],
        loaders: vec!["fabric".into()],
        downloads: 0,
        date_published: "2026-01-01T00:00:00Z".into(),
        dependencies: vec![],
        files: vec![ModrinthFile {
            url: "https://cdn.modrinth.com/x.jar".into(),
            filename: "x.jar".into(),
            primary: true,
            size: 1024,
            hashes: ModrinthHashes {
                sha1: "aa".into(),
                sha512: "bb".into(),
            },
        }],
    }
}

/// Build a minimal `InstalledModRow` for tests.
fn installed_row(mod_id: &str, name: &str, enabled: bool) -> InstalledModRow {
    InstalledModRow {
        mod_id: mod_id.into(),
        project_slug: name.into(),
        display_name: name.into(),
        version_id: "v1".into(),
        version_label: "1.0.0".into(),
        file_name: format!("{name}.jar"),
        sha512: "deadbeef".into(),
        size: 1024,
        source: ModSource::Modrinth,
        enabled,
        installed_at: "2026-01-01T00:00:00Z".into(),
    }
}

#[test]
fn test_open_mod_browser_emits_search_effect_and_sets_active_view() {
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    let effects = update(&mut state, Action::OpenModBrowser { slug: "foo".into() });
    assert!(matches!(state.active_view, ActiveView::ModBrowser { .. }));
    match effects.as_slice() {
        [Effect::SearchModrinth { slug, query, mc, loader: _ }] => {
            assert_eq!(slug, "foo");
            assert_eq!(query, "");
            assert_eq!(mc.as_deref(), Some("1.20.4"));
        }
        other => panic!("expected SearchModrinth; got {other:?}"),
    }
}

#[test]
fn test_open_mod_browser_blocked_when_install_in_flight() {
    // Pitfall 8 — T-08-07-01.
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.running_mod_jobs.insert("foo".into(), CancellationToken::new());
    let prev_active_view_marker = matches!(state.active_view, ActiveView::InstanceList { .. });
    assert!(prev_active_view_marker, "precondition: starts on InstanceList");
    let effects = update(&mut state, Action::OpenModBrowser { slug: "foo".into() });
    assert!(effects.is_empty(), "guard should produce no effect");
    // active_view must NOT have transitioned to ModBrowser.
    assert!(
        matches!(state.active_view, ActiveView::InstanceList { .. }),
        "active_view should remain on InstanceList while install in flight"
    );
}

#[test]
fn test_mod_browser_search_loaded_replaces_results() {
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::ModBrowser {
        slug: "foo".into(),
        search: String::new(),
        mc_filter_override: None,
        loader_filter_override: None,
        results: Vec::new(),
        selected: 0,
        fetch_state: ModBrowserFetchState::Loading,
        selected_detail: None,
    };
    let _ = update(
        &mut state,
        Action::ModBrowserSearchLoaded {
            slug: "foo".into(),
            hits: vec![hit("P1", "sodium"), hit("P2", "iris")],
        },
    );
    if let ActiveView::ModBrowser { results, fetch_state, .. } = &state.active_view {
        assert_eq!(results.len(), 2);
        assert_eq!(*fetch_state, ModBrowserFetchState::Ready);
    } else {
        panic!("active_view changed unexpectedly");
    }
}

#[test]
fn test_mod_browser_move_clamps_to_results_len() {
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::ModBrowser {
        slug: "foo".into(),
        search: String::new(),
        mc_filter_override: None,
        loader_filter_override: None,
        results: vec![hit("P1", "sodium"), hit("P2", "iris")],
        selected: 0,
        fetch_state: ModBrowserFetchState::Ready,
        selected_detail: None,
    };
    // Move down 5 — should saturate at 1 (len-1).
    for _ in 0..5 {
        let _ = update(&mut state, Action::ModBrowserMove(1));
    }
    if let ActiveView::ModBrowser { selected, .. } = &state.active_view {
        assert_eq!(*selected, 1, "saturating add should clamp at len-1");
    } else {
        panic!()
    }
    // Move up 5 — should saturate at 0.
    for _ in 0..5 {
        let _ = update(&mut state, Action::ModBrowserMove(-1));
    }
    if let ActiveView::ModBrowser { selected, .. } = &state.active_view {
        assert_eq!(*selected, 0, "saturating sub should clamp at 0");
    } else {
        panic!()
    }
}

#[test]
fn test_mod_version_picker_select_emits_resolve_deps_effect() {
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::ModVersionPickerModal {
        slug: "foo".into(),
        project_id: "P1".into(),
        project_title: "Sodium".into(),
        versions: vec![ModrinthVersionEntry {
            version_id: "V1".into(),
            version_label: "0.5.8".into(),
            channel: "release".into(),
            is_latest_stable: true,
        }],
        selected: 0,
    };
    let effects = update(&mut state, Action::ModVersionPickerSelect);
    match effects.as_slice() {
        [Effect::ResolveModDependencies {
            slug,
            project_id,
            version_id,
            mc,
            ..
        }] => {
            assert_eq!(slug, "foo");
            assert_eq!(project_id, "P1");
            assert_eq!(version_id, "V1");
            assert_eq!(mc, "1.20.4");
        }
        other => panic!("expected ResolveModDependencies; got {other:?}"),
    }
    // Stays on the version picker until ModDepsResolved arrives.
    assert!(matches!(state.active_view, ActiveView::ModVersionPickerModal { .. }));
}

#[test]
fn test_mod_version_picker_cancel_returns_to_mod_browser() {
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::ModVersionPickerModal {
        slug: "foo".into(),
        project_id: "P1".into(),
        project_title: "Sodium".into(),
        versions: vec![],
        selected: 0,
    };
    let _ = update(&mut state, Action::ModVersionPickerCancel);
    assert!(matches!(state.active_view, ActiveView::ModBrowser { .. }));
}

#[test]
fn test_dep_confirm_y_emits_install_effect_when_no_conflict() {
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::DepConfirmModal {
        slug: "foo".into(),
        project_id: "P1".into(),
        project_title: "Sodium".into(),
        version_id: "V1".into(),
        version_label: "0.5.8".into(),
        deps: vec![],
        total_bytes: 1024,
        total_files: 1,
        has_conflict: false,
        root_version: Box::new(fake_version("V1", "P1")),
    };
    let effects = update(&mut state, Action::ConfirmModInstall);
    match effects.as_slice() {
        [Effect::InstallModWithDeps { slug, project_title, .. }] => {
            assert_eq!(slug, "foo");
            assert_eq!(project_title, "Sodium");
        }
        other => panic!("expected InstallModWithDeps; got {other:?}"),
    }
}

#[test]
fn test_dep_confirm_y_blocked_when_has_conflict() {
    // T-08-07-04 mitigation.
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::DepConfirmModal {
        slug: "foo".into(),
        project_id: "P1".into(),
        project_title: "Sodium".into(),
        version_id: "V1".into(),
        version_label: "0.5.8".into(),
        deps: vec![ResolvedDep {
            kind: DepKind::Incompatible,
            project_id: "P2".into(),
            project_title: "OptiFine".into(),
            version: None,
            already_satisfied: false,
            is_new_download: false,
        }],
        total_bytes: 0,
        total_files: 0,
        has_conflict: true,
        root_version: Box::new(fake_version("V1", "P1")),
    };
    let effects = update(&mut state, Action::ConfirmModInstall);
    assert!(effects.is_empty(), "y must be a no-op when has_conflict");
    // Stays on the same modal.
    assert!(matches!(state.active_view, ActiveView::DepConfirmModal { .. }));
}

#[test]
fn test_mod_installed_stamps_already_installed_in_browser_results() {
    // Pitfall 10 — T-08-07-02.
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::ModBrowser {
        slug: "foo".into(),
        search: String::new(),
        mc_filter_override: None,
        loader_filter_override: None,
        results: vec![hit("P1", "sodium"), hit("P2", "iris")],
        selected: 0,
        fetch_state: ModBrowserFetchState::Ready,
        selected_detail: None,
    };
    let _ = update(
        &mut state,
        Action::ModInstalled { slug: "foo".into(), project_id: "P1".into() },
    );
    if let ActiveView::ModBrowser { results, .. } = &state.active_view {
        assert!(results[0].already_installed, "Pitfall 10 — already_installed must be stamped");
        assert!(!results[1].already_installed, "non-matching project_id must NOT be stamped");
    } else {
        panic!("active_view changed unexpectedly")
    }
}

#[test]
fn test_install_failed_routes_to_failed_modal_with_return_to() {
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::ModBrowser {
        slug: "foo".into(),
        search: String::new(),
        mc_filter_override: None,
        loader_filter_override: None,
        results: vec![],
        selected: 0,
        fetch_state: ModBrowserFetchState::Ready,
        selected_detail: None,
    };
    state.running_mod_jobs.insert("foo".into(), CancellationToken::new());
    let _ = update(
        &mut state,
        Action::ModInstallFailed {
            slug: "foo".into(),
            mod_title: "Sodium".into(),
            version_label: "0.5.8".into(),
            error: "network".into(),
            log_tail: "GET /...".into(),
        },
    );
    assert!(state.running_mod_jobs.is_empty());
    match &state.active_view {
        ActiveView::ModInstallFailedModal { return_to, .. } => {
            assert_eq!(*return_to, ModInstallFailedReturnTo::ModBrowser);
        }
        other => panic!("expected ModInstallFailedModal; got {other:?}"),
    }
}

#[test]
fn test_open_installed_mods_emits_fetch_effect() {
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    let effects = update(&mut state, Action::OpenInstalledMods { slug: "foo".into() });
    assert!(matches!(state.active_view, ActiveView::InstalledModsList { .. }));
    match effects.as_slice() {
        [Effect::FetchInstalledMods { slug }] => assert_eq!(slug, "foo"),
        other => panic!("expected FetchInstalledMods; got {other:?}"),
    }
}

#[test]
fn test_uninstall_confirm_y_emits_uninstall_effect() {
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::UninstallModConfirm {
        slug: "foo".into(),
        mod_id: "P1".into(),
        display_name: "Sodium".into(),
    };
    let effects = update(&mut state, Action::ConfirmUninstallMod);
    // Returns to InstalledModsList immediately for responsive UX.
    assert!(matches!(state.active_view, ActiveView::InstalledModsList { .. }));
    // Effects: UninstallMod followed by FetchInstalledMods refresh.
    match effects.as_slice() {
        [Effect::UninstallMod { slug, mod_id }, Effect::FetchInstalledMods { slug: slug2 }] => {
            assert_eq!(slug, "foo");
            assert_eq!(mod_id, "P1");
            assert_eq!(slug2, "foo");
        }
        other => panic!("expected UninstallMod then FetchInstalledMods; got {other:?}"),
    }
}

#[test]
fn test_close_installed_mods_returns_to_instance_list() {
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::InstalledModsList {
        slug: "foo".into(),
        mods: vec![],
        selected: 0,
    };
    let _ = update(&mut state, Action::CloseInstalledMods);
    assert!(matches!(state.active_view, ActiveView::InstanceList { .. }));
}

#[test]
fn test_toggle_mod_enabled_emits_correct_effect() {
    // Per /gsd-check-plans Issue 5 — MOD-06 integration coverage.
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::InstalledModsList {
        slug: "foo".into(),
        mods: vec![installed_row("P1", "sodium", true)], // currently enabled
        selected: 0,
    };
    let effects = update(&mut state, Action::ToggleModEnabled);
    match effects.as_slice() {
        [Effect::ToggleModEnabledEff { slug, mod_id, want_enabled }] => {
            assert_eq!(slug, "foo");
            assert_eq!(mod_id, "P1");
            assert!(!*want_enabled, "currently enabled → want_enabled should flip to false");
        }
        other => panic!("expected ToggleModEnabledEff; got {other:?}"),
    }
}

#[test]
fn test_toggle_mc_filter_cycles_state_and_re_emits_search() {
    // Per /gsd-check-plans Issue 9 — MOD-01 filter coverage.
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::ModBrowser {
        slug: "foo".into(),
        search: String::new(),
        mc_filter_override: None,
        loader_filter_override: None,
        results: vec![],
        selected: 0,
        fetch_state: ModBrowserFetchState::Ready,
        selected_detail: None,
    };
    // First toggle: None -> Some("any"). Effect must use mc=None (any filter).
    let effects = update(&mut state, Action::ToggleModMcFilter);
    if let ActiveView::ModBrowser { mc_filter_override, .. } = &state.active_view {
        assert_eq!(mc_filter_override.as_deref(), Some("any"));
    } else {
        panic!()
    }
    match effects.as_slice() {
        [Effect::SearchModrinth { mc, .. }] => assert!(
            mc.is_none(),
            "mc_filter_override='any' must produce mc=None in the effect"
        ),
        other => panic!("expected SearchModrinth; got {other:?}"),
    }
    // Second toggle: Some("any") -> None. Effect must use mc=Some("1.20.4")
    // (instance default restored).
    let effects = update(&mut state, Action::ToggleModMcFilter);
    if let ActiveView::ModBrowser { mc_filter_override, .. } = &state.active_view {
        assert!(mc_filter_override.is_none(), "second toggle restores None");
    } else {
        panic!()
    }
    match effects.as_slice() {
        [Effect::SearchModrinth { mc, .. }] => assert_eq!(
            mc.as_deref(),
            Some("1.20.4"),
            "instance default MC must be restored"
        ),
        other => panic!("expected SearchModrinth; got {other:?}"),
    }
}

// ========================================================================
// Phase 8 (08-08): keymap → action wiring tests for the M/m keybinds and
// the mod_browser j/k disambiguation.
// ========================================================================

/// Helper: build a state focused on the InstanceList view with a single instance.
fn instance_list_state_with(slug: &str, mc: &str) -> AppState {
    AppState {
        active_view: ActiveView::InstanceList { selected: 0 },
        instances: vec![InstanceManifest::new(slug.into(), slug.into(), mc.into())],
        ..AppState::default()
    }
}

#[test]
#[allow(non_snake_case)]
fn test_uppercase_M_on_instance_list_emits_open_mod_browser() {
    let state = instance_list_state_with("alpha", "1.20.4");
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &state).expect("M should emit Action");
    match action {
        Action::OpenModBrowser { slug } => assert_eq!(slug, "alpha"),
        other => panic!("expected OpenModBrowser; got {other:?}"),
    }
}

#[test]
#[allow(non_snake_case)]
fn test_uppercase_M_on_instance_list_blocked_when_install_in_flight() {
    // Pitfall 8 (defense in depth at the keymap layer).
    let mut state = instance_list_state_with("alpha", "1.20.4");
    state.running_mod_jobs.insert("alpha".into(), CancellationToken::new());
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::NONE));
    assert!(
        map_event_pub(ev, &state).is_none(),
        "M must be a no-op while a mod install is in flight for the same slug",
    );
}

#[test]
fn test_lowercase_m_on_instance_list_emits_open_installed_mods() {
    let state = instance_list_state_with("alpha", "1.20.4");
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &state).expect("m should emit Action");
    match action {
        Action::OpenInstalledMods { slug } => assert_eq!(slug, "alpha"),
        other => panic!("expected OpenInstalledMods; got {other:?}"),
    }
}

#[test]
fn test_mod_browser_jk_navigates_when_search_empty() {
    use mineltui::mods::types::ModBrowserFetchState;
    let state = AppState {
        active_view: ActiveView::ModBrowser {
            slug: "alpha".into(),
            search: String::new(),
            mc_filter_override: None,
            loader_filter_override: None,
            results: vec![],
            selected: 0,
            fetch_state: ModBrowserFetchState::Ready,
            selected_detail: None,
        },
        ..AppState::default()
    };
    let ev_j = CtEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    let ev_k = CtEvent::Key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    assert!(matches!(
        map_event_pub(ev_j, &state),
        Some(Action::ModBrowserMove(1))
    ));
    assert!(matches!(
        map_event_pub(ev_k, &state),
        Some(Action::ModBrowserMove(-1))
    ));
}

#[test]
fn test_mod_browser_jk_types_when_search_nonempty() {
    use mineltui::mods::types::ModBrowserFetchState;
    let state = AppState {
        active_view: ActiveView::ModBrowser {
            slug: "alpha".into(),
            search: "fa".into(),
            mc_filter_override: None,
            loader_filter_override: None,
            results: vec![],
            selected: 0,
            fetch_state: ModBrowserFetchState::Ready,
            selected_detail: None,
        },
        ..AppState::default()
    };
    let ev_j = CtEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    let ev_k = CtEvent::Key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    assert!(matches!(
        map_event_pub(ev_j, &state),
        Some(Action::ModBrowserTypeSearch('j'))
    ));
    assert!(matches!(
        map_event_pub(ev_k, &state),
        Some(Action::ModBrowserTypeSearch('k'))
    ));
}

#[test]
fn test_mod_browser_arrows_always_navigate_even_with_search() {
    use mineltui::mods::types::ModBrowserFetchState;
    let state = AppState {
        active_view: ActiveView::ModBrowser {
            slug: "alpha".into(),
            search: "fabric".into(),
            mc_filter_override: None,
            loader_filter_override: None,
            results: vec![],
            selected: 0,
            fetch_state: ModBrowserFetchState::Ready,
            selected_detail: None,
        },
        ..AppState::default()
    };
    let ev_up = CtEvent::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    let ev_dn = CtEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(matches!(
        map_event_pub(ev_up, &state),
        Some(Action::ModBrowserMove(-1))
    ));
    assert!(matches!(
        map_event_pub(ev_dn, &state),
        Some(Action::ModBrowserMove(1))
    ));
}
