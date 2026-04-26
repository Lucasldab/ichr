---
phase: 06-fabric-and-quilt-modloaders
plan: "06"
subsystem: tui
tags: [elm-architecture, action, effect, active-view, loader, state-machine, reducer]

# Dependency graph
requires:
  - phase: 06-05
    provides: LoaderService facade + install_loader stub
  - phase: 06-01
    provides: LoaderType, LoaderVersionEntry, LoaderInfo types
  - phase: 06-02
    provides: InstanceManifest.loader field + ModloaderKind enum

provides:
  - Five new ActiveView variants (LoaderPickerModal, LoaderVersionPickerModal,
    LoaderInstallProgressModal, LoaderInstallFailedModal, LoaderSwitchConfirm)
  - 19 new Action variants covering the full loader install lifecycle
  - 4 new Effect variants (FetchLoaderVersions, InstallLoader, CancelLoaderInstall, RemoveLoader)
  - LoaderPickerRow enum (None/Fabric/Quilt)
  - AppState.running_loader_installs HashMap (mirrors running_instances)
  - loader_versions_visible_indices() helper (Open Question 3 lock enforced)
  - loader_label_short() helper for switch dialog labels
  - 16 new unit tests covering all major action paths

affects: [06-07-tui-views, 06-08-tui-wiring]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Loader picker mirrors Java picker (Phase 5 analog): 3-row enum picker → version picker submenu"
    - "running_loader_installs mirrors running_instances: CancellationToken per in-flight install"
    - "loader_versions_visible_indices: pure filter fn, Quilt ignores filter_stable_only (Open Q3 lock)"
    - "SwitchConfirm dialog uses 'none' as sentinel to_loader string for RemoveLoader effect"
    - "ConfirmLoaderSwitch parses 'kind:version' format from to_loader field"

key-files:
  created: []
  modified:
    - src/tui/app.rs
    - src/tui/run.rs
    - src/tui/view.rs

key-decisions:
  - "19 Action variants (not 12 as originally estimated): progress + cancel + dismiss variants were needed for full lifecycle coverage"
  - "Open Question 3 lock: loader_versions_visible_indices always returns all indices for Quilt regardless of filter_stable_only; assertion test enforces this invariant"
  - "LoaderPickerSelect for row 2 (Quilt) sets filter_stable_only=false by default, matching Open Q3 lock"
  - "LoaderVersionSelect uses filtered visible_indices to map selected index to original versions array index"
  - "Placeholder match arms added to run.rs and view.rs (Rule 3 fix) to maintain compilation; 06-08 replaces them"
  - "step_index in LoaderInstallProgress arm derived from pct bands (0-33/34-66/67-99/100) to keep update() pure"

patterns-established:
  - "Loader picker (3-row: None/Fabric/Quilt) → FetchLoaderVersions effect → LoaderVersionPickerModal"
  - "LoaderSwitchConfirm to_loader='none' → RemoveLoader; 'kind:version' → InstallLoader"
  - "Cancel path: CancelLoaderInstall calls token.cancel() synchronously in update() then emits Effect::CancelLoaderInstall for any async cleanup"

requirements-completed: [LOAD-01, LOAD-02, LOAD-05, LOAD-06]

# Metrics
duration: 6min
completed: 2026-04-26
---

# Phase 06 Plan 06: TUI State Machine Summary

**Elm-architecture state contract for Fabric/Quilt loader install: 5 ActiveView variants, 19 Action variants, 4 Effect variants, with pure update() reducer and 16 unit tests**

## Performance

- **Duration:** 6 min
- **Started:** 2026-04-26T08:04:24Z
- **Completed:** 2026-04-26T08:10:18Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments

- Extended `src/tui/app.rs` with all Phase 6 state machine elements: 5 ActiveView variants, 19 Action variants, 4 Effect variants, and `LoaderPickerRow` enum
- Implemented all 19 `update()` arms as pure reducers — no I/O, only state mutation and Vec<Effect> return
- Enforced Open Question 3 lock: `loader_versions_visible_indices` returns all Quilt versions regardless of `filter_stable_only` — asserted by `test_loader_versions_visible_indices_quilt_shows_all_when_filter_on`
- 26 total tests pass (16 new + 10 existing); no pre-existing tests broken

