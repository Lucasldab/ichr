---
gsd_state_version: 1.0
milestone: v1.0
milestone_name: milestone
status: executing
stopped_at: Completed 05-05-system-detect-PLAN.md
last_updated: "2026-04-22T02:36:40.147Z"
last_activity: 2026-04-22
progress:
  total_phases: 12
  completed_phases: 4
  total_plans: 40
  completed_plans: 36
  percent: 90
---

# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-04-20)

**Core value:** A user can create an instance, install a modloader and mods, and launch a working modded Minecraft — entirely from the TUI.
**Current focus:** Phase 5 — Java Runtime Management

## Current Position

Phase: 5 (Java Runtime Management) — EXECUTING
Plan: 6 of 9
Status: Ready to execute
Last activity: 2026-04-22

Progress: [░░░░░░░░░░] 0%

## Performance Metrics

**Velocity:**

- Total plans completed: 0
- Average duration: —
- Total execution time: 0 hours

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| - | - | - | - |

**Recent Trend:**

- Last 5 plans: —
- Trend: —

*Updated after each plan completion*
| Phase 1 P2 | 10 | 1 tasks | 4 files |
| Phase 1 P3 | 5 | 1 tasks | 8 files |
| Phase 1 P04 | 140 | 1 tasks | 6 files |
| Phase 01 P05 | 2min | 1 tasks | 4 files |
| Phase 1 P06 | 25m | 1 tasks | 10 files |
| Phase 2 P01 | 25m | 3 tasks | 10 files |
| Phase 2 P02 | 8 | 3 tasks | 10 files |
| Phase 2 P04 | 35 | 2 tasks | 9 files |
| Phase 2 P05 | 177s | 2 tasks | 5 files |
| Phase 2 P06 | 275 | 2 tasks | 5 files |
| Phase 03-launcher-process-and-offline-launch P01 | 15 | 2 tasks | 12 files |
| Phase 03-launcher-process-and-offline-launch P02 | 5 | 2 tasks | 5 files |
| Phase 03 P03 | 10 | 1 tasks | 2 files |
| Phase 03-launcher-process-and-offline-launch P04 | 25 | 2 tasks | 3 files |
| Phase 03-launcher-process-and-offline-launch P05 | 30 | 2 tasks | 7 files |
| Phase 04 P01 | 15 | 2 tasks | 14 files |
| Phase 04 P02 | 5 | 1 tasks | 1 files |
| Phase 04 P03 | 15 | 2 tasks | 2 files |
| Phase 04 P04 | 15 | 1 tasks | 2 files |
| Phase 04 P05 | 12 | 1 tasks | 1 files |
| Phase 04 P06 | 25 | 1 tasks | 3 files |
| Phase 04 P07 | 10 | 1 tasks | 1 files |
| Phase 04 P08 | 217 | 1 tasks | 6 files |
| Phase 04 P09 | 45 | 3 tasks | 9 files |
| Phase 05 P01 | 15 | 2 tasks | 8 files |
| Phase 05 P02 | 12 | 2 tasks | 2 files |
| Phase 05 P03 | 18 | 2 tasks | 4 files |
| Phase 05 P04 | 25 | 2 tasks | 2 files |
| Phase 05 P05 | 15 | 1 tasks | 2 files |

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- Roadmap: Phases 3 and 4 (launcher + auth) may be worked in parallel after Phase 2 completes
- Roadmap: Phase 5 (Java management) must complete before Phase 7 (Forge/NeoForge) begins
- Roadmap: DIST-04 (rust-toolchain.toml MSRV pin) assigned to Phase 1 as a foundational step
- [Phase 1]: Accept directories crate Windows data suffix rather than stripping — documented in paths.rs
- [Phase 1]: Arch::current() uses str_eq const helper (not match) because Rust 1.88 const fn lacks str pattern matching
- [Phase 1]: AppError defined with thiserror at library layer; anyhow reserved for main.rs/TUI boundary
- [Phase 1]: TaskEvent decoupled from TUI Action enum so tasks/ is testable standalone
- [Phase 1]: Semaphore permit acquired before job body runs, bounding execution not submission
- [Phase 1]: biased select! always catches cancellation even if job body ignores token
- [Phase 01]: Return WorkerGuard directly from logging::init — caller must bind to named variable per PITFALLS.md Pitfall 3
- [Phase 01]: Use try_init (not init) for global subscriber — returns Err on double-init instead of panicking
- [Phase 1]: Panic hook installed before enable_raw_mode so setup failures also restore terminal
- [Phase 1]: AppState initialized with struct update syntax (arch/os inline) to satisfy clippy::field_reassign_with_default
- [Phase 2]: ratatui-textarea 0.9.1 pinned: depends on ratatui-core ^0.1.0 + ratatui-widgets ^0.3.0 which ratatui 0.30 re-exports — no version conflict
- [Phase 2]: Snapshot fixture is 26.2-snapshot-3; asset_object hash validation is caller responsibility (documented in doc-comment)
- [Phase 2]: MAX_INHERITS_DEPTH=3 means 3-node max chain (2 hops); 4-node chains rejected; check fires at depth+1>=MAX
- [Phase 2]: inheritsFrom resolver is pure-sync taking pre-populated HashMap<String, VersionJson>; no async/block_on anywhere
- [Phase 2]: resolve_game_args prefers structured arguments over minecraftArguments; unknown feature flags conservatively disallow
- [Phase 2]: time 0.3 added for RFC3339 formatting (std+formatting features only)
- [Phase 2]: write_instance_manifest creates .minecraft subtree eagerly; atomic_write from 02-03 reused
- [Phase 2]: clone_instance writes manifest LAST so failed mid-copy leaves orphan dir without instance.json; list_instances skips such dirs
- [Phase 2]: copy_tree uses BFS queue instead of async recursion to avoid BoxFuture/pin requirement
- [Phase 2]: Two distinct Semaphores (LIB_CONCURRENCY=8, ASSET_CONCURRENCY=16): library downloads are CDN-friendly at 8; asset objects (4000+) benefit from 16 to hide latency
- [Phase 2]: collect_inherits_chain does async network walk before pure-sync resolve_inherits — eliminates Handle::block_on deadlock risk on tokio multi-thread workers
- [Phase 2]: safe_extract_path: only Component::Normal accepted — ZIP path traversal structurally impossible
- [Phase 03-launcher-process-and-offline-launch]: SpawnFailed(String) not SpawnFailed(#[from] io::Error) to avoid conflicting From<io::Error> impl
- [Phase 03-launcher-process-and-offline-launch]: md-5 = { version = 0.10, default-features = false } pinned (RustCrypto family, matches sha1/sha2)
- [Phase 03-launcher-process-and-offline-launch]: substitute() uses ordered String::replace sweep; classpath_separator() uses cfg!(target_os) build-time constant; LEGACY_JVM_ARGS fallback for pre-1.13; no hardcoded Player UUID test vector (structural properties asserted instead)
- [Phase 03]: spawn.rs takes flat argv not LaunchCommand to keep 03-02 and 03-03 buildable in parallel
- [Phase 03]: std::sync::Mutex used on ring buffer (never held across .await) to avoid async lock anti-pattern
- [Phase 03-04]: Java resolved via MINELTUI_JAVA env var then PATH 'java' fallback; Phase 5 replaces with per-instance JRE auto-download
- [Phase 03-04]: Cancelled does not update play_time_ms; only clean exit triggers update_play_time
- [Phase 03-launcher-process-and-offline-launch]: LaunchJobStarted action preserves single-mutation-point invariant for running_instances (token inserted via Action, not directly in execute_effects)
- [Phase 04]: Use i64 unix epoch seconds for all token expiry timestamps in Account (avoids SystemTime serde complexity)
- [Phase 04]: AuthError::XstsDenied uses named fields { xerr: u64, message: String } matching plan spec
- [Phase 04]: AppPaths::accounts_json_file() added alongside accounts_file() per plan must_haves
- [Phase 04]: XErr 2148916236 and 2148916237 kept as separate match arms for distinct user messages
- [Phase 04]: httpmock 0.8.3 uses body_includes (not body_contains) for substring body matching
- [Phase 04]: XstsErrorBody.message discarded in favour of map_xerr output for consistent user-facing XSTS errors
- [Phase 04]: Hand-rolled reqwest POSTs for MSA device-code instead of oauth2 5.0 crate for httpmock base_url injection
- [Phase 04]: Added reqwest form feature in Cargo.toml — required for .form() in reqwest 0.13 with default-features=false
- [Phase 04]: ensure_valid_mc_token always refreshes regardless of token age (MC tokens expire 24h; cannot safely skip refresh)
- [Phase 04]: AuthChainOutput includes refresh_token + expiry timestamps so store.rs can persist without unpacking Account
- [Phase 04]: Encrypted file uses JSON map {account_id -> base64(nonce||ciphertext)} for per-entry granularity without full re-encryption on update
- [Phase 04]: StoreConfig.force_fallback skips keyring entirely for deterministic CI tests where libsecret daemon is absent
- [Phase 04]: list_accounts also gets #[tracing::instrument(skip_all)] for consistent all-methods tracing policy
- [Phase 04]: resolve_msa_tokens_for_launch returns MsaTokens only — correct return for launcher integration path
- [Phase 04]: MsaAuth adapter struct lives in offline.rs alongside OfflineAuth — all auth-field production in one place
- [Phase 04]: compose_msa is a parallel function to compose — avoids conditional branching in the hot path
- [Phase 04]: account_service injected as Option<&AccountService> — testable without real account store; None returns AppError::NoActiveAccount
- [Phase 04]: AuthContext built in update() and carried in Effect::LaunchInstance to preserve single-mutation-point invariant
- [Phase 04]: AddAccountTokenCreated Action used to store CancellationToken into state (not passed via closure)
- [Phase 05]: JavaRuntimeId placed in src/java/types.rs to keep java module self-contained
- [Phase 05]: jres_dir() aliases runtime_dir() per research Pattern 6 — same on-disk path, semantic clarity
- [Phase 05]: Redundant guard pattern replaced with literal match arm (Other("x86")) per clippy::redundant_guards
- [Phase 05]: adoptium_arch_str falls back to x64 for unknown architectures rather than panicking
- [Phase 05]: DEFAULT_MOJANG_JRE_ALL_URL pinned with inline hash-rotation comment; MINELTUI_JRE_ALL_URL env var for runtime override
- [Phase 05]: AdoptiumClient::new_with_base_url() added to avoid env-var races in parallel tests
- [Phase 05]: sha256_hex uses iter().fold() because sha2 0.11 Array lacks LowerHex
- [Phase 05]: No glob/which crate — PATH split manually; read_dir for common dirs

### Pending Todos

None yet.

### Blockers/Concerns

- Phase 4: `keyring` crate may perform blocking I/O — wrap in `spawn_blocking`; validate early in Phase 4
- Phase 7: Confirm exact Forge version cutoff for post-processor installer format (~1.12.2) during Phase 7 planning
- Phase 9: Verify `furse` crate maintenance on crates.io before Phase 9 begins; hand-roll ~8 endpoints if stale
- Phase 9: CurseForge API key must be obtained from the API portal and stored as a GitHub Actions secret before Phase 9 begins

## Deferred Items

| Category | Item | Status | Deferred At |
|----------|------|--------|-------------|
| *(none)* | | | |

## Session Continuity

Last session: 2026-04-22T02:36:40.144Z
Stopped at: Completed 05-05-system-detect-PLAN.md
Resume file: None
