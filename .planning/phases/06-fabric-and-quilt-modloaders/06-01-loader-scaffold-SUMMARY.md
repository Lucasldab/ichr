---
phase: 06-fabric-and-quilt-modloaders
plan: "01"
subsystem: loader
tags: [loader, fabric, quilt, maven, types, error, thiserror, serde, path-traversal]

# Dependency graph
requires:
  - phase: 05-java-runtime-management
    provides: JavaService, JavaRuntimeId — loader install uses Java for future Forge/NeoForge; Phase 6 depends on domain types
  - phase: 02-mojang-version-pipeline
    provides: safe_extract_path pattern, LIB_CONCURRENCY pattern, InstanceManifest domain types
provides:
  - LoaderType enum (Fabric/Quilt) with serde snake_case roundtrip
  - LoaderVersionEntry struct for version picker rows
  - LoaderLibrary canonical shared struct (no per-loader duplication)
  - LoaderInfo struct for InstanceManifest.loader field (Phase 06-02)
  - LoaderError thiserror enum with 7 variants covering all loader failure modes
  - maven_coord_to_path and maven_download_url pure utilities with path-traversal guard
  - Stub files for fabric.rs, quilt.rs, service.rs keeping crate buildable
affects:
  - 06-02-instance-manifest-extension (imports LoaderInfo)
  - 06-03-fabric-client (imports LoaderLibrary, LoaderError, maven utilities)
  - 06-04-quilt-client (imports LoaderLibrary, LoaderError, maven utilities)
  - 06-05-loader-service (imports all types, error, maven)

# Tech tracking
tech-stack:
  added: []
  patterns:
    - thiserror enum with &'static str loader name field for user-facing Display
    - Maven coordinate segment validation via explicit charset + .. rejection before any disk-path construction
    - Canonical shared type pattern — LoaderLibrary in types.rs, not duplicated per loader

key-files:
  created:
    - src/loader/mod.rs
    - src/loader/types.rs
    - src/loader/error.rs
    - src/loader/maven.rs
    - src/loader/fabric.rs (stub)
    - src/loader/quilt.rs (stub)
    - src/loader/service.rs (stub with LoaderService struct)
  modified:
    - src/lib.rs

key-decisions:
  - "LoaderLibrary lives in types.rs as a single nominal type shared by both Fabric and Quilt clients — no per-loader duplication"
  - "is_safe_maven_segment explicitly rejects .. sentinel in addition to charset check — dots are in [A-Za-z0-9._-] so .. passes charset but must be caught separately"
  - "LoaderService stub created in service.rs (not just doc-comment) because mod.rs re-exports it and the crate must compile after this plan"
  - "error.rs and maven.rs created in Task 1 commit alongside types.rs to satisfy Rust module system requirements — all declared pub mod submodules must resolve"

patterns-established:
  - "Maven path-traversal guard: validate each colon-separated segment against [A-Za-z0-9._-]+ AND != '..' before constructing any disk path"
  - "LoaderError Display strings use &'static str loader field for compact thiserror format strings matching UI-SPEC copywriting"

requirements-completed: [LOAD-01, LOAD-02, LOAD-06]

# Metrics
duration: 15min
completed: 2026-04-26
---

# Phase 6 Plan 01: Loader Scaffold Summary

**Pure-Rust loader module foundation: LoaderType/LoaderLibrary/LoaderInfo types, 7-variant LoaderError enum, and Maven coordinate utilities with path-traversal guard — 27 unit tests, zero I/O.**

## Performance

- **Duration:** ~15 min
- **Started:** 2026-04-26
- **Completed:** 2026-04-26
- **Tasks:** 3
- **Files modified:** 8 (7 created + 1 modified)

## Accomplishments

