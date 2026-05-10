//! Smoke test for the TUI reducer. Does NOT start the event loop (that requires
//! a real terminal); instead exercises `update()` directly, which is the only
//! place state mutation happens.

use ichr::mojang::types::VersionEntry;
use ichr::tasks::{JobId, TaskEvent};
use ichr::tui::{update, Action, ActiveView, AppState, CreateStep, Effect, VersionFilter};

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
        Action::Task(TaskEvent::Progress {
            id,
            pct: 10,
            msg: "starting".into(),
        }),
    );
    assert_eq!(state.active_jobs.len(), 1);
    assert_eq!(state.active_jobs[0].1, 10);

    let _ = update(
        &mut state,
        Action::Task(TaskEvent::Progress {
            id,
            pct: 50,
            msg: "halfway".into(),
        }),
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
        Action::Task(TaskEvent::Progress {
            id,
            pct: 25,
            msg: "running".into(),
        }),
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
    let _effects = update(
        &mut state,
        Action::SetVersionFilter(VersionFilter::Releases),
    );
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
    let Effect::CreateInstance {
        ref mc_version_id, ..
    } = effects[0]
    else {
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
        Action::Task(TaskEvent::Progress {
            id,
            pct: 50,
            msg: "libs".into(),
        }),
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
        effects
            .iter()
            .any(|e| matches!(e, Effect::DeleteInstance(s) if s == "alpha")),
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
    use ichr::tui::run::filter_version_list;

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
    assert!(all
        .iter()
        .all(|v| v.version_type != "old_beta" && v.version_type != "old_alpha"));

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
    terminal.draw(|f| ichr::tui::view::view(&state, f)).unwrap();
}

#[test]
fn test_view_dispatches_without_panic() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    // InstanceList (default)
    let state = AppState::default();
    terminal.draw(|f| ichr::tui::view::view(&state, f)).unwrap();

    // CreateModal / NameInput
    let state = AppState {
        active_view: ActiveView::CreateModal(CreateStep::NameInput {
            current: "test".into(),
            error: None,
        }),
        ..AppState::default()
    };
    terminal.draw(|f| ichr::tui::view::view(&state, f)).unwrap();

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
    terminal.draw(|f| ichr::tui::view::view(&state, f)).unwrap();

    // DeleteConfirm
    let state = AppState {
        active_view: ActiveView::DeleteConfirm {
            slug: "my-inst".into(),
            display_name: "My Inst".into(),
        },
        ..AppState::default()
    };
    terminal.draw(|f| ichr::tui::view::view(&state, f)).unwrap();

    // RenameInline
    let state = AppState {
        active_view: ActiveView::RenameInline {
            slug: "my-inst".into(),
            current: "My Inst".into(),
            original: "My Inst".into(),
        },
        ..AppState::default()
    };
    terminal.draw(|f| ichr::tui::view::view(&state, f)).unwrap();

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
    terminal.draw(|f| ichr::tui::view::view(&state, f)).unwrap();
}

// ---- INST-06 group-assign smoke tests (Task 2-09-01) ------------------------

use ichr::domain::InstanceManifest;

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
    let _ = update(
        &mut state,
        Action::OpenGroupInput {
            slug: "alpha".into(),
            current: String::new(),
        },
    );
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
    let _ = update(
        &mut state,
        Action::OpenGroupInput {
            slug: "beta".into(),
            current: "smp".into(),
        },
    );
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
    assert!(
        group.is_none(),
        "empty submission must clear the group (pass None)"
    );
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
    let effects = update(
        &mut state,
        Action::LaunchInstance {
            slug: "alpha".into(),
        },
    );
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
    state
        .running_instances
        .insert("alpha".into(), CancellationToken::new());
    let effects = update(
        &mut state,
        Action::LaunchInstance {
            slug: "alpha".into(),
        },
    );
    assert!(
        effects.is_empty(),
        "launching an already-running instance must be a no-op"
    );
}

#[test]
fn test_s_on_running_emits_kill_effect() {
    let mut state = AppState::default();
    let token = CancellationToken::new();
    state.running_instances.insert("beta".into(), token.clone());
    let effects = update(
        &mut state,
        Action::StopInstance {
            slug: "beta".into(),
        },
    );
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
    state
        .running_instances
        .insert("gamma".into(), CancellationToken::new());
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
    state
        .running_instances
        .insert("delta".into(), CancellationToken::new());
    let effects = update(
        &mut state,
        Action::InstanceExited {
            slug: "delta".into(),
            duration_ms: 1234,
        },
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
        Action::LaunchJobStarted {
            slug: "epsilon".into(),
            token,
        },
    );
    assert!(
        effects.is_empty(),
        "LaunchJobStarted must produce no effects"
    );
    assert!(
        state.running_instances.contains_key("epsilon"),
        "LaunchJobStarted must insert slug into running_instances"
    );
}

#[test]
fn test_d_on_running_is_noop() {
    use ichr::tui::run::map_event_pub;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    let mut state = AppState {
        active_view: ActiveView::InstanceList { selected: 0 },
        instances: vec![make_instance("zeta", "Zeta", None)],
        ..AppState::default()
    };
    state
        .running_instances
        .insert("zeta".into(), CancellationToken::new());

    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &state);
    assert!(
        action.is_none(),
        "pressing d on a running instance must return None, got {action:?}"
    );
}

// ---- Phase 4 account management smoke tests (Task 04-09-03) -----------------

