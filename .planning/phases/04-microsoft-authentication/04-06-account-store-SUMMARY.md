---
phase: "04"
plan: "06"
subsystem: auth
tags: [storage, keyring, aes-gcm, encryption, fallback, machine-id, base64]
completed_at: "2026-04-21T03:58:16Z"
duration_minutes: 25
tasks_completed: 1
files_created: 0
files_modified: 3

dependency_graph:
  requires:
    - src/auth/mod.rs (AuthError, Account, StorageBackend — from 04-01)
    - src/domain/account.rs (Account struct — from 04-01)
    - src/persistence/paths.rs (accounts_file, accounts_json_file — from 04-01)
  provides:
    - src/auth/store.rs (store_refresh_token, load_refresh_token, delete_refresh_token,
        save_accounts, load_accounts, StoreConfig, derive_machine_key)
  affects: []

tech_stack:
  added:
    - base64 = "0.22" (STANDARD engine for nonce||ciphertext blob encoding)
  patterns:
    - StoreConfig dependency injection for testability (force_fallback flag)
    - AES-256-GCM with per-entry random 12-byte nonce prepended to ciphertext
    - SHA-256(domain_separator || machine_id_bytes) for 32-byte key derivation
    - tokio::task::spawn_blocking wrapping all keyring sync calls (pitfall 21)
    - Atomic write via .tmp -> rename for both accounts.enc and accounts.json
    - 0o600 file permissions on accounts.enc via std::os::unix::fs::PermissionsExt

key_files:
  created: []
  modified:
    - src/auth/store.rs (stub replaced with full implementation, 340 lines)
    - Cargo.toml (base64 = "0.22" added)
    - Cargo.lock (updated for base64 dep)

decisions:
  - "Encrypted file is a JSON map {account_id -> base64(nonce||ciphertext)} for per-entry granularity — one corrupt/added entry does not require full re-encryption"
  - "force_fallback: true in StoreConfig skips keyring entirely — enables deterministic tests in CI where libsecret daemon is absent"
  - "Key derivation uses SHA-256 with domain separator 'mineltui-auth-v1:' prepended to machine_id bytes"
  - "tracing::instrument uses name + skip_all + fields pattern — account_id in span fields, token value never logged"

requirements: [AUTH-04, AUTH-06]
---

# Phase 04 Plan 06: Account Store Summary

**One-liner:** Keyring-primary + AES-256-GCM encrypted-file fallback for MSA refresh tokens, with machine-ID-derived key, per-entry nonce, atomic writes, 0600 perms, and plain-JSON metadata tier.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 04-06-01 | Implement keyring + encrypted-file fallback + account metadata persistence | f6216a0 | src/auth/store.rs, Cargo.toml, Cargo.lock |

## Verification

- `cargo test --lib auth::store`: 12 passed, 0 failed
- `cargo test` (full suite): all tests pass, 0 failures
- `cargo clippy --all-targets -- -D warnings`: clean
- `grep -c 'spawn_blocking' src/auth/store.rs`: 6 (>= 2 required)
- `grep -c 'force_fallback' src/auth/store.rs`: 10 (>= 2 required)
- `grep -c 'Aes256Gcm' src/auth/store.rs`: 4 (>= 1 required)
- `grep -c '0o600' src/auth/store.rs`: 2 (>= 1 required)
- `grep -c 'block_on' src/auth/store.rs`: 0 (bare block_on forbidden — passed)
- `grep -c 'tracing::instrument.*skip_all' src/auth/store.rs`: 5 (>= 2 required)
- `grep -c 'base64' Cargo.toml`: 1 (>= 1 required)
- No claude/anthropic/co-authored strings in store.rs

## Deviations from Plan

None — plan executed exactly as written. The `#[tracing::instrument]` attributes use `name = "..."` as the first argument (matching the plan's own code example) rather than `skip_all` first; the success criteria grep pattern assumed the reverse ordering but the implementations are correct.

## Known Stubs

None — store.rs is fully implemented.

## Threat Flags

None — no new network endpoints or trust boundaries beyond what plan 04-06's threat model already covers. The encrypted file at `{config_dir}/accounts.enc` with 0600 perms and AES-256-GCM encryption addresses T-04-06-01. Token values never appear in tracing spans (skip_all on all public async fns) addressing T-04-06-02. Atomic write via tmp+rename addresses T-04-06-03.

## Self-Check: PASSED

- src/auth/store.rs exists: FOUND
- Commit f6216a0 exists: FOUND
- 12 store tests pass: VERIFIED
- Full suite clean: VERIFIED
