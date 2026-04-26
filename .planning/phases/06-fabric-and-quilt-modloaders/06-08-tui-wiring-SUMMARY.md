---
phase: 06-fabric-and-quilt-modloaders
plan: "08"
subsystem: tui
tags: [ratatui, loader, keybind, effect, runtime, tui-smoke, fabric, quilt]

# Dependency graph
requires:
  - phase: 06-fabric-and-quilt-modloaders
    provides: "LoaderService (06-05), app.rs state+actions+effects (06-06), loader modal views (06-07)"

provides:
  - "Arc<LoaderService> constructed in run() and threaded through execute_effects"
  - "4 wired Effect arms: FetchLoaderVersions, InstallLoader, CancelLoaderInstall, RemoveLoader"
  - "Uppercase L keybind in InstanceList map_event dispatches OpenLoaderPicker; blocked on running+in-flight"
  - "5 loader modal map_event branches routing to their respective view fns"
  - "instance_list.rs status cell renders running > {kind}:{6-char-version} > last_played_at"
  - "Block title updated to 'Instances (c/r/x/d/g/Enter/s/A/L)'"
  - "11 new tui_smoke tests covering Phase 6 keybind and state transitions"

affects: [06-09-uat, future-phases]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "TaskEvent::Progress forwarder pattern: install_loader emits via dedicated mpsc channel; forwarder task converts to Action::LoaderInstallProgress (mirrors Phase 4 AccountAuthEvent forwarder)"
    - "LoaderInstallStarted dispatched before await mirrors LaunchJobStarted single-mutation-point invariant"
    - "CancelLoaderInstall effect arm is a no-op hook — token.cancel() already happened in update()"

key-files:
  created: []
  modified:
    - src/tui/run.rs
    - src/tui/views/instance_list.rs
    - tests/tui_smoke.rs

key-decisions:
  - "CancelLoaderInstall effect arm is a no-op: token cancellation happens in update() before the effect is dispatched, mirroring KillProcess pattern"
  - "Cancelled install dispatches LoaderInstalled (not Failed) because CancelLoaderInstall handler already reset the view in update()"
  - "LoaderVersionEntry import was not needed in tui_smoke.rs — removed to keep clippy clean"
  - "Non-snake_case test function names with uppercase L suppressed via per-function #[allow(non_snake_case)] to preserve plan's grep-guard names"

patterns-established:
  - "Pattern: Progress forwarder channel — service emits TaskEvent::Progress; execute_effects spawns forwarder task converting to Action-typed events"

requirements-completed: [LOAD-01, LOAD-02, LOAD-05, LOAD-06]

# Metrics
duration: 25min
completed: 2026-04-26
---

# Phase 6 Plan 08: TUI Wiring Summary

**LoaderService wired end-to-end into the TUI runtime: Arc construction, L keybind, 5 modal routing branches, 4 effect arms with progress forwarding, and 11 smoke tests covering Phase 6 transitions**

## Performance

- **Duration:** 25 min
- **Started:** 2026-04-26T00:00:00Z
- **Completed:** 2026-04-26T00:25:00Z
- **Tasks:** 3
- **Files modified:** 3

## Accomplishments
- Constructed `Arc<LoaderService>` in `run()` alongside `Arc<JavaService>` and threaded it through both `execute_effects` call sites
- Implemented 4 new Effect arms: `FetchLoaderVersions`, `InstallLoader` (with TaskEvent forwarder), `CancelLoaderInstall` (no-op), and `RemoveLoader`
- Added uppercase `L` keybind in `map_instance_list_event` that blocks on running instances AND in-flight loader installs (T-06-20)
- Replaced 5 stub `None` loader modal arms in `map_event` with real routing to their respective view `map_*_event` fns
- Updated instance list status cell to three-way: running (BOLD) > `{kind}:{6-char-version}` > last_played_at
- Extended block title to include `/L` keybind hint
- 11 new tui_smoke tests; all 55 tests pass; build and clippy clean

## Task Commits

1. **Task 1: instance_list.rs status cell + block title** - `32d2622` (feat)
2. **Task 2: run.rs LoaderService construction, L keybind, modal routing, 4 effect arms** - `bd54268` (feat)
3. **Task 3: tui_smoke Phase 6 tests** - `5208fe3` (test)

## Files Created/Modified
- `src/tui/views/instance_list.rs` - Three-way status cell + `/L` block title
- `src/tui/run.rs` - LoaderService Arc, L keybind, modal routing, 4 effect arms + progress forwarder
- `tests/tui_smoke.rs` - 11 new Phase 6 smoke tests

## Decisions Made
- `CancelLoaderInstall` effect arm is a no-op: `token.cancel()` fires in `update()` before the effect is dispatched, so the effect has nothing left to do (mirrors `KillProcess` pattern).
- `Err(LoaderError::Cancelled)` from `install_loader` dispatches `Action::LoaderInstalled` (not `Failed`) because the `CancelLoaderInstall` handler in `update()` already reset `active_view` to `InstanceList` and cleared `running_loader_installs`.
- `RemoveLoader` failure uses `LoaderType::Fabric` as placeholder — the failure modal copy doesn't depend on loader type for remove operations.
- Non-snake_case test names with uppercase `L` suppressed via `#[allow(non_snake_case)]` per-function to keep plan's grep-guard names intact.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Removed unused `LoaderVersionEntry` import in tui_smoke.rs**
- **Found during:** Task 3 (smoke tests)
- **Issue:** The plan's code block imported `LoaderVersionEntry` but it was never used in the test bodies, causing a clippy warning that would have become an error with `-D warnings`.
- **Fix:** Removed `LoaderVersionEntry` from the import; only `LoaderType` needed.
- **Files modified:** tests/tui_smoke.rs
- **Committed in:** 5208fe3 (Task 3 commit)

**2. [Rule 1 - Bug] Fixed clippy `field_reassign_with_default` in two smoke tests**
- **Found during:** Task 3 (smoke tests)
- **Issue:** Two tests used `AppState::default()` then immediately reassigned `active_view`, which clippy flags as `field_reassign_with_default`.
- **Fix:** Changed to `AppState { active_view: ..., ..AppState::default() }` struct init pattern (same as existing tests in the file).
- **Files modified:** tests/tui_smoke.rs
- **Committed in:** 5208fe3 (Task 3 commit)

---

**Total deviations:** 2 auto-fixed (both Rule 1 — clippy compliance)
**Impact on plan:** Both fixes necessary for `-D warnings` clean build. No scope creep.

## Issues Encountered
None — plan executed cleanly after fixing clippy issues in tests.

## Known Stubs
None — all loader effect arms are fully implemented. The `LoaderType::Fabric` placeholder in the `RemoveLoader` error path is intentional (modal copy doesn't depend on loader type for remove failures) and documented as a decision.

## Threat Flags
No new security-relevant surface introduced beyond what the plan's threat model already covers.

## Next Phase Readiness
- Phase 6 end-to-end flow is complete: service, state, views, runtime, tests all wired
- Phase 6 plan 09 (UAT / manual verification) can now run `cargo run --release` and test the `/L` keybind flow
- Phase 7 (Forge/NeoForge subprocess installer) can build on the same `LoaderService` pattern

---
*Phase: 06-fabric-and-quilt-modloaders*
*Completed: 2026-04-26*