use ichr::auth::{Account, AuthContext, StorageBackend};
use ichr::tui::run::map_event_pub;
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
        instances: vec![ichr::domain::InstanceManifest::new(
            "s".into(),
            "s".into(),
            "1.21.4".into(),
        )],
        active_account_id: Some("acc-1".into()),
        ..AppState::default()
    };
    let effects = update(&mut state, Action::LaunchInstance { slug: "s".into() });
    match effects.as_slice() {
        [Effect::LaunchInstance {
            auth_ctx: AuthContext::Msa { account_id },
            ..
        }] => {
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

use ichr::java::detect::SystemJava;
use ichr::java::types::JavaRuntimeId;
use ichr::tui::app::JavaPickerRow;
use std::path::PathBuf;

fn make_system_java(path: &str, major: u32) -> SystemJava {
    SystemJava {
        path: PathBuf::from(path),
        major_version: major,
    }
}

// (1) j on a running instance is a no-op
#[test]
fn test_j_on_running_instance_is_noop() {
    use ichr::tui::run::map_event_pub;

    let mut state = AppState {
        active_view: ActiveView::InstanceList { selected: 0 },
        instances: vec![make_instance("alpha", "Alpha", None)],
        ..AppState::default()
    };
    state
        .running_instances
        .insert("alpha".into(), CancellationToken::new());

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
    use ichr::tui::run::map_event_pub;

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
        effects
            .iter()
            .any(|e| matches!(e, Effect::FetchSystemJavas { slug } if slug == "beta")),
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
    let _ = update(
        &mut state,
        Action::JavaPickerOptionsLoaded {
            slug: "gamma".into(),
            options: new_options,
        },
    );

    match &state.active_view {
        ActiveView::JavaPickerModal {
            options, selected, ..
        } => {
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
            assert_eq!(
                *selected, 0,
                "three +1 moves on 3 options must wrap back to 0"
            );
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
    assert!(
        found,
        "expected SetJavaOverride with System{{path,21}}, got {effects:?}"
    );
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

use ichr::loader::types::LoaderType;

fn key_l() -> CtEvent {
    CtEvent::Key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE))
}

fn state_with_one_instance(slug: &str, mc: &str) -> AppState {
    let mut s = AppState::default();
    s.instances
        .push(InstanceManifest::new(slug.into(), slug.into(), mc.into()));
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
    s.running_instances
        .insert("ti".into(), CancellationToken::new());
    let action = map_event_pub(key_l(), &s);
    assert!(action.is_none(), "L on a running instance should be no-op");
}

#[test]
#[allow(non_snake_case)]
fn test_uppercase_L_blocked_when_loader_install_in_flight() {
    let mut s = state_with_one_instance("ti", "1.21.4");
    s.running_loader_installs
        .insert("ti".into(), CancellationToken::new());
    let action = map_event_pub(key_l(), &s);
    assert!(
        action.is_none(),
        "L during in-flight install should be no-op"
    );
}

#[test]
fn test_loader_picker_select_quilt_emits_fetch_effect() {
    let mut s = state_with_one_instance("ti", "1.21.4");
    let _ = update(&mut s, Action::OpenLoaderPicker { slug: "ti".into() });
    let _ = update(&mut s, Action::LoaderPickerMove(2)); // Quilt at index 2
    let effects = update(&mut s, Action::LoaderPickerSelect);
    match effects.as_slice() {
        [Effect::FetchLoaderVersions {
            loader_type: LoaderType::Quilt,
            ..
        }] => {}
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
            log_tail: String::new(),
        },
        ..AppState::default()
    };
    let _ = update(
        &mut s,
        Action::LoaderInstallProgress {
            slug: "ti".into(),
            pct: 42,
            step_label: "Downloading loader libraries".into(),
            bytes_done: 100,
            bytes_total: 200,
        },
    );
    if let ActiveView::LoaderInstallProgressModal {
        step_label,
        bytes_done,
        bytes_total,
        ..
    } = &s.active_view
    {
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
    s.running_loader_installs
        .insert("ti".into(), CancellationToken::new());
    let effects = update(&mut s, Action::LoaderInstalled { slug: "ti".into() });
    assert!(s.running_loader_installs.is_empty());
    assert!(matches!(s.active_view, ActiveView::InstanceList { .. }));
    assert!(effects.iter().any(|e| matches!(e, Effect::FetchInstances)));
}

#[test]
fn test_loader_install_failed_routes_to_failed_modal() {
    let mut s = AppState::default();
    s.running_loader_installs
        .insert("ti".into(), CancellationToken::new());
    let _ = update(
        &mut s,
        Action::LoaderInstallFailed {
            slug: "ti".into(),
            loader: LoaderType::Quilt,
            version: "0.30.0-beta.7".into(),
            error: "no network".into(),
            log_tail: "GET ...".into(),
        },
    );
    assert!(s.running_loader_installs.is_empty());
    assert!(matches!(
        s.active_view,
        ActiveView::LoaderInstallFailedModal { .. }
    ));
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
        [Effect::InstallLoader {
            loader_type: LoaderType::Quilt,
            loader_version,
            ..
        }] => {
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
        log_tail: String::new(),
    };
    let effects = update(&mut s, Action::CancelLoaderInstall { slug: "ti".into() });
    assert!(t.is_cancelled());
    assert!(s.running_loader_installs.is_empty());
    assert!(matches!(s.active_view, ActiveView::InstanceList { .. }));
    assert!(matches!(
        effects.as_slice(),
        [Effect::CancelLoaderInstall { .. }]
    ));
}

// ========================================================================
// Phase 8 (08-07): Modrinth state machine -- 15 transition tests
// ========================================================================

use ichr::mods::types::{
    DepKind, HashAlgo, InstalledItemKind, InstalledModRow, ModBrowserFetchState, ModSource,
    ModrinthFile, ModrinthHashes, ModrinthSearchHit, ModrinthVersion, ModrinthVersionEntry,
    ResolvedDep,
};
use ichr::tui::app::ModInstallFailedReturnTo;

/// Like `state_with_one_instance`, but returns an instance with no loader
/// (None). Same shape as Phase 6's helper; named distinctly so future
/// loader-bearing helpers can coexist.
fn make_state_with_one_instance(slug: &str, mc: &str) -> AppState {
    let mut s = AppState::default();
    s.instances
        .push(InstanceManifest::new(slug.into(), slug.into(), mc.into()));
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
        icon_url: None,
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
        hash_algo: HashAlgo::Sha512,
        kind: InstalledItemKind::Mod,
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
    // Two effects fire: the Modrinth search itself, plus a
    // FetchInstalledMods that warms the installed-set cache so
    // SearchLoaded can stamp `already_installed` immune to the
    // install/search round-trip race.
    match effects.as_slice() {
        [Effect::SearchModrinth {
            slug,
            query,
            mc,
            loader: _,
        }, Effect::FetchInstalledMods { slug: slug2 }] => {
            assert_eq!(slug, "foo");
            assert_eq!(query, "");
            assert_eq!(mc.as_deref(), Some("1.20.4"));
            assert_eq!(slug2, "foo");
        }
        other => panic!("expected [SearchModrinth, FetchInstalledMods]; got {other:?}"),
    }
}

#[test]
fn test_open_mod_browser_blocked_when_install_in_flight() {
    // Pitfall 8 -- T-08-07-01.
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state
        .running_mod_jobs
        .insert("foo".into(), CancellationToken::new());
    let prev_active_view_marker = matches!(state.active_view, ActiveView::InstanceList { .. });
    assert!(
        prev_active_view_marker,
        "precondition: starts on InstanceList"
    );
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
        is_searching: false,
        mc_filter_override: None,
        loader_filter_override: None,
        results: Vec::new(),
        selected: 0,
        fetch_state: ModBrowserFetchState::Loading,
        selected_detail: None,
        scroll_offset: 0,
    };
    let _ = update(
        &mut state,
        Action::ModBrowserSearchLoaded {
            slug: "foo".into(),
            hits: vec![hit("P1", "sodium"), hit("P2", "iris")],
        },
    );
    if let ActiveView::ModBrowser {
        results,
        fetch_state,
        ..
    } = &state.active_view
    {
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
        is_searching: false,
        mc_filter_override: None,
        loader_filter_override: None,
        results: vec![hit("P1", "sodium"), hit("P2", "iris")],
        selected: 0,
        fetch_state: ModBrowserFetchState::Ready,
        selected_detail: None,
        scroll_offset: 0,
    };
    // Move down 5 -- should saturate at 1 (len-1).
    for _ in 0..5 {
        let _ = update(&mut state, Action::ModBrowserMove(1));
    }
    if let ActiveView::ModBrowser { selected, .. } = &state.active_view {
        assert_eq!(*selected, 1, "saturating add should clamp at len-1");
    } else {
        panic!()
    }
    // Move up 5 -- should saturate at 0.
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
    assert!(matches!(
        state.active_view,
        ActiveView::ModVersionPickerModal { .. }
    ));
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
        [Effect::InstallModWithDeps {
            slug,
            project_title,
            ..
        }] => {
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
    assert!(matches!(
        state.active_view,
        ActiveView::DepConfirmModal { .. }
    ));
}

#[test]
fn test_mod_installed_stamps_already_installed_in_browser_results() {
    // Pitfall 10 -- T-08-07-02.
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::ModBrowser {
        slug: "foo".into(),
        search: String::new(),
        is_searching: false,
        mc_filter_override: None,
        loader_filter_override: None,
        results: vec![hit("P1", "sodium"), hit("P2", "iris")],
        selected: 0,
        fetch_state: ModBrowserFetchState::Ready,
        selected_detail: None,
        scroll_offset: 0,
    };
    let _ = update(
        &mut state,
        Action::ModInstalled {
            slug: "foo".into(),
            project_id: "P1".into(),
        },
    );
    if let ActiveView::ModBrowser { results, .. } = &state.active_view {
        assert!(
            results[0].already_installed,
            "Pitfall 10 -- already_installed must be stamped"
        );
        assert!(
            !results[1].already_installed,
            "non-matching project_id must NOT be stamped"
        );
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
        is_searching: false,
        mc_filter_override: None,
        loader_filter_override: None,
        results: vec![],
        selected: 0,
        fetch_state: ModBrowserFetchState::Ready,
        selected_detail: None,
        scroll_offset: 0,
    };
    state
        .running_mod_jobs
        .insert("foo".into(), CancellationToken::new());
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
    assert!(matches!(
        state.active_view,
        ActiveView::InstalledModsList { .. }
    ));
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
    assert!(matches!(
        state.active_view,
        ActiveView::InstalledModsList { .. }
    ));
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
    // Per /gsd-check-plans Issue 5 -- MOD-06 integration coverage.
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::InstalledModsList {
        slug: "foo".into(),
        mods: vec![installed_row("P1", "sodium", true)], // currently enabled
        selected: 0,
    };
    let effects = update(&mut state, Action::ToggleModEnabled);
    match effects.as_slice() {
        [Effect::ToggleModEnabledEff {
            slug,
            mod_id,
            want_enabled,
        }] => {
            assert_eq!(slug, "foo");
            assert_eq!(mod_id, "P1");
            assert!(
                !*want_enabled,
                "currently enabled → want_enabled should flip to false"
            );
        }
        other => panic!("expected ToggleModEnabledEff; got {other:?}"),
    }
}

#[test]
fn test_toggle_mc_filter_cycles_state_and_re_emits_search() {
    // Per /gsd-check-plans Issue 9 -- MOD-01 filter coverage.
    let mut state = make_state_with_one_instance("foo", "1.20.4");
    state.active_view = ActiveView::ModBrowser {
        slug: "foo".into(),
        search: String::new(),
        is_searching: false,
        mc_filter_override: None,
        loader_filter_override: None,
        results: vec![],
        selected: 0,
        fetch_state: ModBrowserFetchState::Ready,
        selected_detail: None,
        scroll_offset: 0,
    };
    // First toggle: None -> Some("any"). Effect must use mc=None (any filter).
    let effects = update(&mut state, Action::ToggleModMcFilter);
    if let ActiveView::ModBrowser {
        mc_filter_override, ..
    } = &state.active_view
    {
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
    if let ActiveView::ModBrowser {
        mc_filter_override, ..
    } = &state.active_view
    {
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
    state
        .running_mod_jobs
        .insert("alpha".into(), CancellationToken::new());
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
    use ichr::mods::types::ModBrowserFetchState;
    let state = AppState {
        active_view: ActiveView::ModBrowser {
            slug: "alpha".into(),
            search: String::new(),
            is_searching: false,
            mc_filter_override: None,
            loader_filter_override: None,
            results: vec![],
            selected: 0,
            fetch_state: ModBrowserFetchState::Ready,
            selected_detail: None,
            scroll_offset: 0,
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
fn test_mod_browser_jk_types_when_in_search_mode() {
    // Vim-style: typing into search requires `is_searching: true`,
    // which the user enters by pressing `/`. Non-empty `search` no
    // longer implicitly enables typing (closes the v/l/j/k starting-
    // letter bug).
    use ichr::mods::types::ModBrowserFetchState;
    let state = AppState {
        active_view: ActiveView::ModBrowser {
            slug: "alpha".into(),
            search: "fa".into(),
            is_searching: true,
            mc_filter_override: None,
            loader_filter_override: None,
            results: vec![],
            selected: 0,
            fetch_state: ModBrowserFetchState::Ready,
            selected_detail: None,
            scroll_offset: 0,
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
    use ichr::mods::types::ModBrowserFetchState;
    let state = AppState {
        active_view: ActiveView::ModBrowser {
            slug: "alpha".into(),
            search: "fabric".into(),
            is_searching: false,
            mc_filter_override: None,
            loader_filter_override: None,
            results: vec![],
            selected: 0,
            fetch_state: ModBrowserFetchState::Ready,
            selected_detail: None,
            scroll_offset: 0,
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

// ========================================================================
// Phase 9 (09-06): CurseForge state machine -- 12 tui_smoke tests
// ========================================================================

use ichr::mods::curseforge::types::{
    CurseForgeFileEntry, CurseForgeProjectDetail, CurseForgeSearchHit,
};

/// Build an `AppState` with `cf_api_key_present` set as requested.
/// Uses struct-update syntax to satisfy `clippy::field_reassign_with_default`.
fn cf_state(api_key_present: bool) -> AppState {
    AppState {
        cf_api_key_present: api_key_present,
        ..AppState::default()
    }
}

/// Build a minimal `CurseForgeSearchHit` for tests.
fn cf_hit(id: u64, slug: &str) -> CurseForgeSearchHit {
    CurseForgeSearchHit {
        id,
        slug: slug.into(),
        name: slug.into(),
        summary: "x".into(),
        download_count: 100,
        categories: vec![],
        logo: None,
        already_installed: false,
    }
}

/// Build a minimal `CurseForgeProjectDetail` for tests.
fn cf_project_detail(id: u64, slug: &str) -> CurseForgeProjectDetail {
    CurseForgeProjectDetail {
        id,
        slug: slug.into(),
        name: slug.into(),
        summary: "x".into(),
        description: "x".into(),
        download_count: 100,
        authors: vec![],
        links: Default::default(),
    }
}

/// Build a minimal `CurseForgeFileEntry` for tests. `dl` is the (nullable)
/// downloadUrl -- None mirrors the FileNotDownloadable wire shape (MOD-04).
fn cf_file_entry(id: u64, fname: &str, dl: Option<String>) -> CurseForgeFileEntry {
    CurseForgeFileEntry {
        id,
        display_name: fname.into(),
        file_name: format!("{fname}.jar"),
        release_type: 1,
        file_status: 4,
        hashes: vec![],
        file_date: "2026-01-01T00:00:00Z".into(),
        file_length: 1024,
        download_count: 0,
        download_url: dl,
        game_versions: vec!["1.20.4".into()],
        dependencies: vec![],
        is_available: true,
    }
}

#[test]
fn test_cf_open_no_op_when_api_key_absent() {
    // Pitfall 1 -- F is silently disabled when no API key is configured.
    let mut s = cf_state(false);
    let effects = update(&mut s, Action::OpenCfBrowser { slug: "x".into() });
    assert!(
        !matches!(s.active_view, ActiveView::CfBrowser { .. }),
        "active_view must NOT transition to CfBrowser when api key absent"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::SearchCurseForge { .. })),
        "no SearchCurseForge effect must be emitted"
    );
}

#[test]
fn test_cf_open_transitions_to_cf_browser_when_api_key_present() {
    // Happy path -- F opens CfBrowser + emits SearchCurseForge.
    let mut s = cf_state(true);
    let effects = update(&mut s, Action::OpenCfBrowser { slug: "x".into() });
    assert!(matches!(s.active_view, ActiveView::CfBrowser { .. }));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SearchCurseForge { .. })));
}

#[test]
fn test_cf_open_blocked_while_install_in_flight_on_same_instance() {
    // Pitfall 8 inheritance from Phase 8 -- running_mod_jobs is the
    // source-agnostic per-instance install lock.
    let mut s = cf_state(true);
    s.running_mod_jobs
        .insert("x".into(), CancellationToken::new());
    let effects = update(&mut s, Action::OpenCfBrowser { slug: "x".into() });
    assert!(
        !matches!(s.active_view, ActiveView::CfBrowser { .. }),
        "Pitfall 8 -- install in flight must block the F keybind"
    );
    assert!(effects.is_empty(), "Pitfall 8 guard must produce no effect");
}

#[test]
fn test_cf_browser_search_loaded_resets_state() {
    let mut s = cf_state(true);
    let _ = update(&mut s, Action::OpenCfBrowser { slug: "x".into() });
    let hits = vec![cf_hit(1, "sodium"), cf_hit(2, "iris")];
    let _ = update(
        &mut s,
        Action::CfBrowserSearchLoaded {
            slug: "x".into(),
            hits: hits.clone(),
        },
    );
    if let ActiveView::CfBrowser {
        results,
        fetch_state,
        selected,
        ..
    } = &s.active_view
    {
        assert_eq!(*selected, 0);
        assert!(matches!(
            fetch_state,
            ichr::mods::types::ModBrowserFetchState::Ready
        ));
        assert_eq!(results.len(), hits.len());
    } else {
        panic!("expected CfBrowser, got {:?}", s.active_view);
    }
}

#[test]
fn test_cf_browser_type_search_appends_char() {
    let mut s = cf_state(true);
    let _ = update(&mut s, Action::OpenCfBrowser { slug: "x".into() });
    let _ = update(&mut s, Action::CfBrowserTypeSearch('s'));
    let _ = update(&mut s, Action::CfBrowserTypeSearch('o'));
    if let ActiveView::CfBrowser { search_input, .. } = &s.active_view {
        assert_eq!(search_input, "so");
    } else {
        panic!("expected CfBrowser, got {:?}", s.active_view);
    }
}

#[test]
fn test_cf_browser_open_detail_emits_fetch_cf_mod_effect() {
    // Phase 8-mirror Action ping-pong: half 1.
    // CfBrowserOpenDetail emits Effect::FetchCfMod -- NOT a combined
    // OpenCfFilePicker effect. The design locks separate FetchCfMod + ListCfFiles.
    let mut s = cf_state(true);
    let _ = update(&mut s, Action::OpenCfBrowser { slug: "x".into() });
    let effects = update(
        &mut s,
        Action::CfBrowserOpenDetail {
            slug: "x".into(),
            mod_id: 12345,
        },
    );
    let found_fetch = effects.iter().any(|e| match e {
        Effect::FetchCfMod { mod_id, .. } => *mod_id == 12345,
        _ => false,
    });
    assert!(
        found_fetch,
        "expected FetchCfMod with mod_id=12345 (Action ping-pong half 1), got {effects:?}"
    );
}

#[test]
fn test_cf_browser_detail_loaded_chains_to_list_cf_files_effect() {
    // Phase 8-mirror Action ping-pong: half 2.
    // CfBrowserDetailLoaded MUST automatically chain to ListCfFiles --
    // mirrors Phase 8 ModDetailLoaded -> ModVersionsLoaded pattern.
    let mut s = cf_state(true);
    let _ = update(&mut s, Action::OpenCfBrowser { slug: "x".into() });
    let detail = cf_project_detail(12345, "sodium");
    let effects = update(
        &mut s,
        Action::CfBrowserDetailLoaded {
            slug: "x".into(),
            detail,
        },
    );
    let found_list = effects.iter().any(|e| match e {
        Effect::ListCfFiles { mod_id, .. } => *mod_id == 12345,
        _ => false,
    });
    assert!(
        found_list,
        "expected ListCfFiles after detail loaded (Phase 8 ping-pong mirror), got {effects:?}"
    );
}

#[test]
fn test_cf_file_picker_confirm_emits_install_cf_mod_effect_when_no_install_in_flight() {
    let mut s = cf_state(true);
    s.active_view = ActiveView::CfFilePickerModal {
        slug: "x".into(),
        mod_detail: cf_project_detail(1, "sodium"),
        files: vec![cf_file_entry(2, "sodium", Some("https://x".into()))],
        selected: 0,
    };
    let effects = update(&mut s, Action::CfFilePickerConfirm);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::InstallCfMod { .. })),
        "expected InstallCfMod, got {effects:?}"
    );
}

#[test]
fn test_cf_file_picker_confirm_blocked_while_install_in_flight() {
    // Pitfall 8 guard -- even with an open file picker, install-in-flight on
    // the same instance must silently no-op.
    let mut s = cf_state(true);
    s.running_mod_jobs
        .insert("x".into(), CancellationToken::new());
    s.active_view = ActiveView::CfFilePickerModal {
        slug: "x".into(),
        mod_detail: cf_project_detail(1, "sodium"),
        files: vec![cf_file_entry(2, "sodium", Some("https://x".into()))],
        selected: 0,
    };
    let effects = update(&mut s, Action::CfFilePickerConfirm);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::InstallCfMod { .. })),
        "Pitfall 8 -- InstallCfMod must NOT be emitted while install in flight"
    );
}

#[test]
fn test_cf_mod_install_failed_transitions_to_modal_with_web_url() {
    // MOD-04 success criterion 3 -- FileNotDownloadable carries web_url,
    // and the modal renders the link.
    let mut s = cf_state(true);
    let url = "https://www.curseforge.com/minecraft/mc-mods/wonderful-world-mod/files/4567890"
        .to_string();
    let _ = update(
        &mut s,
        Action::CfModInstallFailed {
            slug: "x".into(),
            mod_title: "Wonderful World".into(),
            file_label: "1.5.0".into(),
            error: "Author has disabled third-party downloads".into(),
            web_url: Some(url.clone()),
        },
    );
    if let ActiveView::CfInstallFailedModal {
        web_url: Some(u), ..
    } = &s.active_view
    {
        assert_eq!(*u, url);
    } else {
        panic!(
            "expected CfInstallFailedModal with Some(web_url), got {:?}",
            s.active_view
        );
    }
}

#[test]
fn test_cf_dismiss_install_failed_returns_to_instance_list() {
    let mut s = cf_state(true);
    s.active_view = ActiveView::CfInstallFailedModal {
        slug: "x".into(),
        mod_title: "X".into(),
        file_label: "v".into(),
        error_message: "e".into(),
        web_url: None,
    };
    let _ = update(&mut s, Action::CfDismissInstallFailed);
    assert!(
        matches!(s.active_view, ActiveView::InstanceList { .. }),
        "Esc on CfInstallFailedModal must return to InstanceList, got {:?}",
        s.active_view
    );
}

#[test]
fn test_cf_mod_install_started_inserts_running_mod_job_and_cf_mod_installed_removes_it() {
    // Single-mutation-point invariant for running_mod_jobs.
    let mut s = cf_state(true);
    let token = CancellationToken::new();
    let _ = update(
        &mut s,
        Action::CfModInstallStarted {
            slug: "x".into(),
            mod_id: 1,
            file_id: 2,
            token,
        },
    );
    assert!(
        s.running_mod_jobs.contains_key("x"),
        "CfModInstallStarted must insert into running_mod_jobs"
    );
    let _ = update(
        &mut s,
        Action::CfModInstalled {
            slug: "x".into(),
            mod_id: 1,
        },
    );
    assert!(
        !s.running_mod_jobs.contains_key("x"),
        "CfModInstalled must remove the running_mod_jobs entry"
    );
}

// ========================================================================
// Phase 7 Plan 05 (07-05): Forge/NeoForge TUI integration smoke tests
// ========================================================================

/// Render `state` into an 80×height buffer and return the concatenated cell
/// content as a single string (each row joined, rows concatenated).
fn render_state_to_string(state: &AppState, width: u16, height: u16) -> String {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| ichr::tui::view::view(state, f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    let mut out = String::new();
    for row in 0..height {
        for col in 0..width {
            out.push_str(buf.cell((col, row)).map(|c| c.symbol()).unwrap_or(" "));
        }
    }
    out
}

#[test]
fn test_loader_picker_shows_5_rows_for_supported_mc() {
    let mut s = state_with_one_instance("ti", "1.20.1");
    s.active_view = ActiveView::LoaderPickerModal {
        slug: "ti".into(),
        selected: 0,
    };
    let rendered = render_state_to_string(&s, 80, 24);
    assert!(
        rendered.contains("Fabric Loader"),
        "picker must show Fabric row: {rendered}"
    );
    assert!(
        rendered.contains("Quilt Loader"),
        "picker must show Quilt row: {rendered}"
    );
    assert!(
        rendered.contains("Forge"),
        "picker must show Forge row: {rendered}"
    );
    assert!(
        rendered.contains("NeoForge"),
        "picker must show NeoForge row: {rendered}"
    );
}

#[test]
fn test_loader_install_progress_renders_log_tail() {
    let s = AppState {
        active_view: ActiveView::LoaderInstallProgressModal {
            slug: "ti".into(),
            loader: LoaderType::Forge,
            version: "47.4.20".into(),
            step_label: "Running installer".into(),
            step_index: 1,
            step_total: 5,
            bytes_done: 0,
            bytes_total: 0,
            cancel_token_key: "k".into(),
            log_tail: "Running Processor 3/7".into(),
        },
        ..AppState::default()
    };
    let rendered = render_state_to_string(&s, 80, 30);
    assert!(
        rendered.contains("Running Processor 3/7"),
        "log_tail not rendered: {rendered}"
    );
}

#[test]
fn test_loader_install_failed_renders_subprocess_tail() {
    let s = AppState {
        active_view: ActiveView::LoaderInstallFailedModal {
            slug: "ti".into(),
            loader: LoaderType::Forge,
            version: "47.4.20".into(),
            error: "Installer exited with code 1".into(),
            log_tail: "java.lang.NullPointerException at Foo".into(),
        },
        ..AppState::default()
    };
    let rendered = render_state_to_string(&s, 80, 24);
    assert!(
        rendered.contains("java.lang.NullPointerException"),
        "log_tail not rendered in failed modal: {rendered}"
    );
}

#[test]
fn test_version_picker_empty_state_forge_below_113() {
    let mut s = state_with_one_instance("ti", "1.12.2");
    s.active_view = ActiveView::LoaderVersionPickerModal {
        slug: "ti".into(),
        loader: LoaderType::Forge,
        versions: vec![],
        filter_stable_only: true,
        search: String::new(),
        selected: 0,
        current_version: None,
    };
    let rendered = render_state_to_string(&s, 80, 24);
    assert!(
        rendered.contains("Forge requires 1.13+"),
        "MC-incompatibility copy not shown for Forge/1.12.2: {rendered}"
    );
}

#[test]
fn test_version_picker_empty_state_neoforge_below_1201() {
    let mut s = state_with_one_instance("ti", "1.19.4");
    s.active_view = ActiveView::LoaderVersionPickerModal {
        slug: "ti".into(),
        loader: LoaderType::NeoForge,
        versions: vec![],
        filter_stable_only: true,
        search: String::new(),
        selected: 0,
        current_version: None,
    };
    let rendered = render_state_to_string(&s, 80, 24);
    assert!(
        rendered.contains("NeoForge requires 1.20.1+"),
        "MC-incompatibility copy not shown for NeoForge/1.19.4: {rendered}"
    );
}

// ========================================================================
// Phase 10: Modpack Import TUI integration (Plan 10-06)
// 13 tui_smoke tests covering keybind, modal flow, progress dispatch, cancel
// cleanup, failure modal dismiss.
// ========================================================================

// Test 1: 'i' on InstanceList opens path-input modal
#[test]
fn test_i_key_on_instance_list_opens_path_input_modal() {
    let mut s = state_with_one_instance("ti", "1.21.4");
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &s).expect("'i' must emit an action");
    assert!(
        matches!(action, Action::OpenModpackImport),
        "expected OpenModpackImport; got {action:?}"
    );
    update(&mut s, action);
    assert!(
        matches!(s.active_view, ActiveView::ModpackImportPathInput { .. }),
        "active_view must be ModpackImportPathInput; got {:?}",
        s.active_view
    );
}

// Test 2: Typing chars in the path-input modal updates the buffer
#[test]
fn test_path_input_typing_updates_buffer() {
    let mut s = AppState {
        active_view: ActiveView::ModpackImportPathInput {
            buffer: String::new(),
            error: None,
        },
        ..AppState::default()
    };
    update(&mut s, Action::ImportPathTypeSearch('a'));
    update(&mut s, Action::ImportPathTypeSearch('b'));
    update(&mut s, Action::ImportPathTypeSearch('c'));
    match &s.active_view {
        ActiveView::ModpackImportPathInput { buffer, .. } => {
            assert_eq!(buffer, "abc", "buffer must be 'abc' after typing 3 chars");
        }
        _ => panic!("view changed unexpectedly: {:?}", s.active_view),
    }
}

// Test 3: Paste appends to buffer
#[test]
fn test_path_input_paste_appends_to_buffer() {
    let mut s = AppState {
        active_view: ActiveView::ModpackImportPathInput {
            buffer: "x".into(),
            error: None,
        },
        ..AppState::default()
    };
    update(
        &mut s,
        Action::ImportPathPasteSearch("/path/to/pack.mrpack".into()),
    );
    match &s.active_view {
        ActiveView::ModpackImportPathInput { buffer, .. } => {
            assert_eq!(buffer, "x/path/to/pack.mrpack");
        }
        _ => panic!("view changed unexpectedly"),
    }
}

// Test 4: Backspace pops the last char
#[test]
fn test_path_input_backspace_pops_buffer() {
    let mut s = AppState {
        active_view: ActiveView::ModpackImportPathInput {
            buffer: "abc".into(),
            error: None,
        },
        ..AppState::default()
    };
    update(&mut s, Action::ImportPathBackspaceSearch);
    match &s.active_view {
        ActiveView::ModpackImportPathInput { buffer, .. } => {
            assert_eq!(buffer, "ab");
        }
        _ => panic!("view changed unexpectedly"),
    }
}

// Test 5: Submit with empty path sets error and stays on path-input modal
#[test]
fn test_path_input_submit_empty_sets_error() {
    let mut s = AppState {
        active_view: ActiveView::ModpackImportPathInput {
            buffer: String::new(),
            error: None,
        },
        ..AppState::default()
    };
    let effects = update(&mut s, Action::ImportPathSubmit);
    assert!(
        effects.is_empty(),
        "empty-path submit must emit no effects; got {effects:?}"
    );
    match &s.active_view {
        ActiveView::ModpackImportPathInput { error: Some(e), .. } => {
            assert!(
                e.contains("path required"),
                "error must mention 'path required'; got {e}"
            );
        }
        ActiveView::ModpackImportPathInput { error: None, .. } => {
            panic!("submit with empty path must set error")
        }
        _ => panic!("view must remain ModpackImportPathInput"),
    }
}

// Test 6: Submit with non-empty path dispatches Effect::ImportModpack
#[test]
fn test_path_input_submit_dispatches_import_modpack_effect() {
    let mut s = AppState {
        active_view: ActiveView::ModpackImportPathInput {
            buffer: "/some/path.mrpack".into(),
            error: None,
        },
        ..AppState::default()
    };
    let effects = update(&mut s, Action::ImportPathSubmit);
    let has_import = effects.iter().any(|e| {
        matches!(e, Effect::ImportModpack { mrpack_path }
            if mrpack_path == &std::path::PathBuf::from("/some/path.mrpack"))
    });
    assert!(
        has_import,
        "submit with non-empty path must emit Effect::ImportModpack{{path}}; got {effects:?}"
    );
}

// Test 7: Esc on path-input modal cancels and returns to InstanceList
#[test]
fn test_path_input_esc_cancels_to_instance_list() {
    let mut s = AppState {
        active_view: ActiveView::ModpackImportPathInput {
            buffer: "/some/path.mrpack".into(),
            error: None,
        },
        ..AppState::default()
    };
    update(&mut s, Action::ImportPathCancel);
    assert!(
        matches!(s.active_view, ActiveView::InstanceList { selected: 0 }),
        "Esc must return to InstanceList; got {:?}",
        s.active_view
    );
}

// Test 8: ModpackImportStarted transitions to progress modal and records token
#[test]
fn test_modpack_import_started_transitions_to_progress_modal_and_records_token() {
    let token = CancellationToken::new();
    let mut s = AppState::default();
    update(
        &mut s,
        Action::ModpackImportStarted {
            slug: "test-pack".into(),
            modpack_name: "Test Pack".into(),
            token: token.clone(),
        },
    );
    assert!(
        matches!(s.active_view, ActiveView::ModpackImportProgressModal { .. }),
        "must transition to ModpackImportProgressModal; got {:?}",
        s.active_view
    );
    assert!(
        s.running_modpack_imports.contains_key("test-pack"),
        "token must be recorded in running_modpack_imports"
    );
}

// Test 9: ModpackImportProgress updates step_label and bytes_done in modal
#[test]
fn test_modpack_import_progress_updates_modal_step_label() {
    let token = CancellationToken::new();
    let mut s = AppState {
        active_view: ActiveView::ModpackImportProgressModal {
            modpack_name: "Test Pack".into(),
            step_label: "Starting".into(),
            step_index: 0,
            step_total: 7,
            bytes_done: 0,
            bytes_total: 0,
            cancel_token_key: "test-pack".into(),
            log_tail: String::new(),
        },
        ..AppState::default()
    };
    s.running_modpack_imports.insert("test-pack".into(), token);
    update(
        &mut s,
        Action::ModpackImportProgress {
            slug: String::new(),
            pct: 50,
            step_label: "Downloading mods 5/10".into(),
            bytes_done: 1024,
            bytes_total: 2048,
        },
    );
    match &s.active_view {
        ActiveView::ModpackImportProgressModal {
            step_label,
            bytes_done,
            ..
        } => {
            assert_eq!(step_label, "Downloading mods 5/10");
            assert_eq!(*bytes_done, 1024);
        }
        _ => panic!("view must remain ModpackImportProgressModal"),
    }
}

// Test 10: CancelModpackImport calls token.cancel() and returns to InstanceList
#[test]
fn test_cancel_modpack_import_calls_token_cancel_and_returns_to_instance_list() {
    let token = CancellationToken::new();
    let token_clone = token.clone();
    let mut s = AppState {
        active_view: ActiveView::ModpackImportProgressModal {
            modpack_name: "Test Pack".into(),
            step_label: "Downloading".into(),
            step_index: 3,
            step_total: 7,
            bytes_done: 512,
            bytes_total: 1024,
            cancel_token_key: "test-pack".into(),
            log_tail: String::new(),
        },
        ..AppState::default()
    };
    s.running_modpack_imports.insert("test-pack".into(), token);

    let effects = update(&mut s, Action::CancelModpackImport);

    assert!(
        token_clone.is_cancelled(),
        "CancelModpackImport must call token.cancel()"
    );
    assert!(
        matches!(s.active_view, ActiveView::InstanceList { selected: 0 }),
        "must return to InstanceList; got {:?}",
        s.active_view
    );
    assert!(
        s.running_modpack_imports.is_empty(),
        "running_modpack_imports must be empty after cancel"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CancelModpackImport)),
        "must emit Effect::CancelModpackImport; got {effects:?}"
    );
}

