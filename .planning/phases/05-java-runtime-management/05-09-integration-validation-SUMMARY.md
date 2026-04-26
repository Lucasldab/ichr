---
phase: 05
plan: "09"
subsystem: integration
status: partial
tags: [integration, live, validation, human-checkpoint, nyquist-gate]
requirements: [JAVA-01, JAVA-02, JAVA-03, JAVA-04, JAVA-05]
dependency_graph:
  requires: [05-08]
  provides: [jre-live-smoke-test, phase5-validation-matrix]
  affects:
    - tests/jre_live.rs
    - .planning/phases/05-java-runtime-management/05-VALIDATION.md
tech_stack:
  added: []
  patterns: [ignore-gated-live-test, tempfile-isolation, validation-matrix]
key_files:
  created:
    - tests/jre_live.rs
  modified:
    - .planning/phases/05-java-runtime-management/05-VALIDATION.md
decisions:
  - "live_mojang_jre_download_java_runtime_delta skips gracefully on platforms with no Mojang key (aarch64 Linux)"
  - "Synthetic archive fixtures generated in-memory in test modules (not on-disk) per 05-04 decision — Wave 0 fixture check left partial"
  - "nyquist_compliant left false — requires human smoke launch to flip (Task 3 awaiting)"
metrics:
  duration_minutes: 3
  completed_date: "2026-04-26"
  tasks_completed: 2
  tasks_total: 3
  files_changed: 2
  new_tests: 1
---

# Phase 5 Plan 09: Integration Validation Summary

**One-liner:** `#[ignore]`-gated live Mojang JRE smoke test + 17-row per-task validation matrix closing Phase 5; nyquist gate pending human end-to-end smoke launch.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Live Mojang JRE download smoke test | b465e7a | tests/jre_live.rs |
| 2 | Populate 05-VALIDATION.md per-task map + Wave 0 check-ins | 97dbe3f | .planning/phases/05-java-runtime-management/05-VALIDATION.md |

## Task 3: AWAITING HUMAN VERIFICATION

Task 3 is a blocking `checkpoint:human-verify` gate. The plan instructs:

1. Build: `cargo build --release`
2. Run: `cargo run --release`
3. Create a new instance with version **1.20.4** (uses `java-runtime-gamma`, Java 17)
4. Launch the instance — observe JRE auto-download (~200 MB, ~20–60 seconds)
5. Confirm Minecraft title screen appears
6. Test per-instance override (press `j`, select a system Java >= 17, relaunch)
7. Test mismatch validation (edit instance.json to set Java 8 override; expect error modal)

After all steps pass: flip `nyquist_compliant: true` and `status: validated` in the frontmatter of `.planning/phases/05-java-runtime-management/05-VALIDATION.md`.

## What Was Built

### Task 1: tests/jre_live.rs (64 lines)

- `live_mojang_jre_download_java_runtime_delta` — `#[tokio::test] #[ignore]` live test
- Creates a `TempDir`, constructs `AppPaths::with_roots` pointing to it
- Detects current platform via `Arch::current()` / `OsName::current()` + `mojang_platform_key`
- Gracefully skips if no Mojang platform key (aarch64 Linux → "use Adoptium fallback test")
- Calls `JavaService::install_mojang(&paths, "java-runtime-delta")`
- Asserts `exe.is_file()`; on unix asserts `permissions().mode() & 0o111 != 0`
- Default run: `0 passed; 0 failed; 1 ignored`
- Opt-in run: `cargo test --test jre_live -- --ignored --nocapture`

### Task 2: 05-VALIDATION.md

- 17 rows in the per-task map covering every substantive task in plans 05-01 through 05-09
- All rows have `status: pass` except 05-09-01 (live test, `pending (human opt-in)`)
- Wave 0 requirements: 4 of 5 ticked; the synthetic archive fixture checkbox remains partial with an explanatory note (in-memory generation decision from plan 05-04)
- `nyquist_compliant: false` pending Task 3 human gate

## Deviations from Plan

None — both auto tasks executed exactly as written.

## Known Stubs

None — Task 1 and Task 2 are fully complete. Task 3 (nyquist flip) is explicitly gated on human verification.

## Threat Surface Scan

No new network endpoints, auth paths, file access patterns, or schema changes at trust boundaries:
- `tests/jre_live.rs` uses `TempDir` (isolated) and calls existing `JavaService::install_mojang`
- `05-VALIDATION.md` is documentation-only

Threat model dispositions from the plan:
- T-05-09-01 (live test cost): accepted — `#[ignore]` by default, opt-in only
- T-05-09-02 (live test data): accepted — TempDir isolated, no effect on user data_dir
- T-05-09-03 (human checkpoint logs): accepted — standard TUI surface, no secrets

## Self-Check: PASSED

- `tests/jre_live.rs` exists
- `cargo test --test jre_live` shows: `0 passed; 0 failed; 1 ignored`
- `cargo test --test jre_live -- --list` shows: `live_mojang_jre_download_java_runtime_delta: test`
- 17 rows in 05-VALIDATION.md `| 05-` pattern
- Commit b465e7a exists
- Commit 97dbe3f exists
