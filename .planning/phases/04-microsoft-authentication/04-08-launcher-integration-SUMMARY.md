---
phase: 04
plan: "08"
subsystem: launcher
tags: [launcher, auth-context, msa-branch, substitute-context]
dependency_graph:
  requires: [04-05, 04-07]
  provides: [real-msa-launch-command, offline-path-preserved]
  affects: [tui/run.rs, launcher/service.rs, launcher/command.rs, launcher/offline.rs]
tech_stack:
  added: []
  patterns: [auth-context-dispatch, msa-auth-adapter, compose_msa-parallel-function]
key_files:
  created: []
  modified:
    - src/launcher/service.rs
    - src/launcher/command.rs
    - src/launcher/offline.rs
    - src/launcher/mod.rs
    - src/tui/run.rs
    - tests/launch_integration.rs
decisions:
  - "MsaAuth adapter struct lives in offline.rs alongside OfflineAuth — all auth-field production in one place"
  - "compose_msa is a parallel function to compose (not an overloaded enum) — avoids conditional branching in the hot path"
  - "account_service: Option<&AccountService> injected as parameter — testable without real account store"
  - "MSA path with None service returns AppError::NoActiveAccount (not a panic)"
metrics:
  duration_seconds: 217
  completed_date: "2026-04-21"
  tasks_completed: 1
  files_modified: 6
---

# Phase 4 Plan 08: Launcher Integration Summary

**One-liner:** `launch_instance` now dispatches on `AuthContext` — offline path preserved byte-identical, MSA path resolves live MC tokens via `AccountService` and populates `SubstitutionContext` with real session fields.

## What Was Built

### `src/launcher/offline.rs` — MsaAuth adapter
Added `MsaAuth` struct and `from_tokens(&MsaTokens) -> MsaAuth` constructor. Sources `clientid` from `crate::auth::device_code::client_id()` (env-overridable). `user_type` always `"msa"`. Test `test_msa_auth_from_tokens` verifies all 7 fields.

### `src/launcher/command.rs` — compose_msa
Added `compose_msa(version, auth: &MsaAuth, paths, slug, ctx, java_bin)` parallel to the existing `compose`. Populates `SubstitutionContext` with live MC session fields: `auth_player_name`, `auth_uuid`, `auth_access_token`, `auth_xuid`, `clientid`, `auth_xbox_user_hash`, `user_type = "msa"`. Six new unit tests assert each field propagates correctly.

### `src/launcher/service.rs` — AuthContext dispatch
Changed `launch_instance` signature:
- Old: `(paths, slug, username: &str, tx, token, job_id)`
- New: `(paths, slug, auth_ctx: AuthContext, account_service: Option<&AccountService>, tx, token, job_id)`

Step 5 now matches on `auth_ctx`:
- `Offline { username }` → `offline_auth(&username)` → `compose(...)` (zero behavior change)
- `Msa { account_id }` → `account_service.ok_or(NoActiveAccount)?` → `resolve_msa_tokens_for_launch` → `MsaAuth::from_tokens` → `compose_msa(...)`

Added `#[tracing::instrument(skip_all)]`. No raw tokens in logs.

### `src/launcher/mod.rs` — re-export
Added `pub use crate::auth::AuthContext`.

### `src/tui/run.rs` — call site updated
`Effect::LaunchInstance` now wraps `username` in `AuthContext::Offline { username }` and passes `None` for `account_service`. Behavior is identical to Phase 3.

### `tests/launch_integration.rs` — call site updated
Integration test updated to use `AuthContext::Offline { username: "TestUser".to_string() }` and `None` for `account_service`.

## Verification

- `cargo build`: clean
- `cargo clippy --all-targets -- -D warnings`: clean
- `cargo test`: 120 lib tests + 7 integration test suites — all pass
- `tests/launch_command.rs` (offline snapshot): 4 tests pass, offline path unchanged
- New MSA tests: `test_msa_auth_from_tokens`, `test_compose_msa_access_token_in_game_args`, `test_compose_msa_user_type_is_msa`, `test_compose_msa_player_name`, `test_compose_msa_uuid`, `test_compose_msa_xuid_present`, `test_compose_msa_no_unsubstituted_tokens`, `test_compose_msa_java_bin_preserved`

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None. The offline path is fully wired (unchanged from Phase 3). The MSA path is fully wired to `AccountService::resolve_msa_tokens_for_launch`. No placeholder values in any returned `LaunchCommand`.

## Threat Flags

| Flag | File | Description |
|------|------|-------------|
| T-04-08-01 (accepted) | src/launcher/service.rs | MC access token reaches process argv via compose_msa — unavoidable per Minecraft protocol |

No new unplanned threat surface introduced.

## Self-Check: PASSED

- `src/launcher/service.rs` — exists, contains `AuthContext::Offline` and `AuthContext::Msa` branches
- `src/launcher/command.rs` — exists, contains `compose_msa` function definition + 6 tests
- `src/launcher/offline.rs` — exists, contains `pub struct MsaAuth` + `from_tokens`
- `src/launcher/mod.rs` — exists, contains `pub use crate::auth::AuthContext`
- Commit `430268b` — verified in git log