// Test 11: ModpackImported clears state and emits FetchInstances
#[test]
fn test_modpack_imported_clears_state_and_emits_fetch_instances() {
    let token = CancellationToken::new();
    let mut s = AppState {
        active_view: ActiveView::ModpackImportProgressModal {
            modpack_name: "Test Pack".into(),
            step_label: "Done".into(),
            step_index: 7,
            step_total: 7,
            bytes_done: 1024,
            bytes_total: 1024,
            cancel_token_key: "test-pack".into(),
            log_tail: String::new(),
        },
        ..AppState::default()
    };
    s.running_modpack_imports.insert("test-pack".into(), token);

    let effects = update(
        &mut s,
        Action::ModpackImported {
            slug: "test-pack".into(),
        },
    );

    assert!(
        matches!(s.active_view, ActiveView::InstanceList { selected: 0 }),
        "must return to InstanceList; got {:?}",
        s.active_view
    );
    assert!(
        s.running_modpack_imports.is_empty(),
        "running_modpack_imports must be empty after import succeeded"
    );
    assert!(
        effects.iter().any(|e| matches!(e, Effect::FetchInstances)),
        "must emit Effect::FetchInstances; got {effects:?}"
    );
}

// Test 12: ModpackImportFailed shows failed modal; DismissModpackImportFailed returns to InstanceList
#[test]
fn test_modpack_import_failed_opens_failed_modal_then_dismiss() {
    let mut s = AppState::default();
    update(
        &mut s,
        Action::ModpackImportFailed {
            modpack_name: "Bad Pack".into(),
            error: "boom".into(),
            log_tail: String::new(),
        },
    );
    assert!(
        matches!(s.active_view, ActiveView::ModpackImportFailedModal { .. }),
        "must transition to ModpackImportFailedModal; got {:?}",
        s.active_view
    );

    update(&mut s, Action::DismissModpackImportFailed);
    assert!(
        matches!(s.active_view, ActiveView::InstanceList { selected: 0 }),
        "DismissModpackImportFailed must return to InstanceList; got {:?}",
        s.active_view
    );
}

