---
phase: 06-fabric-and-quilt-modloaders
plan: "02"
subsystem: domain
tags: [instance-manifest, loader, serde, fabric, quilt, domain]

requires:
  - phase: 06-01-loader-scaffold
    provides: LoaderInfo struct in src/loader/types.rs

provides:
  - InstanceManifest.loader: Option<LoaderInfo> field with forward-compat serde annotations
  - Import of LoaderInfo into domain/instance.rs
  - 4 unit tests covering backward-compat, None omission, Fabric roundtrip, Quilt roundtrip

affects:
  - 06-05-loader-service (reads/writes loader field via InstanceManifest)
  - 06-07-instance-ui (displays loader info from manifest)
  - phase-03-launch (picks up loader version_id from manifest for inheritsFrom walker)

tech-stack:
  added: []
  patterns:
    - "Optional fields on InstanceManifest use #[serde(default, skip_serializing_if = \"Option::is_none\")] — same pattern as java_override"
    - "Forward-compat: never use deny_unknown_fields; missing optional fields default to None"

key-files:
  created: []
  modified:
    - src/domain/instance.rs
    - tests/instance_domain.rs

key-decisions:
  - "loader field placed after java_override to keep optional fields grouped together"
  - "Integration test struct literals updated with loader: None to maintain exhaustive construction (no struct update syntax ..Default::default())"

patterns-established:
  - "Pattern: InstanceManifest optional field = #[serde(default, skip_serializing_if = \"Option::is_none\")] + None in ::new()"

requirements-completed: [LOAD-01, LOAD-02, LOAD-05]

duration: 8min
completed: 2026-04-26
---

# Phase 06 Plan 02: Domain Loader Field Summary

**InstanceManifest gains `loader: Option<LoaderInfo>` with forward-compat serde, enabling LoaderService (06-05) to persist the active Fabric/Quilt loader version**

## Performance

- **Duration:** 8 min
- **Started:** 2026-04-26T00:00:00Z
- **Completed:** 2026-04-26T00:08:00Z
- **Tasks:** 1
- **Files modified:** 2

## Accomplishments

- Added `pub loader: Option<LoaderInfo>` to `InstanceManifest` with `#[serde(default, skip_serializing_if = "Option::is_none")]` — same pattern as existing `java_override` field
- Added `use crate::loader::types::LoaderInfo;` import without circular dependency (loader::types already imports ModloaderKind from domain::instance)
- Initialized `loader: None` in `InstanceManifest::new()` constructor
- Added 4 TDD tests: backward-compat (legacy JSON without loader field), None omission, Fabric roundtrip, Quilt roundtrip
- Fixed 5 integration test struct literals in `tests/instance_domain.rs` that required exhaustive construction (Rule 3 auto-fix)

## Task Commits

1. **Task 1: Add loader field to InstanceManifest with forward-compat tests** - `1055f54` (feat)

**Plan metadata:** (docs commit follows)

## Files Created/Modified

- `src/domain/instance.rs` - Added LoaderInfo import, loader field, loader: None init, 4 new unit tests
- `tests/instance_domain.rs` - Added loader: None to all 5 InstanceManifest struct literals

## Decisions Made

- Placed `loader` field after `java_override` to keep optional instance-configuration fields grouped
- Updated integration tests with `loader: None` rather than using struct update syntax, maintaining explicit exhaustive construction (easier to spot when new fields are added)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed missing `loader` field in integration test struct literals**
- **Found during:** Task 1 (clippy --all-targets verification)
- **Issue:** `tests/instance_domain.rs` constructs `InstanceManifest` with exhaustive struct syntax; adding a new field to the struct causes compile errors in all 5 literal expressions
- **Fix:** Added `loader: None` to all 5 `InstanceManifest { ... }` struct literals in the integration test file
- **Files modified:** `tests/instance_domain.rs`
- **Verification:** `cargo clippy --all-targets -- -D warnings` passes cleanly
- **Committed in:** `1055f54` (part of task commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Necessary exhaustive struct fix. No scope creep.

## Issues Encountered

None beyond the integration test struct exhaustiveness fix (handled as Rule 3 auto-fix).

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- `InstanceManifest.loader` field is live; `LoaderService::install_loader` (06-05) can now persist the active loader
- The `version_id` field on `LoaderInfo` is what the launch pipeline (Phase 3) uses for `inheritsFrom` resolution
- No blockers for 06-03 (Fabric client) or 06-04 (Quilt client)

---
*Phase: 06-fabric-and-quilt-modloaders*
*Completed: 2026-04-26*
