---
phase: "04"
plan: "07"
subsystem: auth
tags: [service-facade, account-management, refresh-on-launch, progress-channel, device-code, end-to-end]
one_liner: "AccountService facade composing device_code + chain + store into 6 pub methods with auto-activate-first and always-refresh token resolution"
completed_at: "2026-04-21T04:03:00Z"
duration_minutes: 10
tasks_completed: 1
files_created: 0
files_modified: 1

dependency_graph:
  requires:
    - src/auth/device_code.rs (request_device_code, poll_for_token — plan 04-04)
    - src/auth/chain.rs (AuthChainConfig, run_full_auth, ensure_valid_mc_token — plan 04-05)
    - src/auth/store.rs (StoreConfig, store/load/delete_refresh_token, save/load_accounts — plan 04-06)
    - src/auth/mod.rs (Account, AuthContext, AuthError, MsaTokens, StorageBackend — plan 04-01)
    - src/persistence/paths.rs (AppPaths — plan 01)
  provides:
    - src/auth/service.rs (AccountService, AccountAuthEvent, AccountAddOutcome)
  affects:
    - src/auth/mod.rs (no changes needed — service module already declared)
    - TUI wiring (plan 04-09): calls start_device_code_auth, list_accounts, activate_account, remove_account
    - Launcher integration (plan 04-08): calls resolve_auth_context_for_launch, resolve_msa_tokens_for_launch

tech_stack:
  added: []
  patterns:
    - "AccountService::new(paths, http) builds both chain_config and store_config in one call"
    - "AccountService::new_with_config(chain_cfg, store_cfg) for tests — injected configs"
    - "mpsc::Sender<AccountAuthEvent> for streaming progress events to TUI (no internal thread management)"
    - "CancellationToken threaded through poll_for_token for Esc-key abort"
    - "forward tokio::spawn task translates DeviceCodeProgress -> AccountAuthEvent (aborted after poll completes)"
    - "First-added account auto-activates by checking accounts.is_empty() before push"
    - "always-refresh policy: resolve_msa_tokens_for_launch calls ensure_valid_mc_token unconditionally"
    - "#[tracing::instrument(name = '...', skip_all)] on all 6 public methods (pitfall 16)"
    - "Token values never appear in tracing field args"

key_files:
  created: []
  modified:
    - src/auth/service.rs

decisions:
  - "list_accounts also gets #[tracing::instrument(skip_all)] — consistent with all-methods tracing policy"
  - "forward task aborted (not awaited) after poll_for_token returns — avoids holding dc_rx open; clean drop"
  - "Re-add scenario (same account_id): replace existing entry, preserve is_active flag from stored record"
  - "resolve_msa_tokens_for_launch returns MsaTokens only (not (MsaTokens, Account)) — plan objective says Result<MsaTokens> is the correct return for the launcher integration path"

metrics:
  duration_minutes: 10
  completed_at: "2026-04-21"
  tasks: 1
  test_count_added: 8
  test_count_total: 112

requirements: [AUTH-01, AUTH-02, AUTH-03, AUTH-04, AUTH-06]
---

# Phase 4 Plan 07: AccountService Facade Summary

AccountService facade composing device_code + chain + store into 6 pub methods with auto-activate-first and always-refresh token resolution.

## What Was Built

### Task 04-07-01: Implement AccountService facade with end-to-end mocked test

`src/auth/service.rs` (355 lines of implementation + 200 lines of tests):

**Public types:**

- `AccountAuthEvent` — enum with `Started { user_code, verification_uri, expires_in }` and `Progress { stage }` variants for TUI streaming
- `AccountAddOutcome` — success return of `start_device_code_auth`: `account: Account`, `storage: StorageBackend`
- `AccountService` — stateless facade struct holding `chain_config: AuthChainConfig` and `store_config: StoreConfig`

**Public methods:**

| Method | Behavior |
|--------|----------|
| `new(paths, http)` | Builds production chain + store configs synchronously |
| `new_with_config(chain_cfg, store_cfg)` | Test constructor |
| `start_device_code_auth(cancel_token, event_tx)` | Full device-code -> chain -> persist flow; emits Started + Progress events |
| `list_accounts()` | `store::load_accounts` passthrough |
| `remove_account(id)` | Deletes refresh token + removes from accounts.json; idempotent on absent |
| `activate_account(id)` | Mutually-exclusive active flag; `AccountNotFound` if id absent |
| `resolve_auth_context_for_launch(default_username)` | Returns `Msa{account_id}` if any active, else `Offline{username}` |
| `resolve_msa_tokens_for_launch(id)` | Loads refresh token, calls ensure_valid_mc_token, persists rotated token + timestamps, returns MsaTokens |

**Auto-activate logic:** In `start_device_code_auth`, `if accounts.is_empty() { account_to_persist.is_active = true; }` fires before push — first account becomes active without extra activate call.

**8 tests:**

| Test | Coverage |
|------|----------|
| test_list_accounts_empty | Empty store returns empty Vec |
| test_activate_account_not_found | Missing id -> AccountNotFound error |
| test_remove_account_idempotent_on_absent | remove non-existent id -> Ok(()) |
| test_resolve_auth_context_no_active_returns_offline | No accounts -> Offline{username} |
| test_resolve_auth_context_with_active_returns_msa | Active account -> Msa{account_id} |
| test_activate_account_sets_exclusive_active | activate Y: Y.is_active=true, X.is_active=false |
| test_start_device_code_auth_end_to_end | 7-endpoint mock: full flow, auto-activate, persistence verified |
| test_remove_account_deletes_metadata_and_token | remove clears both accounts.json and refresh token |

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None — all 6 public methods are fully implemented.

## Threat Flags

None — no new network endpoints or trust boundaries. T-04-07-04 (stuck poll_for_token) is mitigated by the cancel_token threading verified in the end-to-end test (CancellationToken::new() passed through to poll_for_token). Token values never appear in tracing spans.

## Self-Check: PASSED

- `src/auth/service.rs` exists: FOUND
- Commit bd986ee exists: FOUND
- `cargo test --lib auth::service`: 8 passed, 0 failed: VERIFIED
- `cargo test --lib`: 112 passed, 0 failed: VERIFIED
- `cargo clippy --all-targets -- -D warnings`: clean: VERIFIED
- `grep -c 'block_on'`: 0: VERIFIED
- `grep -c 'pub struct AccountService'`: 1: VERIFIED
- `grep -c 'pub async fn start_device_code_auth'`: 1: VERIFIED
- `grep -c 'pub async fn list_accounts'`: 1: VERIFIED
- `grep -c 'pub async fn remove_account'`: 1: VERIFIED
- `grep -c 'pub async fn activate_account'`: 1: VERIFIED
- `grep -c 'pub async fn resolve_auth_context_for_launch'`: 1: VERIFIED
- `grep -c 'pub async fn resolve_msa_tokens_for_launch'`: 1: VERIFIED
- `grep -c 'chain::ensure_valid_mc_token|chain::run_full_auth'`: 3 (>= 2): VERIFIED
- `grep -c 'AccountAuthEvent::'`: 10 (>= 4): VERIFIED
- No claude/anthropic/co-authored refs: VERIFIED