// Test 13: HIGH-2 regression pin -- ModpackImportCancelled clears running_modpack_imports
// and returns to InstanceList (NOT ModpackImportFailedModal). This pinned regression
// ensures the dedicated ModpackImportCancelled arm calls clear() rather than the
// ModpackImported arm calling remove("") (a no-op against a real-slug key).
#[test]
fn test_modpack_import_cancelled_clears_running_imports_and_returns_to_instance_list() {
    let token = CancellationToken::new();
    let mut s = AppState {
        active_view: ActiveView::ModpackImportProgressModal {
            modpack_name: "Real Pack".into(),
            step_label: "Downloading".into(),
            step_index: 2,
            step_total: 7,
            bytes_done: 0,
            bytes_total: 0,
            cancel_token_key: "real-slug".into(),
            log_tail: String::new(),
        },
        ..AppState::default()
    };
    // Pre-populate with a real slug (as if ModpackImportStarted had fired)
    s.running_modpack_imports.insert("real-slug".into(), token);

    let effects = update(&mut s, Action::ModpackImportCancelled);

    // Must return to InstanceList (not ModpackImportFailedModal -- silent treatment)
    assert!(
        matches!(s.active_view, ActiveView::InstanceList { selected: 0 }),
        "ModpackImportCancelled must return to InstanceList; got {:?}",
        s.active_view
    );

    // HIGH-2: running_modpack_imports must be empty regardless of which slug was assigned
    assert!(
        s.running_modpack_imports.is_empty(),
        "running_modpack_imports must be cleared by ModpackImportCancelled"
    );

    // Must NOT emit FetchInstances (nothing was created)
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::FetchInstances)),
        "ModpackImportCancelled must NOT emit FetchInstances; got {effects:?}"
    );

    // Must NOT be a failed modal (silent cancellation)
    assert!(
        !matches!(s.active_view, ActiveView::ModpackImportFailedModal { .. }),
        "ModpackImportCancelled must NOT open the failed modal"
    );

    // Effects must be empty (no side-effects needed -- everything is cleaned up by update())
    assert!(
        effects.is_empty(),
        "ModpackImportCancelled must emit no effects; got {effects:?}"
    );
}

