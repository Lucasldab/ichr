---
phase: "04"
plan: "05"
subsystem: auth
tags: [chain, orchestrator, refresh, end-to-end, httpmock, auth-02, auth-03]
one_liner: "Auth chain orchestrator composing XBL -> XSTS -> MC login -> entitlement -> profile into run_full_auth, with always-refresh ensure_valid_mc_token via MSA refresh_token rotation"
completed_at: "2026-04-20T18:12:40Z"
duration_minutes: 12
tasks_completed: 1
files_created: 0
files_modified: 1

dependency_graph:
  requires: ["04-01", "04-03", "04-04"]
  provides:
    - src/auth/chain.rs (run_full_auth, ensure_valid_mc_token, AuthChainConfig, AuthChainOutput)
  affects:
    - src/auth/service.rs (plan 04-07 calls run_full_auth + ensure_valid_mc_token)
    - src/launcher/service.rs (plan 04-08 uses MsaTokens from ensure_valid_mc_token)

tech_stack:
  added: []
  patterns:
    - "AuthChainConfig with production() and single_host() constructors — single_host() used by httpmock tests"
    - "All base URLs as parameters on AuthChainConfig — consistent with xbox.rs / mc_services.rs / device_code.rs pattern"
    - "#[tracing::instrument(name = '...', skip_all)] on both pub async fns (pitfall 16)"
    - "ensure_valid_mc_token ALWAYS refreshes — no conditional expiry check (design decision, see below)"
    - "run_full_auth uses Account::to_unix(SystemTime::now()) for timestamps — consistent with domain type"

key_files:
  created: []
  modified:
    - src/auth/chain.rs

decisions:
  - "ensure_valid_mc_token always runs the full refresh path regardless of token age — MC tokens expire 24h after issue; we cannot reliably skip refresh since the launcher may be used infrequently"
  - "AuthChainOutput includes refresh_token and expiry timestamps as top-level fields so store.rs (plan 04-06) can persist without unpacking Account fields"
  - "Account.storage defaults to StorageBackend::EncryptedFile in chain.rs — store.rs will overwrite this when it knows which backend actually persisted the token"
  - "Used body_includes (not body_contains) in httpmock mocks — httpmock 0.8.3 API convention established in plans 04-03 and 04-04"

requirements: [AUTH-02, AUTH-03]

metrics:
  duration_minutes: 12
  completed_at: "2026-04-20"
  tasks: 1
  test_count_added: 5
  test_count_total: 92
---

# Phase 4 Plan 05: Chain Orchestrator Summary

Auth chain orchestrator composing XBL -> XSTS -> MC login -> entitlement -> profile into `run_full_auth`, with always-refresh `ensure_valid_mc_token` via MSA refresh_token rotation.

## What Was Built

### Task 04-05-01: chain.rs — full implementation + httpmock tests

`src/auth/chain.rs` (419 lines) implements:

**`AuthChainConfig`**

Config struct with `http: reqwest::Client` and four base URLs (`msa_base_url`, `xbl_base_url`, `xsts_base_url`, `mc_base_url`). Two constructors:
- `production(http)` — uses real MS/Xbox/MC endpoint bases
- `single_host(http, base)` — all four URLs point at the same MockServer host (path routing disambiguates)

**`AuthChainOutput`**

Return type of both entry points: `account: Account`, `tokens: MsaTokens`, `refresh_token: String`, `mc_token_expires_at: i64`, `msa_token_expires_at: i64`.

**`run_full_auth(config, msa_access_token, msa_refresh_token, msa_expires_in_sec)`**

Sequences the five HTTP calls in order:
1. `xbox::authenticate_xbox_live` — XBL token + user_hash
2. `xbox::authenticate_xsts` — XSTS token (401 XErr already mapped inside xbox.rs)
3. `mc_services::login_with_xbox` — MC access_token + expires_in
4. `mc_services::check_entitlement` — ensures product_minecraft + game_minecraft present
5. `mc_services::fetch_profile` + `mc_services::format_uuid` — MC UUID (hyphenated) + player name

Builds `Account` with `id = profile.id` (raw 32-char hex), `mc_uuid` (hyphenated), unix timestamps for `added_at` / `last_refreshed_at` / expiry fields. `MsaTokens.user_type = "msa"`, `xuid = user_hash = xsts.user_hash`.

**`ensure_valid_mc_token(config, refresh_token)`**

Always runs the refresh path: `device_code::refresh_access_token` -> get new MSA `access_token` + rotated `refresh_token` -> `run_full_auth`. On `AuthError::RefreshFailed` (invalid_grant), propagates to caller to trigger re-auth.

**5 httpmock tests:**

| Test | Coverage |
|------|----------|
| `test_run_full_auth_happy_path` | All 5 endpoints mocked; asserts mc_username, mc_uuid (hyphenated), access_token, user_type="msa", xuid, refresh_token, timestamps within 30s |
| `test_run_full_auth_no_entitlement` | Empty items[] -> NoMinecraftLicense propagated |
| `test_run_full_auth_xsts_denied_propagates_xerr` | XSTS 401 XErr=2148916233 -> XstsDenied with xerr + message containing "xbox profile" |
| `test_ensure_valid_mc_token_refresh_path` | refresh_token=old-ref -> new-ref via token endpoint; re-chain produces PlayerOne account |
| `test_ensure_valid_mc_token_refresh_revoked` | invalid_grant 400 -> RefreshFailed |

## Deviations from Plan

None — plan executed exactly as written. The only pre-emptive adjustment was using `body_includes` instead of `body_contains` in httpmock mocks (established convention from plans 04-03 and 04-04; the plan code used `body_contains` which is the wrong method name for httpmock 0.8.3, but this was caught by the convention awareness from prior summaries and corrected before compilation).

## Security Controls Verified (Threat Model T-04-05-01 through T-04-05-03)

| Threat | Control | Verification |
|--------|---------|-------------|
| T-04-05-01: Token disclosure in logs | `#[tracing::instrument(skip_all)]` on both pub fns; `tracing::info!` uses only account_id, mc_username, mc_expires_at | `grep -Ec 'tracing::(info|debug|warn|error)!.*(access_token|refresh_token)' == 0` |
| T-04-05-02: Account ID provenance | `account.id = profile.id` from Minecraft services (server-authoritative) | Verified in test assertions |
| T-04-05-03: Stale MC token reuse | `ensure_valid_mc_token` always refreshes | Design enforced by function structure |

## Test Results

```
cargo test --lib auth::chain   — 5 passed, 0 failed
cargo test --lib               — 92 passed, 0 failed
cargo clippy --all-targets -- -D warnings — clean
grep -c 'skip_all' src/auth/chain.rs — 3 (>= 2 required)
grep -c 'block_on' src/auth/chain.rs — 0
grep -c '"msa"' src/auth/chain.rs — 1
```

## Known Stubs

None — both public functions are fully implemented.

## Threat Flags

None — no new network endpoints or trust boundaries beyond what the plan's threat model covers.

## Self-Check: PASSED

- `src/auth/chain.rs` exists with 419 lines: FOUND
- Commit 8d74a9c exists: FOUND
- 92 lib tests pass (87 prior + 5 new): VERIFIED
- clippy clean: VERIFIED
- `grep -c 'skip_all' == 3`: VERIFIED
- `grep -c 'block_on' == 0`: VERIFIED
- No claude/anthropic refs: VERIFIED
