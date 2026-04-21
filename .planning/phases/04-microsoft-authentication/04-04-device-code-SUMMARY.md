---
phase: "04"
plan: "04"
subsystem: auth
tags: [device-code, oauth2, msa, polling, rfc8628, refresh, httpmock]
one_liner: "RFC 8628 MSA device-code polling loop with slow_down/expired/denied state transitions, cancellation via CancellationToken, and refresh_access_token for AUTH-03"
completed_at: "2026-04-21T03:48:00Z"
duration_minutes: 15
tasks_completed: 1
files_created: 0
files_modified: 2

dependency_graph:
  requires: ["04-01"]
  provides:
    - src/auth/device_code.rs (request_device_code, poll_for_token, refresh_access_token, DeviceCodeStart, DeviceCodeProgress, TokenResponse, client_id, DEFAULT_MSA_CLIENT_ID, MSA_SCOPE)
  affects:
    - src/auth/chain.rs (plan 04-05 calls request_device_code + poll_for_token + refresh_access_token)

tech_stack:
  added:
    - reqwest form feature (added to Cargo.toml — required for .form() on RequestBuilder in reqwest 0.13 with default-features = false)
  patterns:
    - "reqwest::Client passed by reference — single client shared across auth chain"
    - "base_url parameter for httpmock injection (trim_end_matches('/') + path append)"
    - ".form(&[(&str, String)]) for application/x-www-form-urlencoded POST bodies"
    - "#[tracing::instrument(name = \"...\", skip_all)] on every pub async fn (pitfall 16)"
    - "tokio::select! biased; with cancel_token.cancelled() for clean cancellation"
    - "interval_secs.saturating_add(5) for slow_down per RFC 8628 §3.5"
    - "mpsc::Sender<DeviceCodeProgress> for progress events to the TUI"
    - "Merged env-var tests into a single sequential test to avoid parallel interference"

key_files:
  created: []
  modified:
    - src/auth/device_code.rs
    - Cargo.toml

decisions:
  - "Hand-rolled reqwest POSTs instead of using oauth2 5.0 crate — preserves base_url override for httpmock and full control over polling interval"
  - "Merged test_client_id_default and test_client_id_env_override into single test to prevent parallel env-var interference"
  - "Used body_includes (not body_contains) — httpmock 0.8.3 API (same as xbox.rs / mc_services.rs convention)"
  - "Added reqwest form feature — required in reqwest 0.13 with default-features=false; the form() method is feature-gated"
  - "#[tracing::instrument(name = '...', skip_all)] with explicit name= for span disambiguation in traces"

metrics:
  duration_minutes: 15
  completed_at: "2026-04-21"
  tasks: 1
  test_count_added: 13
  test_count_total: 87

requirements: [AUTH-01]
---

# Phase 4 Plan 04: MSA Device-Code Flow Summary

RFC 8628 MSA device-code polling loop with slow_down/expired/denied state transitions, cancellation via CancellationToken, and refresh_access_token for AUTH-03.

## What Was Built

### Task 04-04-01: device_code.rs — full state machine + tests

`src/auth/device_code.rs` implements the complete MSA OAuth 2.0 device-code flow:

**Constants:**
- `DEFAULT_MSA_CLIENT_ID = "00000000402b5328"` — legacy Mojang public launcher ID
- `MSA_CLIENT_ID_ENV = "MINELTUI_MSA_CLIENT_ID"` — env override (A1 from RESEARCH.md)
- `MSA_SCOPE = "XboxLive.signin offline_access"` — both scopes required (pitfall 5: offline_access for refresh_token)

**Public functions:**

- `client_id()` — reads env override, falls back to DEFAULT_MSA_CLIENT_ID
- `request_device_code(client, base_url)` — POST form to `/consumers/oauth2/v2.0/devicecode`, returns `DeviceCodeStart { user_code, verification_uri, device_code, interval, expires_in }`
- `poll_for_token(client, base_url, device_code, initial_interval, cancel_token, event_tx)` — RFC 8628 polling loop:
  - `tokio::select! biased;` on `cancel_token.cancelled()` → `Err(UserCancelled)`
  - 200 OK → emit `DeviceCodeProgress::Complete`, return `TokenResponse`
  - `authorization_pending` → emit `AuthorizationPending`, continue
  - `slow_down` → `interval_secs.saturating_add(5)`, emit `SlowDown`, continue
  - `expired_token` → `Err(DeviceCodeExpired)`
  - `access_denied` → `Err(DeviceCodeFailed("user denied access"))`
  - other → `Err(DeviceCodeFailed(format!("{other}: {description}")))`