// ── Phase 11 (11-04): Pack browser + installed list + D-LOCK keybind tests ──

use ichr::packs::kind::PackKind;

// Helper: state with one instance + InstanceList active view.
fn pack_instance_state(slug: &str) -> AppState {
    let mut s = AppState::default();
    s.instances.push(ichr::domain::InstanceManifest::new(
        slug.into(),
        slug.into(),
        "1.20.4".into(),
    ));
    s
}

// Helper: minimal installed pack row.
fn pack_row(mod_id: &str, name: &str) -> InstalledModRow {
    use ichr::mods::types::{HashAlgo, InstalledItemKind, ModSource};
    InstalledModRow {
        mod_id: mod_id.into(),
        project_slug: name.into(),
        display_name: name.into(),
        version_id: "v1".into(),
        version_label: "1.0.0".into(),
        file_name: format!("{name}.zip"),
        sha512: "deadbeef".into(),
        size: 1024,
        hash_algo: HashAlgo::Sha512,
        kind: InstalledItemKind::ResourcePack,
        source: ModSource::Local,
        enabled: true,
        installed_at: "2026-01-01T00:00:00Z".into(),
    }
}

#[test]
#[allow(non_snake_case)]
fn test_uppercase_R_opens_resource_pack_browser() {
    use ichr::tui::run::map_event_pub;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    let s = pack_instance_state("foo");
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT));
    let action = map_event_pub(ev, &s);
    assert!(
        matches!(
            action,
            Some(Action::OpenPackBrowser {
                kind: PackKind::Resource,
                ..
            })
        ),
        "expected OpenPackBrowser{{Resource}}; got {action:?}"
    );
    // State transition.
    let mut s2 = pack_instance_state("foo");
    let effects = update(
        &mut s2,
        Action::OpenPackBrowser {
            slug: "foo".into(),
            kind: PackKind::Resource,
        },
    );
    assert!(
        matches!(
            s2.active_view,
            ActiveView::PackBrowser {
                kind: PackKind::Resource,
                ..
            }
        ),
        "active_view should be PackBrowser(Resource)"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::SearchPacks {
                kind: PackKind::Resource,
                ..
            }
        )),
        "should emit SearchPacks(Resource)"
    );
}