## Task Commits

1. **Task 1: Add ActiveView/Effect variants, LoaderPickerRow, AppState.running_loader_installs** - `14ec3bc` (feat)
2. **Task 2: Add 19 Action variants + update() arms + loader state helpers** - `e3ca35b` (feat)

**Plan metadata:** (committed with this SUMMARY)

## Files Created/Modified

- `src/tui/app.rs` — All Phase 6 state machine elements + 16 new unit tests
- `src/tui/run.rs` — Placeholder match arms for 5 new ActiveView variants and 4 new Effect variants (06-08 wires)
- `src/tui/view.rs` — Placeholder match arms for 5 new ActiveView variants (06-07 wires)

## Decisions Made

- **19 vs 12 actions:** The plan frontmatter estimated 12 actions but the full lifecycle (progress + cancel + dismiss + switch confirm/cancel) requires 19. All 19 are needed and documented.
- **Quilt filter_stable_only=false default:** When the user picks Quilt in LoaderPickerSelect, filter_stable_only is set to false (not true like Fabric), matching the Open Q3 lock that Quilt has no stable boolean.
- **step_index from pct bands:** The LoaderInstallProgress arm derives step_index from percentage bands (0-33%=1, 34-66%=2, 67-99%=3, 100%=4) to keep update() pure without needing separate step-count parameters.
- **Placeholder arms in run.rs/view.rs:** Added to maintain compilation per Rule 3 (blocking issue). The plan notes these wiring points are for 06-07 and 06-08 respectively.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added placeholder match arms in run.rs and view.rs**
- **Found during:** Task 1 (adding new ActiveView and Effect variants)
- **Issue:** Rust exhaustive pattern matching requires all variants to be covered; adding variants to `ActiveView` and `Effect` broke compilation in run.rs (event mapper + execute_effects) and view.rs (view dispatcher)
- **Fix:** Added wildcard-group match arms returning None/`{}` in run.rs (map_event and execute_effects) and view.rs (view dispatcher). These are intentional stubs labeled "wired in 06-07/06-08"
- **Files modified:** src/tui/run.rs, src/tui/view.rs
- **Verification:** `cargo build` passes cleanly after addition
- **Committed in:** 14ec3bc (Task 1 commit)

**2. [Rule 1 - Bug] Fixed clippy field_reassign_with_default lint in test**
- **Found during:** Task 2 (clippy pass after tests)
- **Issue:** `test_dismiss_loader_install_failed_returns_to_list` used `AppState::default()` then immediately reassigned `active_view`, triggering clippy `field_reassign_with_default` lint
- **Fix:** Changed to struct update syntax `AppState { active_view: ..., ..AppState::default() }`
- **Files modified:** src/tui/app.rs (test only)
- **Verification:** `cargo clippy --all-targets -- -D warnings` passes
- **Committed in:** e3ca35b (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 Rule 3 blocking, 1 Rule 1 bug)
**Impact on plan:** Both auto-fixes necessary for correct compilation and clean clippy. No scope creep.

## Issues Encountered

None — plan executed as specified with the two auto-fixes noted above.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- All state machine contracts are codified in `src/tui/app.rs`
- 06-07 (TUI views) can read ActiveView variant shapes directly from this file
- 06-08 (TUI wiring) can read update() routing and Effect variant shapes from this file
- Placeholder arms in run.rs and view.rs are clearly labeled for replacement

## Self-Check: PASSED

- `src/tui/app.rs` exists and contains all required variants
- Commit `14ec3bc` exists (Task 1)
- Commit `e3ca35b` exists (Task 2)
- `cargo test --lib -- tui::app::tests`: 26 passed, 0 failed
- `cargo build`: clean
- `cargo clippy --all-targets -- -D warnings`: clean

---
*Phase: 06-fabric-and-quilt-modloaders*
*Completed: 2026-04-26*
