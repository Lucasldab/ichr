---
phase: "06"
plan: "09"
name: integration-validation
status: partial
tasks_completed: 2/3
subsystem: loader
tags: [validation, integration, live-test, uat, checkpoint]
requires: [06-01, 06-02, 06-03, 06-04, 06-05, 06-06, 06-07, 06-08]
provides: [live-test-fabric, live-test-quilt, human-uat-checklist]
affects: [tests/loader_live.rs, 06-VALIDATION.md, 06-HUMAN-UAT.md]
tech_stack:
  added: []
  patterns: [ignore-gated-live-test, dynamic-version-picker]
key_files:
  created: [tests/loader_live.rs, .planning/phases/06-fabric-and-quilt-modloaders/06-HUMAN-UAT.md]
  modified: [.planning/phases/06-fabric-and-quilt-modloaders/06-VALIDATION.md]
decisions:
  - "JobId(0) used as ad-hoc sentinel — JobId is a tuple struct with no Default impl"
  - "Quilt live test fetches versions[0] dynamically to resist beta version churn"
  - "wave_0_complete: true set after confirming all 21 Wave 0 files exist on disk"
  - "nyquist_compliant: false retained until Task 3 human checkpoint approval"
metrics:
  duration_seconds: 149
  completed_date: 2026-04-26
  file_count: 3
---

# Phase 6 Plan 9: Integration Validation Summary

**One-liner:** Live #[ignore]-gated integration tests for Fabric + Quilt loader installs against real meta APIs, plus 9-check human UAT script and populated validation document.

## Tasks

| Task | Name | Commit | Status |
|------|------|--------|--------|
| 1 | Write tests/loader_live.rs with Fabric + Quilt #[ignore] live tests | fec905e | complete |
| 2 | Populate 06-VALIDATION.md task IDs and write 06-HUMAN-UAT.md | 3988e49 | complete |
| 3 | Human checkpoint — run UAT, verify nyquist gate, flip nyquist_compliant | — | awaiting human |

## What Was Built

**Task 1 — `tests/loader_live.rs`**

Two `#[tokio::test] #[ignore]` integration tests:

- `live_fabric_install_1_21_4`: pins MC `1.21.4` + Fabric Loader `0.16.9`. Creates a tempdir with isolated `AppPaths`, writes a vanilla `InstanceManifest`, calls `LoaderService::install_loader`, then asserts the version JSON exists at `versions/fabric-loader-0.16.9-1.21.4/`, the primary `net.fabricmc:fabric-loader:0.16.9` JAR is present under `libraries/`, and `manifest.loader.version_id` matches the profile ID verbatim (Pitfall 7 invariant).

- `live_quilt_install_1_21_4`: fetches the Quilt loader version list dynamically (`versions[0]`) to resist beta version churn, installs via `LoaderService::install_loader`, and asserts the same invariants for the Quilt path (no SHA1 — existence check only).

Both tests compile and are verified with `cargo build --tests && cargo clippy --all-targets -- -D warnings`.

**Task 2 — Documentation artifacts**

- `06-VALIDATION.md`: confirmed 0 remaining `06-XX-XX` placeholders; set `wave_0_complete: true` after verifying all 21 Wave 0 files exist on disk; `nyquist_compliant` remains `false` pending Task 3 approval.

- `06-HUMAN-UAT.md`: 9-check manual reproduction script covering:
  1. Instance list `/L` keybind display
  2. Loader picker modal (open + Esc cancel)
  3. Install Fabric end-to-end (progress modal + status cell)
  4. Switch Fabric version (currently-installed marker + confirm)
  5. Switch loader type Fabric→Quilt (type-change warning)
  6. Remove loader (revert to vanilla)
  7. Cancel install mid-stream (atomicity invariant)
  8. Install failure UX (offline → LoaderInstallFailedModal)
  9. Loader-status cell at 80-col + 120-col terminal widths

## Deviations from Plan

### Auto-fixed Issues

None — plan executed exactly as written.

**Note on validation file:** The plan states "replace any remaining `06-XX-XX` placeholders" — confirmed 0 were present (the planning phase had already populated all task IDs). `wave_0_complete` was flipped to `true` because all 21 Wave 0 files were confirmed present on disk after the 8 preceding plans completed.

## Known Stubs

None. Both tests are structurally complete with real assertions; they are gated by `#[ignore]` because they require internet access, not because they are stubs.

## Self-Check: PASSED

- `tests/loader_live.rs` exists: FOUND
- `06-VALIDATION.md` wave_0_complete: FOUND (set to true)
- `06-HUMAN-UAT.md` exists: FOUND
- Commit fec905e exists: FOUND
- Commit 3988e49 exists: FOUND
- `grep -c '06-XX-XX' 06-VALIDATION.md` returns 0: PASSED
- 9 UAT checks in HUMAN-UAT.md: PASSED (count=9)