#[test]
#[allow(non_snake_case)]
fn test_uppercase_S_opens_shader_pack_browser() {
    use ichr::tui::run::map_event_pub;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    let s = pack_instance_state("foo");
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT));
    let action = map_event_pub(ev, &s);
    assert!(
        matches!(
            action,
            Some(Action::OpenPackBrowser {
                kind: PackKind::Shader,
                ..
            })
        ),
        "expected OpenPackBrowser{{Shader}}; got {action:?}"
    );
    let mut s2 = pack_instance_state("foo");
    let effects = update(
        &mut s2,
        Action::OpenPackBrowser {
            slug: "foo".into(),
            kind: PackKind::Shader,
        },
    );
    assert!(
        matches!(
            s2.active_view,
            ActiveView::PackBrowser {
                kind: PackKind::Shader,
                ..
            }
        ),
        "active_view should be PackBrowser(Shader)"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::SearchPacks {
                kind: PackKind::Shader,
                ..
            }
        )),
        "should emit SearchPacks(Shader)"
    );
}

#[test]
fn test_lowercase_r_still_opens_rename_inline() {
    // D-LOCK keybind-conflict-resolution: lowercase 'r' must remain OpenRenameInline.
    use ichr::tui::run::map_event_pub;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    let s = pack_instance_state("foo");
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &s);
    assert!(
        matches!(action, Some(Action::OpenRenameInline { .. })),
        "lowercase 'r' must still dispatch OpenRenameInline; got {action:?}"
    );
    assert!(
        !matches!(action, Some(Action::OpenPackBrowser { .. })),
        "lowercase 'r' must NOT dispatch OpenPackBrowser"
    );
}

