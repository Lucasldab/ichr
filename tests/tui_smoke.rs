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