- `refresh_access_token(client, base_url, refresh_token)` — POST `grant_type=refresh_token` to token endpoint; `invalid_grant` → `Err(RefreshFailed)`

**13 tests:**

| Test | Coverage |
|------|----------|
| test_client_id_default_and_env_override | default ID + env override in one sequential test |
| test_msa_scope_contains_xbox_live_signin | scope constant validation |
| test_msa_scope_contains_offline_access | pitfall 5 enforcement |
| test_request_device_code_success | 200 → DeviceCodeStart fields |
| test_request_device_code_http_error | 400 → DeviceCodeRequest error |
| test_poll_success_on_first_poll | 200 → TokenResponse + Complete event |
| test_poll_authorization_pending_then_cancelled | pending + cancel → UserCancelled + AuthorizationPending event |
| test_poll_slow_down_bumps_interval | slow_down → SlowDown event with new_interval=6 (1+5) |
| test_poll_expired_token | expired_token → DeviceCodeExpired |
| test_poll_access_denied | access_denied → DeviceCodeFailed("user denied access") |
| test_poll_cancellation_immediate | cancel fires before poll → UserCancelled |
| test_refresh_success | refresh_token grant → new TokenResponse |
| test_refresh_invalid_grant | invalid_grant → RefreshFailed |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] reqwest 0.13 `form` feature missing**
- **Found during:** Task 04-04-01 compilation
- **Issue:** `reqwest` was configured with `default-features = false, features = ["rustls", "stream", "json", "gzip"]`. In reqwest 0.13, the `.form()` method on `RequestBuilder` requires the `form` feature to be enabled. Compilation failed with "no method named `form` found for struct `RequestBuilder`".
- **Fix:** Added `"form"` to the reqwest features list in Cargo.toml.
- **Files modified:** Cargo.toml
- **Commit:** 03aebe4

**2. [Rule 1 - Bug] Parallel env-var test interference**
- **Found during:** Full test suite run (`cargo test --lib`)
- **Issue:** Two separate tests (`test_client_id_default` and `test_client_id_env_override`) both manipulated `MINELTUI_MSA_CLIENT_ID`. When run in parallel (cargo's default), the env var set in the override test leaked into the default test, causing it to see "override-id-xyz" instead of the default.
- **Fix:** Merged both assertions into a single sequential test `test_client_id_default_and_env_override` that sets, asserts, then removes the env var — no parallel interference possible.
- **Files modified:** src/auth/device_code.rs
- **Commit:** 03aebe4

## Protocol Constants Verified

| Constant | Value | Grep count |
|----------|-------|------------|
| DEFAULT_MSA_CLIENT_ID | "00000000402b5328" | 4 |
| MSA_CLIENT_ID_ENV | "MINELTUI_MSA_CLIENT_ID" | 3 |
| MSA_SCOPE | "XboxLive.signin offline_access" | 3 |
| saturating_add(5) | slow_down branch | 1 |
| tracing::instrument | 3 pub fns (name=..., skip_all) | 3 |
| block_on | absent | 0 |
| raw token in tracing macros | absent | 0 |

## Test Results

```
cargo test --lib auth::device_code   — 13 passed, 0 failed
cargo test --lib                     — 87 passed, 0 failed
cargo clippy --all-targets -- -D warnings — clean
```

## Known Stubs

None — all three public functions are fully implemented.

## Threat Flags

None — no new network endpoints beyond what the plan's threat model covers (T-04-04-01 through T-04-04-04). The `form` feature addition to reqwest does not introduce new trust boundaries.

## Self-Check: PASSED

- `src/auth/device_code.rs` exists: FOUND
- Commit 03aebe4 exists: FOUND
- 87 lib tests pass: VERIFIED
- clippy clean: VERIFIED
- grep DEFAULT_MSA_CLIENT_ID ≥ 1: 4
- grep MINELTUI_MSA_CLIENT_ID ≥ 1: 3
- grep 'XboxLive.signin offline_access' ≥ 1: 3
- grep saturating_add(5) ≥ 1: 1
- grep block_on == 0: 0
- grep raw tokens in tracing == 0: 0