#[test]
fn test_lowercase_s_running_still_stops_instance() {
    // D-LOCK keybind-conflict-resolution: lowercase 's' must remain StopInstance when running.
    use ichr::tui::run::map_event_pub;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    let mut s = pack_instance_state("foo");
    s.running_instances
        .insert("foo".into(), CancellationToken::new());
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &s);
    assert!(
        matches!(action, Some(Action::StopInstance { .. })),
        "lowercase 's' with running instance must dispatch StopInstance; got {action:?}"
    );
    assert!(
        !matches!(action, Some(Action::OpenPackBrowser { .. })),
        "lowercase 's' must NOT dispatch OpenPackBrowser"
    );
}

#[test]
fn test_lowercase_s_not_running_is_no_op() {
    // lowercase 's' on non-running instance = no-op (pre-existing Phase 3 behavior).
    use ichr::tui::run::map_event_pub;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    let s = pack_instance_state("foo");
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &s);
    assert!(
        action.is_none(),
        "lowercase 's' on non-running instance should be no-op; got {action:?}"
    );
}

#[test]
#[allow(non_snake_case)]
fn test_uppercase_D_inside_resource_browser_opens_drop_modal_with_resource_kind() {
    // D inside PackBrowser(Resource) → PackDropPathInput{kind=Resource}.
    let mut s = AppState {
        active_view: ActiveView::PackBrowser {
            slug: "foo".into(),
            kind: PackKind::Resource,
            search: String::new(),
            is_searching: false,
            fetch_state: ichr::mods::types::ModBrowserFetchState::Ready,
            results: Vec::new(),
            selected: 0,
            scroll_offset: 0,
        },
        ..AppState::default()
    };
    let effects = update(
        &mut s,
        Action::PackDropPathOpen {
            slug: "foo".into(),
            kind: PackKind::Resource,
        },
    );
    assert!(
        matches!(
            s.active_view,
            ActiveView::PackDropPathInput {
                kind: PackKind::Resource,
                ..
            }
        ),
        "active_view should be PackDropPathInput(Resource)"
    );
    assert!(effects.is_empty());
}

#[test]
#[allow(non_snake_case)]
fn test_uppercase_D_inside_shader_browser_opens_drop_modal_with_shader_kind() {
    let mut s = AppState {
        active_view: ActiveView::PackBrowser {
            slug: "bar".into(),
            kind: PackKind::Shader,
            search: String::new(),
            is_searching: false,
            fetch_state: ichr::mods::types::ModBrowserFetchState::Ready,
            results: Vec::new(),
            selected: 0,
            scroll_offset: 0,
        },
        ..AppState::default()
    };
    let effects = update(
        &mut s,
        Action::PackDropPathOpen {
            slug: "bar".into(),
            kind: PackKind::Shader,
        },
    );
    assert!(
        matches!(
            s.active_view,
            ActiveView::PackDropPathInput {
                kind: PackKind::Shader,
                ..
            }
        ),
        "active_view should be PackDropPathInput(Shader)"
    );
    assert!(effects.is_empty());
}

#[test]
fn test_pack_drop_path_cancel_returns_to_browser() {
    // PackDropPathCancel from PackDropPathInput{Resource} → PackBrowser{Resource}.
    let mut s = AppState {
        active_view: ActiveView::PackDropPathInput {
            slug: "foo".into(),
            kind: PackKind::Resource,
            buffer: String::new(),
            error: None,
        },
        ..AppState::default()
    };
    let effects = update(&mut s, Action::PackDropPathCancel);
    assert!(
        matches!(s.active_view, ActiveView::PackBrowser { kind: PackKind::Resource, slug, .. } if slug == "foo"),
        "should return to PackBrowser(Resource, foo)"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::SearchPacks {
                kind: PackKind::Resource,
                ..
            }
        )),
        "should emit SearchPacks to repopulate browser"
    );
}

#[test]
fn test_lowercase_m_enters_installed_mods() {
    // `m` on InstanceList → InstalledModsList (existing Phase 8 behavior, NOT InstalledPacksList).
    use ichr::tui::run::map_event_pub;
    use ratatui::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    let s = pack_instance_state("foo");
    let ev = CtEvent::Key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    let action = map_event_pub(ev, &s);
    let mut s2 = pack_instance_state("foo");
    if let Some(a) = action {
        let _ = update(&mut s2, a);
    }
    assert!(
        matches!(s2.active_view, ActiveView::InstalledModsList { .. }),
        "m should enter InstalledModsList (Mod kind); got {:?}",
        s2.active_view
    );
    assert!(
        !matches!(s2.active_view, ActiveView::InstalledPacksList { .. }),
        "m must NOT enter InstalledPacksList"
    );
}

#[test]
fn test_tab_from_installed_mods_cycles_to_resource() {
    let mut s = AppState {
        active_view: ActiveView::InstalledModsList {
            slug: "foo".into(),
            mods: Vec::new(),
            selected: 0,
        },
        ..AppState::default()
    };
    let effects = update(&mut s, Action::InstalledPacksCycleKind);
    assert!(
        matches!(
            s.active_view,
            ActiveView::InstalledPacksList {
                kind: PackKind::Resource,
                ..
            }
        ),
        "Tab from InstalledMods should cycle to InstalledPacksList(Resource)"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::FetchInstalledPacks {
                kind: PackKind::Resource,
                ..
            }
        )),
        "should emit FetchInstalledPacks(Resource)"
    );
}

