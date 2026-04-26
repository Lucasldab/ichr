---
phase: 06-fabric-and-quilt-modloaders
plan: "03"
subsystem: loader
tags: [fabric, http, reqwest, httpmock, modloader, meta-api]

requires:
  - phase: 06-01
    provides: LoaderLibrary, LoaderVersionEntry, LoaderError canonical types in src/loader/types.rs and src/loader/error.rs

provides:
  - FabricMetaClient with dual constructors (env-var + test injection)
  - list_loader_versions() returning Vec<LoaderVersionEntry>
  - fetch_profile() returning FabricProfile with id, raw_bytes, libraries
  - 6 httpmock-driven unit tests covering all error paths

affects:
  - 06-04-quilt-client (same pattern, quilt.rs uses canonical types from same location)
  - 06-05-loader-service (calls list_loader_versions + fetch_profile on FabricMetaClient)

tech-stack:
  added: []
  patterns:
    - "FabricMetaClient dual-constructor: new() reads env var, new_with_base_url() injects test base URL without touching env"
    - "Error mapping: reqwest .error_for_status() -> LoaderError::MetaFetch; serde_json parse failures -> LoaderError::MetaParse"
    - "raw_bytes preservation: fetch_profile returns verbatim API bytes alongside parsed fields for disk write in later plan"
    - "Env-var save/restore in test_env_override_base_url to avoid parallel-test pollution"

key-files:
  created: []
  modified:
    - src/loader/fabric.rs

key-decisions:
  - "FabricProfile returns raw_bytes (Vec<u8>) alongside parsed fields so 06-05 can atomic_write verbatim response without re-serializing"
  - "FabricLoaderItem.build defaults to 0 via #[serde(default)] — consistent with Fabric API where build field is always present but protects against future shape changes"
  - "LoaderLibrary imported from crate::loader::types (not redeclared) to maintain single source of truth shared with quilt.rs and service.rs"

patterns-established:
  - "Pattern: Fabric meta client mirrors AdoptiumClient pattern exactly — dual constructors, reqwest builder with USER_AGENT + gzip + 30s timeout, bytes() then serde_json::from_slice"
  - "Pattern: httpmock tests use MockServer::start() + server.mock(|when, then|) + make_client() helper, never touch env vars"

requirements-completed: [LOAD-01]

duration: 2min
completed: 2026-04-26
---

# Phase 06 Plan 03: Fabric Meta Client Summary

**FabricMetaClient HTTP wrapper over meta.fabricmc.net/v2/ with httpmock unit tests — list loader versions and fetch profile JSON with canonical LoaderLibrary import**

## Performance

- **Duration:** 2 min
- **Started:** 2026-04-26T12:05:40Z
- **Completed:** 2026-04-26T12:07:48Z
- **Tasks:** 1
- **Files modified:** 1

## Accomplishments

- Replaced the one-line stub `src/loader/fabric.rs` with a complete 323-line implementation
- `FabricMetaClient` constructed via `new()` (reads `MINELTUI_FABRIC_META_BASE_URL`) or `new_with_base_url()` for test injection
- `list_loader_versions()` GETs `/v2/versions/loader`, maps `FabricLoaderItem` wire type to canonical `LoaderVersionEntry`
- `fetch_profile()` GETs `/v2/versions/loader/{game}/{loader}/profile/json`, returns `FabricProfile { id, raw_bytes, libraries }` with verbatim bytes preserved for disk write
- All 6 httpmock tests pass: list happy path, 5xx→MetaFetch, bad JSON→MetaParse, profile happy path with sha1 preservation, 404→MetaFetch, env override

## Task Commits

1. **Task 1: FabricMetaClient + list_loader_versions + httpmock tests** - `4791f58` (feat)

**Plan metadata:** (docs commit follows)

## Files Created/Modified

- `src/loader/fabric.rs` — FabricMetaClient with list_loader_versions, fetch_profile, FabricProfile struct, and 6 httpmock tests

## Decisions Made

- Returned `raw_bytes: Vec<u8>` in `FabricProfile` alongside parsed fields — 06-05 will `atomic_write` the verbatim API bytes to disk without re-serializing, preserving unknown fields (e.g., `inheritsFrom`, `arguments`, `mainClass`)
- Used `#[serde(default)]` on `FabricLoaderItem.build: u32` — Fabric always includes this field but the default protects against unexpected future response shape changes
- Did not use `serde_json::Value` for profile parsing — typed `FabricProfileJson` struct gives clearer compile-time guarantees; raw bytes still preserved for disk write

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- `FabricMetaClient` is ready for 06-04 to mirror as `QuiltMetaClient` (identical pattern, different base URL and endpoint paths)
- `fetch_profile` returns `FabricProfile` with all fields 06-05's `LoaderService::install_loader` needs: `id` for version directory naming, `raw_bytes` for disk write, `libraries` for download loop
- No blockers

---
*Phase: 06-fabric-and-quilt-modloaders*
*Completed: 2026-04-26*

## Self-Check: PASSED

- `src/loader/fabric.rs` exists and compiles: FOUND
- Commit `4791f58` exists: FOUND
- 6 tests pass (verified by cargo test run above): FOUND
