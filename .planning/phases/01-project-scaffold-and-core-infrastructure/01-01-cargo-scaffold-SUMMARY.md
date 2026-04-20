---
phase: 1
plan: "01"
subsystem: scaffold
tags: [cargo, toolchain, dependencies, rust]
dependency_graph:
  requires: []
  provides: [mineltui-crate, cargo-lock, toolchain-pin]
  affects: [all-downstream-plans]
tech_stack:
  added:
    - ratatui 0.30.0
    - crossterm 0.29.0
    - tokio 1.52.1
    - tokio-util 0.7.18
    - tokio-stream 0.1.18
    - futures 0.3.32
    - serde 1.0.228
    - serde_json 1.0.149
    - toml 1.1.2
    - directories 6.0.0
    - anyhow 1.0.102
    - thiserror 2.0.18
    - tracing 0.1.44
    - tracing-subscriber 0.3.23
    - tracing-appender 0.2.5
    - reqwest 0.13.2
    - sha1 0.10.6
    - sha2 0.11.0
    - zip 8.5.1
  patterns:
    - Single-crate layout (no workspace) for Phase 1
    - Toolchain pinned to numeric channel "1.88" for reproducibility
key_files:
  created:
    - rust-toolchain.toml
    - Cargo.toml
    - Cargo.lock
    - src/main.rs
    - src/lib.rs
  modified:
    - .gitignore
decisions:
  - reqwest feature name changed from rustls-tls to rustls in v0.13 — fixed at build time
metrics:
  duration: ~35min
  completed: 2026-04-20
  tasks_completed: 1
  files_created: 5
  files_modified: 1
---

# Phase 1 Plan 01: Cargo Scaffold Summary

Single task plan that initialized the `mineltui` Rust crate with pinned toolchain 1.88, full locked dependency set for all phases, and a smoke-test binary printing the package version.

## Resolved Versions in Cargo.lock

```
mineltui v0.1.0
├── anyhow v1.0.102
├── crossterm v0.29.0
├── directories v6.0.0
├── futures v0.3.32
├── ratatui v0.30.0
├── reqwest v0.13.2
├── serde v1.0.228
├── serde_json v1.0.149
├── sha1 v0.10.6
├── sha2 v0.11.0
├── thiserror v2.0.18
├── tokio v1.52.1
├── tokio-stream v0.1.18
├── tokio-util v0.7.18
├── toml v1.1.2+spec-1.1.0
├── tracing v0.1.44
├── tracing-appender v0.2.5
├── tracing-subscriber v0.3.23
└── zip v8.5.1
[dev-dependencies]
├── tempfile v3.27.0
└── tokio v1.52.1 (*)
```

## Toolchain Version

```
rustc 1.88.0 (6b00bc388 2025-06-23)
```
Toolchain 1.88-x86_64-unknown-linux-gnu, overridden by `rust-toolchain.toml` at the repo root.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] reqwest feature name changed from `rustls-tls` to `rustls` in v0.13**
- **Found during:** Task 1-01-01, first `cargo build` attempt
- **Issue:** The plan's Cargo.toml snippet specified `features = ["rustls-tls", ...]` for reqwest 0.13. In reqwest 0.13, the feature was renamed from `rustls-tls` to `rustls`. Build failed with "package `reqwest` does not have feature `rustls-tls`".
- **Fix:** Changed feature name to `rustls` in Cargo.toml. Semantic is identical — rustls-based TLS with no OpenSSL dependency. Single-binary distribution is preserved.
- **Files modified:** Cargo.toml
- **Commit:** 7ed1b03

## Verification Results

- `cargo build`: exit 0
- `cargo build --release`: exit 0
- `cargo run --quiet`: prints `mineltui v0.1.0 — scaffold`, exits 0
- `rustup show active-toolchain`: `1.88-x86_64-unknown-linux-gnu (overridden by rust-toolchain.toml)`
- No `[workspace]` table in Cargo.toml
- `Cargo.lock` committed

## Next Plans Unblocked

Plans 02 and 03 (Wave 2) can now start in parallel:
- 01-02: domain types and error hierarchy (`src/domain/`, `src/error.rs`)
- 01-03: persistence/paths.rs, AppPaths struct
- Any downstream plan that needs `cargo test` to run

## Known Stubs

- `src/lib.rs`: no `pub mod` declarations — populated by later plans in Phase 1 as each module is added
- `src/main.rs`: prints greeting only — replaced by TUI event loop in Plan 05

These stubs are intentional per the plan design. `src/lib.rs` is the integration test anchor; `src/main.rs` is the scaffold placeholder.

## Self-Check: PASSED

- rust-toolchain.toml: FOUND
- Cargo.toml: FOUND (contains `name = "mineltui"`, `rust-version = "1.88"`)
- src/main.rs: FOUND
- src/lib.rs: FOUND
- .gitignore: FOUND (contains `target`)
- Cargo.lock: FOUND
- Commit 7ed1b03: FOUND
