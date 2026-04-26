---
phase: "06"
plan: "05"
subsystem: loader
tags: [loader, service, install, semaphore, atomic-write, cancellation, sha1, reattach]
dependency_graph:
  requires:
    - 06-01  # LoaderLibrary, LoaderType, LoaderInfo, LoaderVersionEntry, LoaderError, maven helpers
    - 06-02  # InstanceManifest.loader field
    - 06-03  # FabricMetaClient, FabricProfile
    - 06-04  # QuiltMetaClient, QuiltProfile, is_quilt_stable
  provides:
    - LoaderService facade (list_loader_versions, install_loader, remove_loader)
  affects:
    - 06-06  # loader_version_picker_modal will call list_loader_versions
    - 06-07  # loader install modal will call install_loader
    - 06-08  # TUI run.rs wire-up of LoaderService
tech_stack:
  added:
    - tokio::task::JoinSet for parallel library downloads
    - tokio::sync::Semaphore (LIB_CONCURRENCY=8 cap)
    - tokio_util::sync::CancellationToken (propagated per-library-download-task)
  patterns:
    - Struct-of-args (InstallArgs) to avoid too_many_arguments clippy
    - predict_version_id + try_reattach_from_disk for zero-HTTP re-attach
    - Atomic-last write: instance.json written LAST for partial-install safety
key_files:
  created:
    - src/loader/service.rs  # LoaderService full implementation (1,300+ lines)
  modified:
    - src/loader/fabric.rs   # add pub(crate) fn http() accessor
    - src/loader/quilt.rs    # add pub(crate) fn http() accessor
decisions:
  - "predict_version_id used for zero-HTTP re-attach: fabric-loader-{v}-{mc} / quilt-loader-{v}-{mc} prefix pattern lets us check disk before any network call; if prediction misses (edge case) we fall through to full install"
  - "InstallArgs struct introduced to work around clippy::too_many_arguments (9 args > 7 limit); public install_loader method keeps #[allow] to preserve the caller-visible signature for 06-08"
  - "Cancellation pre-check at top of install_loader_impl so a pre-fired token returns immediately without touching the instance manifest"
  - "SHA1 verification (T-06-11) happens on downloaded bytes BEFORE atomic_write; existing files skip re-download only if verify_sha1 returns true"
metrics:
  duration: "417s"
  completed_at: "2026-04-26T08:01:31Z"
  tasks_completed: 2
  files_changed: 3
---

# Phase 06 Plan 05: LoaderService Facade Summary

LoaderService implementing the four-step install pipeline (fetch profile, semaphore-bounded library download, atomic version JSON write, atomic instance manifest write) with zero-HTTP re-attach optimization, SHA1 verification for Fabric, cancellation safety, and remove+reinstall switch semantics.

## Tasks Completed

| Task | Description | Commit | Key Files |
|------|-------------|--------|-----------|
| 1 | LoaderService struct, list_loader_versions, remove_loader + 4 tests | 357c255 | src/loader/service.rs, fabric.rs, quilt.rs |
| 2 | install_loader 4-step pipeline + re-attach + cancellation + 7 tests | 357c255 | src/loader/service.rs |

Note: Both tasks landed in a single commit since Task 2's full implementation was written alongside Task 1.

## What Was Built

`src/loader/service.rs` (1,300+ lines) implementing:

- `LoaderService::new()` — production constructor with env-var-overridable base URLs
- `LoaderService::with_clients(fabric, quilt)` — test injection constructor
- `list_loader_versions(loader_type, _mc_version)` — dispatches on LoaderType to FabricMetaClient or QuiltMetaClient
- `remove_loader(paths, slug)` — reads manifest, deletes `versions/{loader_version_id}/` dir, sets `manifest.loader = None`
- `install_loader(paths, slug, mc_version, loader_type, loader_version, progress_tx, token, job_id)` — full four-step pipeline

The install pipeline:
1. Zero-HTTP re-attach pre-check: predict version_id from known format, read on-disk version JSON, verify all library paths exist → if all present, skip to Step 4 with no network traffic
2. Step 1 (1%): fetch loader profile from meta API (FabricMetaClient / QuiltMetaClient)
3. Step 2 (2-90%): parallel library downloads via `JoinSet` + `Arc<Semaphore::new(8)>`; per-library cancellation check; Fabric libraries SHA1-verified before write; Quilt libraries accepted on existence
4. Step 3 (95%): `atomic_write(versions/{id}/{id}.json, raw_bytes)`
5. Step 4 (100%): `manifest.loader = Some(LoaderInfo { ... })` + `write_instance_manifest` — written LAST (atomicity invariant per Pitfall 7)