#[test]
fn test_tab_from_resource_cycles_to_shader() {
    let mut s = AppState {
        active_view: ActiveView::InstalledPacksList {
            slug: "foo".into(),
            kind: PackKind::Resource,
            packs: Vec::new(),
            selected: 0,
            transient_status: None,
        },
        ..AppState::default()
    };
    let effects = update(&mut s, Action::InstalledPacksCycleKind);
    assert!(
        matches!(
            s.active_view,
            ActiveView::InstalledPacksList {
                kind: PackKind::Shader,
                ..
            }
        ),
        "Tab from Resource should cycle to InstalledPacksList(Shader)"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::FetchInstalledPacks {
                kind: PackKind::Shader,
                ..
            }
        )),
        "should emit FetchInstalledPacks(Shader)"
    );
}

#[test]
fn test_tab_from_shader_cycles_back_to_mods() {
    let mut s = AppState {
        active_view: ActiveView::InstalledPacksList {
            slug: "foo".into(),
            kind: PackKind::Shader,
            packs: Vec::new(),
            selected: 0,
            transient_status: None,
        },
        ..AppState::default()
    };
    let effects = update(&mut s, Action::InstalledPacksCycleKind);
    assert!(
        matches!(s.active_view, ActiveView::InstalledModsList { .. }),
        "Tab from Shader should cycle back to InstalledModsList"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FetchInstalledMods { .. })),
        "should emit FetchInstalledMods"
    );
}

#[test]
fn test_e_on_resource_row_dispatches_toggle() {
    let row = pack_row("r1", "cool-pack");
    let mut s = AppState {
        active_view: ActiveView::InstalledPacksList {
            slug: "foo".into(),
            kind: PackKind::Resource,
            packs: vec![row],
            selected: 0,
            transient_status: None,
        },
        ..AppState::default()
    };
    let effects = update(&mut s, Action::TogglePackEnabled);
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::TogglePackEnabledEff {
                kind: PackKind::Resource,
                ..
            }
        )),
        "TogglePackEnabled on Resource should emit TogglePackEnabledEff(Resource); got {effects:?}"
    );
}

#[test]
fn test_e_on_shader_row_dispatches_shader_toggle_notice() {
    let mut s = AppState {
        active_view: ActiveView::InstalledPacksList {
            slug: "foo".into(),
            kind: PackKind::Shader,
            packs: Vec::new(),
            selected: 0,
            transient_status: None,
        },
        ..AppState::default()
    };
    let effects = update(&mut s, Action::ShaderToggleNotice);
    assert!(effects.is_empty());
    if let ActiveView::InstalledPacksList {
        transient_status, ..
    } = &s.active_view
    {
        assert!(
            transient_status.as_deref()
                == Some("Shaders cannot be toggled -- use Iris/OptiFine in-game"),
            "transient_status should be set to shader notice; got {transient_status:?}"
        );
    } else {
        panic!("active_view changed unexpectedly");
    }
}

#[test]
fn test_x_on_any_pack_kind_opens_confirm() {
    for kind in [PackKind::Resource, PackKind::Shader] {
        let row = pack_row("r1", "pack");
        let mut s = AppState {
            active_view: ActiveView::InstalledPacksList {
                slug: "foo".into(),
                kind,
                packs: vec![row],
                selected: 0,
                transient_status: None,
            },
            ..AppState::default()
        };
        let effects = update(&mut s, Action::OpenUninstallPackConfirm);
        assert!(effects.is_empty());
        assert!(
            matches!(s.active_view, ActiveView::UninstallPackConfirm { kind: k, .. } if k == kind),
            "x should open UninstallPackConfirm with kind={kind:?}"
        );
    }
}

#[test]
fn test_pack_browser_search_loaded_with_matching_slug_kind_populates_results() {
    let hits = vec![ModrinthSearchHit {
        project_id: "p1".into(),
        slug: "cool".into(),
        title: "Cool Pack".into(),
        description: "nice".into(),
        downloads: 100,
        already_installed: false,
        icon_url: None,
    }];
    let mut s = AppState {
        active_view: ActiveView::PackBrowser {
            slug: "foo".into(),
            kind: PackKind::Resource,
            search: String::new(),
            is_searching: false,
            fetch_state: ichr::mods::types::ModBrowserFetchState::Loading,
            results: Vec::new(),
            selected: 0,
            scroll_offset: 0,
        },
        ..AppState::default()
    };
    let _ = update(
        &mut s,
        Action::PackBrowserSearchLoaded {
            slug: "foo".into(),
            kind: PackKind::Resource,
            hits: hits.clone(),
        },
    );
    if let ActiveView::PackBrowser {
        results,
        fetch_state,
        ..
    } = &s.active_view
    {
        assert_eq!(results.len(), 1, "results should be populated");
        assert_eq!(*fetch_state, ichr::mods::types::ModBrowserFetchState::Ready);
    } else {
        panic!("active_view changed unexpectedly");
    }
}

#[test]
fn test_pack_browser_search_loaded_with_mismatched_slug_does_not_overwrite() {
    // Slug-match-guard: mismatched slug → results stay empty.
    let mut s = AppState {
        active_view: ActiveView::PackBrowser {
            slug: "foo".into(),
            kind: PackKind::Resource,
            search: String::new(),
            is_searching: false,
            fetch_state: ichr::mods::types::ModBrowserFetchState::Loading,
            results: Vec::new(),
            selected: 0,
            scroll_offset: 0,
        },
        ..AppState::default()
    };
    let _ = update(
        &mut s,
        Action::PackBrowserSearchLoaded {
            slug: "bar".into(), // mismatched slug
            kind: PackKind::Resource,
            hits: vec![ModrinthSearchHit {
                project_id: "p1".into(),
                slug: "x".into(),
                title: "X".into(),
                description: "x".into(),
                downloads: 0,
                already_installed: false,
                icon_url: None,
            }],
        },
    );
    if let ActiveView::PackBrowser { results, .. } = &s.active_view {
        assert!(
            results.is_empty(),
            "mismatched slug should NOT overwrite results"
        );
    } else {
        panic!("active_view changed unexpectedly");
    }
}

#[test]
fn test_pack_browser_search_loaded_with_mismatched_kind_does_not_overwrite() {
    // Kind-match-guard: mismatched kind → results stay empty.
    let mut s = AppState {
        active_view: ActiveView::PackBrowser {
            slug: "foo".into(),
            kind: PackKind::Resource,
            search: String::new(),
            is_searching: false,
            fetch_state: ichr::mods::types::ModBrowserFetchState::Loading,
            results: Vec::new(),
            selected: 0,
            scroll_offset: 0,
        },
        ..AppState::default()
    };
    let _ = update(
        &mut s,
        Action::PackBrowserSearchLoaded {
            slug: "foo".into(),
            kind: PackKind::Shader, // mismatched kind
            hits: vec![ModrinthSearchHit {
                project_id: "p1".into(),
                slug: "x".into(),
                title: "X".into(),
                description: "x".into(),
                downloads: 0,
                already_installed: false,
                icon_url: None,
            }],
        },
    );
    if let ActiveView::PackBrowser { results, .. } = &s.active_view {
        assert!(
            results.is_empty(),
            "mismatched kind should NOT overwrite results"
        );
    } else {
        panic!("active_view changed unexpectedly");
    }
}

#[test]
fn test_pack_installed_action_clears_running_pack_jobs_entry() {
    let mut s = AppState::default();
    s.running_pack_jobs
        .insert(("foo".into(), PackKind::Resource), CancellationToken::new());
    assert_eq!(
        s.running_pack_jobs.len(),
        1,
        "precondition: one entry in running_pack_jobs"
    );
    let _ = update(
        &mut s,
        Action::PackInstalled {
            slug: "foo".into(),
            kind: PackKind::Resource,
        },
    );
    assert!(
        s.running_pack_jobs.is_empty(),
        "PackInstalled should clear the running_pack_jobs entry"
    );
}
