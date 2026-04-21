---
phase: "04"
plan: "03"
subsystem: auth
tags: [xbox, xsts, mc-services, reqwest, httpmock, pitfall-1, pitfall-5, pitfall-16]
one_liner: "Hand-rolled XBL + XSTS + MC services HTTP clients with d= RpsTicket prefix, XSTS 401 XErr parsing, and NoMinecraftLicense entitlement guard"
completed_at: "2026-04-21T03:45:00Z"
duration_minutes: 15
tasks_completed: 2
files_created: 0
files_modified: 2

dependency_graph:
  requires: ["04-01", "04-02"]
  provides:
    - src/auth/xbox.rs (authenticate_xbox_live, authenticate_xsts, XblTokens, XstsTokens)
    - src/auth/mc_services.rs (login_with_xbox, check_entitlement, fetch_profile, format_uuid, McLoginResponse, McProfile, EntitlementResponse)
  affects:
    - src/auth/chain.rs (plan 04-05 composes these functions)

tech_stack:
  added: []
  patterns:
    - "reqwest::Client passed by reference — single client shared across auth chain"
    - "base_url parameter for httpmock test injection (trim_end_matches('/') + path append)"
    - "serde_json::json! macro for POST bodies, .json(&body) for serialization"
    - "#[tracing::instrument(name = \"...\", skip_all)] on every pub fn (pitfall 16)"
    - "truncate_for_msg helper: caps error body at 200 chars, never logs raw tokens"
    - "body_includes matcher (httpmock 0.8.3 API — not body_contains)"

key_files:
  created: []
  modified:
    - src/auth/xbox.rs
    - src/auth/mc_services.rs

decisions:
  - "Used body_includes (not body_contains) — httpmock 0.8.3 uses body_includes for substring matching"
  - "XstsErrorBody.message field kept with #[allow(dead_code)] — always use map_xerr instead of raw MS message for consistent UX"
  - "check_entitlement checks both product_minecraft AND game_minecraft — Game Pass users may have product but not game entry"
  - "fetch_profile validates 32-char hex before returning — MalformedResponse early rather than passing bad id to format_uuid"
  - "Added test_login_non_200_returns_mc_login_error beyond plan's 8 tests — improves error coverage for free"

metrics:
  duration_minutes: 15
  completed_at: "2026-04-21"
  tasks: 2
  test_count_added: 16
  test_count_total: 74
---

# Phase 4 Plan 03: Xbox Live + XSTS + MC Services HTTP Clients Summary

Hand-rolled XBL authentication, XSTS authorization, and Minecraft services (login + entitlement + profile) HTTP clients. Enforces the three most error-prone protocol details: the `d=` RpsTicket prefix, XSTS 401 XErr parsing via `map_xerr`, and the `XBL3.0 x={uhs};{xsts_token}` identityToken format.

## What Was Built

### Task 04-03-01: xbox.rs

`src/auth/xbox.rs` implements two public async functions:

- `authenticate_xbox_live(client, base_url, ms_access_token)` — POST to `/user/authenticate` with `RpsTicket: "d={token}"` (pitfall 1 hard-enforced). Returns `XblTokens { token, user_hash }`. Non-2xx → `AuthError::XboxLive`.
- `authenticate_xsts(client, base_url, xbl_token)` — POST to `/xsts/authorize` with `SandboxId: "RETAIL"` and `RelyingParty: "rp://api.minecraftservices.com/"`. On 401, parses `XErr: u64` from JSON body and returns `AuthError::XstsDenied { xerr, message: map_xerr(xerr) }`. Non-2xx (other than 401) → `AuthError::Http`.

Both POSTs set `Content-Type: application/json`, `Accept: application/json`, and `x-xbl-contract-version: 1`.

6 httpmock tests: xbl success, d= prefix body assertion, xsts success with SandboxId/RelyingParty body assertions, xsts 401 with XErr 2148916233 mapped to "xbox profile" message, unknown XErr 9999999999 in fallback message, xbl 500 → `XboxLive` variant.

### Task 04-03-02: mc_services.rs

`src/auth/mc_services.rs` implements three public async functions and one pure helper:

- `login_with_xbox(client, base_url, user_hash, xsts_token)` — POST `"identityToken":"XBL3.0 x={user_hash};{xsts_token}"`. Returns `McLoginResponse { access_token, expires_in }`.
- `check_entitlement(client, base_url, mc_access_token)` — GET `/entitlements/mcstore` with Bearer auth. Returns `Ok(())` only when items contains both `product_minecraft` AND `game_minecraft`. Empty items or missing either entry → `AuthError::NoMinecraftLicense` (pitfall 5).
- `fetch_profile(client, base_url, mc_access_token)` — GET `/minecraft/profile` with Bearer auth. Validates returned `id` is 32-char hex before returning `McProfile { id, name }`.
- `format_uuid(hex32)` — Pure fn, inserts hyphens into 32-char hex to produce 8-4-4-4-12 UUID. Returns `AuthError::MalformedResponse` on invalid input.

10 tests: format_uuid ok/wrong-length/non-hex (pure), login success with identityToken body assertion, login 401 error, entitlement valid/empty/missing-one-item, profile success/malformed-id.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] httpmock 0.8.3 has `body_includes` not `body_contains`**
- **Found during:** Task 1 compilation
- **Issue:** Plan code used `body_contains` which does not exist in httpmock 0.8.3 API; the method is named `body_includes`
- **Fix:** Replaced all `body_contains(...)` calls with `body_includes(...)` in both files
- **Files modified:** src/auth/xbox.rs
- **Commit:** 76f4a27

**2. [Rule 2 - Missing functionality] Added login 401 error test**
- **Found during:** Task 2 implementation
- **Issue:** Plan specified 8 tests but omitted a non-200 error case for login_with_xbox
- **Fix:** Added `test_login_non_200_returns_mc_login_error` covering 401 → `AuthError::McLogin`
- **Files modified:** src/auth/mc_services.rs
- **Commit:** 28bc85a

## Protocol Constants Verified

| Constant | File | Value | Grep count |
|----------|------|-------|------------|
| RpsTicket d= prefix | xbox.rs | `format!("d={ms_access_token}")` | 1 |
| SandboxId RETAIL | xbox.rs | `XSTS_SANDBOX = "RETAIL"` | 4 |
| rp:// relying party | xbox.rs | `XSTS_MC_RELYING_PARTY = "rp://api.minecraftservices.com/"` | 3 |
| x-xbl-contract-version | xbox.rs | `"x-xbl-contract-version", "1"` (both POSTs + both tests) | 4 |
| identityToken XBL3.0 | mc_services.rs | `format!("XBL3.0 x={user_hash};{xsts_token}")` | 5 |
| NoMinecraftLicense | mc_services.rs | both empty-items and missing-item branches | 6 |

## Test Results

```
cargo test --lib auth::xbox   — 6 passed, 0 failed
cargo test --lib auth::mc_services — 10 passed, 0 failed
cargo test --lib — 74 passed, 0 failed
cargo clippy --all-targets -- -D warnings — clean
```

## Known Stubs

None — all functions are fully implemented with real HTTP logic.

## Threat Flags

None — no new network endpoints, auth paths, or schema changes beyond what the plan's threat model covers (T-04-03-01 through T-04-03-04).

## Self-Check: PASSED

- `src/auth/xbox.rs` exists and contains 356 lines
- `src/auth/mc_services.rs` exists and contains 366 lines
- Commit 76f4a27 exists (xbox.rs)
- Commit 28bc85a exists (mc_services.rs)
- 74 lib tests pass
- clippy clean