## Test Results

11/11 tests pass:

| Test | What It Validates |
|------|------------------|
| test_list_loader_versions_dispatches_fabric | LoaderType::Fabric → FabricMetaClient |
| test_list_loader_versions_dispatches_quilt | LoaderType::Quilt → QuiltMetaClient |
| test_remove_loader_clears_manifest_and_removes_version_dir | Remove writes manifest.loader=None + deletes dir |
| test_remove_loader_noop_when_no_loader | remove_loader on vanilla instance is a no-op |
| test_install_fabric_full_flow | Full Fabric install: version JSON written, library on disk, manifest set |
| test_install_quilt_full_flow | Full Quilt install: no hash verification, all artifacts written |
| test_install_skips_when_already_attached | Re-attach: second install with artifacts on disk makes 0 profile fetches |
| test_install_sha1_mismatch_returns_sha1mismatch | Wrong bytes → LoaderError::Sha1Mismatch (T-06-11) |
| test_install_cancelled_before_completion_returns_cancelled | Pre-fired token → Err(LoaderError::Cancelled) |
| test_install_does_not_write_instance_manifest_on_cancel | instance.json loader field stays None on cancel |
| test_switch_loader_via_remove_then_install | remove Fabric + install Quilt leaves clean Quilt state |

## Decisions Made

1. **Zero-HTTP re-attach** uses `predict_version_id` (derived from known `fabric-loader-{v}-{mc}` / `quilt-loader-{v}-{mc}` format) + `try_reattach_from_disk` (reads on-disk JSON, parses libraries, verifies all lib paths). If prediction is wrong (edge case), falls through to full install. This satisfies "no HTTP fetches happen on re-attach" without the circular dependency of needing the profile to know the libraries to check re-attach.

2. **`InstallArgs` struct** wraps the 9 parameters of `install_loader_impl` to satisfy `clippy::too_many_arguments`. The public `install_loader` method keeps `#[allow(clippy::too_many_arguments)]` since its signature is a stable API contract for 06-08.

3. **SHA1 verification order** (T-06-11): verify on bytes before `atomic_write`. Existing files skip re-download only if `verify_sha1` returns true. Quilt files have no hash → existence is sufficient.

4. **Cancellation pre-check** at the very top of `install_loader_impl` so a pre-fired token returns `LoaderError::Cancelled` immediately without reading instance manifest.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Re-attach test design: first install needed library mock**

- **Found during:** Task 2 test execution
- **Issue:** Test `test_install_skips_when_already_attached` only registered the profile mock, not the library mock. First install failed with 404 on library download, preventing artifacts from landing on disk for re-attach detection.
- **Fix:** Added library mock registration for first install pass. This matches the test's actual intent: populate artifacts via first install, then assert second install hits profile mock 0 additional times.
- **Files modified:** src/loader/service.rs (test body)
- **Commit:** 357c255

**2. [Rule 2 - Missing functionality] predict_version_id + try_reattach_from_disk**

- **Found during:** Task 2 implementation
- **Issue:** Plan specified re-attach check happens after `fetch_profile` (needs profile.id and libraries), but context_notes required "no HTTP fetches on re-attach." These two constraints are contradictory if re-attach check follows profile fetch. The plan's `test_install_skips_when_already_attached` assertion `profile_mock.assert_calls(1)` confirms the second install must not call the meta API at all.
- **Fix:** Added `predict_version_id` (format derived from observed Fabric/Quilt API behavior) and `try_reattach_from_disk` (reads on-disk version JSON, parses library list, checks all paths). If pre-check passes, returns early before any HTTP calls. If prediction fails, falls through to full install.
- **Files modified:** src/loader/service.rs
- **Commit:** 357c255

## Threat Surface Scan

| Mitigation | File | Status |
|------------|------|--------|
| T-06-11: Fabric SHA1 before write | service.rs::download_one_library | Implemented |
| T-06-12: Quilt TLS-only (accepted) | service.rs::download_one_library | Documented |
| T-06-13: instance.json written LAST | service.rs::install_loader_impl Step 4 | Implemented |
| T-06-14: maven_coord_to_path validation | maven.rs (06-01) | Pre-existing |
| T-06-15: semaphore + abort_all on error | service.rs::install_loader_impl Step 2 | Implemented |

No new threat surface introduced beyond the plan's threat model.

## Known Stubs

None. All public methods are fully implemented. No placeholder data flows to any UI rendering path.

## Self-Check: PASSED