- Declared `pub mod loader;` in src/lib.rs and built the full module skeleton with all submodule stubs so the crate compiles immediately
- Defined all four loader domain types (LoaderType, LoaderVersionEntry, LoaderLibrary, LoaderInfo) with serde roundtrip tests — LoaderLibrary is the canonical single type shared by both Fabric and Quilt clients
- Implemented LoaderError with 7 thiserror variants plus Display strings matching UI-SPEC copywriting, and maven_coord_to_path/maven_download_url with a path-traversal guard covering `..`, `/`, `\`, and non-safe characters

## Task Commits

Each task was committed atomically:

1. **Task 1: Module skeleton + types + stubs** - `9397047` (feat)
2. **Task 2: LoaderError enum** - included in `9397047` (required for compilation — deviation documented below)
3. **Task 3: maven.rs utilities** - `5fe0196` (feat — includes Rule 1 fix for `..` traversal)

**Plan metadata:** (this commit)

## Files Created/Modified

- `src/lib.rs` — added `pub mod loader;` between `launcher` and `mojang` (alphabetical)
- `src/loader/mod.rs` — pub mod declarations + re-exports for LoaderType, LoaderError, LoaderService, LoaderInfo, LoaderLibrary, LoaderVersionEntry
- `src/loader/types.rs` — LoaderType, LoaderVersionEntry, LoaderLibrary, LoaderInfo + 7 serde roundtrip tests
- `src/loader/error.rs` — LoaderError enum (7 variants) + 7 Display tests
- `src/loader/maven.rs` — maven_coord_to_path, maven_download_url, parse_maven_coord, is_safe_maven_segment + 13 unit tests
- `src/loader/fabric.rs` — stub (doc-comment only)
- `src/loader/quilt.rs` — stub (doc-comment only)
- `src/loader/service.rs` — stub with `pub struct LoaderService;` to satisfy re-export in mod.rs

## Decisions Made

- LoaderLibrary lives in types.rs as a single canonical type — no per-loader duplication in fabric.rs/quilt.rs
- is_safe_maven_segment explicitly rejects `..` sentinel because `.` is in the allowed charset `[A-Za-z0-9._-]` so a pure charset check would pass `..`
- LoaderService stub must have the struct definition (not just a doc-comment) because mod.rs re-exports it and the crate must compile after this plan
- error.rs and maven.rs created in Task 1 commit alongside types.rs — Rust requires all declared `pub mod` submodules to exist as files before compilation

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] .. traversal segment passes is_safe_maven_segment charset check**
- **Found during:** Task 3 (maven.rs unit test run)
- **Issue:** `is_safe_maven_segment("..")` returned `true` because `.` is in the allowed charset `[A-Za-z0-9._-]`. The test `test_maven_coord_to_path_rejects_traversal_in_artifact` with coord `"org.evil:..:1.0"` failed.
- **Fix:** Added explicit `&& s != ".."` guard to `is_safe_maven_segment` before the charset check.
- **Files modified:** `src/loader/maven.rs`
- **Verification:** All 13 maven tests pass including the artifact traversal test.
- **Committed in:** `5fe0196` (Task 3 commit)

**2. [Rule 3 - Blocking] error.rs and maven.rs created in Task 1 commit**
- **Found during:** Task 1 (pre-build verification)
- **Issue:** `src/loader/mod.rs` declares `pub mod error; pub mod maven;` — Rust requires all declared modules to resolve as files before compilation. Creating only `types.rs` in Task 1 would cause a build failure.
- **Fix:** Created `error.rs` and `maven.rs` (and the stub files for `fabric.rs`, `quilt.rs`, `service.rs`) in the same Task 1 commit so the crate compiles immediately.
- **Files modified:** `src/loader/error.rs`, `src/loader/maven.rs`, `src/loader/fabric.rs`, `src/loader/quilt.rs`, `src/loader/service.rs`
- **Verification:** `cargo build` passes after Task 1 commit.
- **Committed in:** `9397047` (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (1 bug fix, 1 blocking pre-requisite)
**Impact on plan:** Both essential for correctness and compilability. No scope creep.

## Issues Encountered

None beyond the documented deviations.

## Known Stubs

| Stub | File | Reason |
|------|------|--------|
| `pub struct LoaderService;` | `src/loader/service.rs` | Populated by plan 06-05 |
| Empty module body | `src/loader/fabric.rs` | Populated by plan 06-03 |
| Empty module body | `src/loader/quilt.rs` | Populated by plan 06-04 |

## Next Phase Readiness

- Plan 06-02 (instance manifest extension) can import `LoaderInfo` from `crate::loader::types`
- Plans 06-03/06-04 (Fabric/Quilt clients) can import `LoaderLibrary`, `LoaderError`, and `maven_coord_to_path`/`maven_download_url`
- All 27 loader unit tests pass; `cargo build` and `cargo clippy --all-targets -- -D warnings` clean

---
*Phase: 06-fabric-and-quilt-modloaders*
*Completed: 2026-04-26*

## Self-Check: PASSED

Files exist:
- FOUND: src/loader/mod.rs
- FOUND: src/loader/types.rs
- FOUND: src/loader/error.rs
- FOUND: src/loader/maven.rs
- FOUND: src/loader/fabric.rs
- FOUND: src/loader/quilt.rs
- FOUND: src/loader/service.rs

Commits exist:
- FOUND: 9397047 (feat(06-01-01): add loader module scaffold and types)
- FOUND: 5fe0196 (feat(06-01-03): maven coord utilities with path-traversal guard)
