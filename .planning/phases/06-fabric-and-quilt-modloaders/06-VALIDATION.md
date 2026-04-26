---
phase: 06
slug: fabric-and-quilt-modloaders
status: draft
nyquist_compliant: false
wave_0_complete: true
created: 2026-04-26
---

# Phase 6 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.
> Source: `06-RESEARCH.md` § Validation Architecture.
> Task IDs assigned by `/gsd-plan-phase` 2026-04-26.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | cargo-nextest (dev) / `cargo test` (CI fallback) |
| **Config file** | none — `#[cfg(test)]` modules + `tests/*.rs` |
| **Quick run command** | `cargo test --lib -- loader::` |
| **Full suite command** | `cargo test && cargo test --test '*'` |
| **Estimated runtime** | ~5s quick · ~60s full |

---

## Sampling Rate

- **After every task commit:** Run `cargo test --lib -- loader::`
- **After every plan wave:** Run `cargo test && cargo test --test '*'`
- **Before `/gsd-verify-work`:** Full suite must be green
- **Max feedback latency:** 60 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 06-01-03 | 06-01 | 1 | LOAD-01 | T-06-01 | `maven_coord_to_path` rejects `..`, `/`, `\` (path-traversal V5) | unit | `cargo test --lib -- loader::maven::tests` | ❌ W0 | ⬜ pending |
| 06-01-01 | 06-01 | 1 | LOAD-01,LOAD-02,LOAD-05 | — | LoaderType/LoaderVersionEntry/LoaderInfo serde roundtrip | unit | `cargo test --lib -- loader::types::tests` | ❌ W0 | ⬜ pending |
| 06-01-02 | 06-01 | 1 | LOAD-06 | — | LoaderError variants Display match user-facing copy in UI-SPEC | unit | `cargo test --lib -- loader::error::tests` | ❌ W0 | ⬜ pending |
| 06-02-01 | 06-02 | 2 | LOAD-05 | — | InstanceManifest roundtrip with `loader: Option<LoaderInfo>` field | unit | `cargo test --lib -- domain::instance::tests::test_loader_field_backward_compat` | ✅ existing (extend) | ⬜ pending |
| 06-02-02 | 06-02 | 2 | LOAD-05 | — | None loader omitted from JSON (skip_serializing_if guard) | unit | `cargo test --lib -- domain::instance::tests::test_loader_none_not_serialized` | ✅ existing (extend) | ⬜ pending |
| 06-02-03 | 06-02 | 2 | LOAD-01 | — | Fabric LoaderInfo roundtrip in InstanceManifest | unit | `cargo test --lib -- domain::instance::tests::test_loader_field_roundtrip_fabric` | ✅ existing (extend) | ⬜ pending |
| 06-02-04 | 06-02 | 2 | LOAD-02 | — | Quilt LoaderInfo roundtrip in InstanceManifest | unit | `cargo test --lib -- domain::instance::tests::test_loader_field_roundtrip_quilt` | ✅ existing (extend) | ⬜ pending |
| 06-03-01 | 06-03 | 2 | LOAD-01 | T-06-06 | Fabric meta API list+profile (httpmock) + 5xx/parse-error mapping + sha1 preservation | unit | `cargo test --lib -- loader::fabric::tests` | ❌ W0 | ⬜ pending |
| 06-04-01 | 06-04 | 2 | LOAD-02 | T-06-09 | Quilt v3 list with derived stable + profile no-hash invariant + 5xx/parse-error mapping | unit | `cargo test --lib -- loader::quilt::tests` | ❌ W0 | ⬜ pending |
| 06-05-01 | 06-05 | 3 | LOAD-01 | — | LoaderService dispatches list_loader_versions to Fabric client | unit | `cargo test --lib -- loader::service::tests::test_list_loader_versions_dispatches_fabric` | ❌ W0 | ⬜ pending |
| 06-05-02 | 06-05 | 3 | LOAD-02 | — | LoaderService dispatches list_loader_versions to Quilt client | unit | `cargo test --lib -- loader::service::tests::test_list_loader_versions_dispatches_quilt` | ❌ W0 | ⬜ pending |
| 06-05-03 | 06-05 | 3 | LOAD-05 | — | remove_loader clears manifest.loader and removes versions/{id} dir | unit | `cargo test --lib -- loader::service::tests::test_remove_loader_clears_manifest_and_removes_version_dir` | ❌ W0 | ⬜ pending |
| 06-05-04 | 06-05 | 3 | LOAD-05 | — | remove_loader is no-op when no loader is set | unit | `cargo test --lib -- loader::service::tests::test_remove_loader_noop_when_no_loader` | ❌ W0 | ⬜ pending |
| 06-05-05 | 06-05 | 3 | LOAD-01 | — | Fabric install_loader full mock flow (4-step pipeline) | unit | `cargo test --lib -- loader::service::tests::test_install_fabric_full_flow` | ❌ W0 | ⬜ pending |
| 06-05-06 | 06-05 | 3 | LOAD-02 | — | Quilt install_loader full mock flow (no-hash libraries) | unit | `cargo test --lib -- loader::service::tests::test_install_quilt_full_flow` | ❌ W0 | ⬜ pending |
| 06-05-07 | 06-05 | 3 | LOAD-05 | — | Re-attach: skip downloads when version JSON + libraries already on disk | unit | `cargo test --lib -- loader::service::tests::test_install_skips_when_already_attached` | ❌ W0 | ⬜ pending |
| 06-05-08 | 06-05 | 3 | LOAD-01 | T-06-11 | Fabric library SHA1 verified before write; mismatch → LoaderError::Sha1Mismatch | unit | `cargo test --lib -- loader::service::tests::test_install_sha1_mismatch_returns_sha1mismatch` | ❌ W0 | ⬜ pending |
| 06-05-09 | 06-05 | 3 | LOAD-06 | — | install_loader returns LoaderError::Cancelled when token fires | unit | `cargo test --lib -- loader::service::tests::test_install_cancelled_before_completion_returns_cancelled` | ❌ W0 | ⬜ pending |
| 06-05-10 | 06-05 | 3 | LOAD-05 | T-06-13 | Cancellation never writes instance.json's loader field | unit | `cargo test --lib -- loader::service::tests::test_install_does_not_write_instance_manifest_on_cancel` | ❌ W0 | ⬜ pending |
| 06-05-11 | 06-05 | 3 | LOAD-05 | — | Switch loader = remove + install; final manifest correct | unit | `cargo test --lib -- loader::service::tests::test_switch_loader_via_remove_then_install` | ❌ W0 | ⬜ pending |
| 06-06-01 | 06-06 | 4 | LOAD-01,02,05,06 | T-06-16 | All 19 Action variants update() arms (open/move/select/cancel + install lifecycle + switch confirm) | unit | `cargo test --lib -- tui::app::tests` | ✅ existing (extend) | ⬜ pending |
| 06-07-01 | 06-07 | 5 | LOAD-01,02,05,06 | T-06-19 | 5 view files + view.rs dispatch compile cleanly | smoke | `cargo build` | ❌ W0 | ⬜ pending |
| 06-08-01 | 06-08 | 6 | LOAD-01,02 | — | Loader status cell + block title /L extension | smoke | `cargo build` then UAT Check 1, Check 9 | ❌ W0 (extend) | ⬜ pending |
| 06-08-02 | 06-08 | 6 | LOAD-01,02,05,06 | — | run.rs LoaderService construction + 4 effect arms + L keybind | smoke | `cargo build && cargo test --test tui_smoke` | ❌ W0 (extend) | ⬜ pending |
| 06-08-03 | 06-08 | 6 | LOAD-01,02,05,06 | T-06-20 | 11+ tui_smoke tests covering Phase 6 keybind and state transitions | tui smoke | `cargo test --test tui_smoke` | ❌ W0 (extend) | ⬜ pending |
| 06-09-01 | 06-09 | 7 | LOAD-01,LOAD-02 | T-06-22 | Live Fabric + Quilt install end-to-end with version_id verbatim invariant | live | `cargo test --test loader_live -- --ignored` | ❌ W0 | ⬜ pending |
| 06-09-02 | 06-09 | 7 | LOAD-01,02,05,06 | — | All 9 UAT checks pass (install / switch / remove / cancel / failure / rendering) | manual | 06-HUMAN-UAT.md | ❌ W0 | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

- [ ] `src/loader/mod.rs` — module skeleton (`pub mod types; pub mod error; pub mod maven; pub mod fabric; pub mod quilt; pub mod service;`)
- [ ] `src/loader/types.rs` — `LoaderType`, `LoaderVersionEntry`, `LoaderInfo` structs
- [ ] `src/loader/error.rs` — `LoaderError` enum (thiserror)
- [ ] `src/loader/maven.rs` — `maven_coord_to_path`, `maven_download_url` pure fns + Component::Normal-only validation
- [ ] `src/loader/fabric.rs` — `FabricMetaClient` with httpmock tests
- [ ] `src/loader/quilt.rs` — `QuiltMetaClient` with httpmock tests
- [ ] `src/loader/service.rs` — `LoaderService` facade (mirrors `JavaService` Arc-in-run.rs pattern)
- [ ] `tests/loader_live.rs` — `#[ignore]`-gated live API tests
- [ ] `tests/tui_smoke.rs` — extend with new ActiveView variants from UI-SPEC
- [ ] `src/tui/views/loader_picker_modal.rs` (NEW)
- [ ] `src/tui/views/loader_version_picker_modal.rs` (NEW)
- [ ] `src/tui/views/loader_install_progress_modal.rs` (NEW)
- [ ] `src/tui/views/loader_install_failed_modal.rs` (NEW)
- [ ] `src/tui/views/loader_switch_confirm.rs` (NEW)
- [ ] `src/tui/app.rs` — extend (5 ActiveView, 19 Action, 4 Effect variants)
- [ ] `src/tui/run.rs` — extend (LoaderService Arc, 4 effect arms, L keybind)
- [ ] `src/tui/view.rs` — extend (5 dispatch arms)
- [ ] `src/tui/views/mod.rs` — extend (5 pub mod + 5 pub use)
- [ ] `src/tui/views/instance_list.rs` — extend (loader status cell + /L title)
- [ ] `src/domain/instance.rs` — extend (`loader: Option<LoaderInfo>` field)
- [ ] `src/lib.rs` — extend (`pub mod loader;`)

*Existing `tests/tui_smoke.rs` extended; everything else new.*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| LoaderInstallFailedModal copy renders correctly across terminal sizes | LOAD-06 | TUI rendering at edge widths (80 cols min, narrow terminals) cannot be fully asserted in headless smoke tests | UAT Check 8: install while offline; verify modal copy and dismiss |
| End-to-end Fabric install → launch reaches Minecraft title screen | LOAD-01 | Real Mojang + Fabric meta APIs + real Java; smoke covers logical path but can't replace real launch confirmation | UAT Check 3 + post-UAT manual launch confirmation |
| End-to-end Quilt install → launch reaches title screen | LOAD-02 | Same as above for Quilt | UAT Check 5 + post-UAT manual launch confirmation |
| Loader-status cell renders correctly (`fabric:0.16.9`, `quilt:0.30.0-beta.7`, timestamp, `running`) | LOAD-01, LOAD-02, LOAD-05 | Visual layout under terminal width constraints | UAT Check 9: 80-col + 120-col |
| Switch confirm + remove confirm flows behave as in UI-SPEC | LOAD-05 | Inline confirm bar (DeleteConfirm pattern) interaction is hard to assert without full keystroke harness | UAT Checks 4–6 |

*All other phase behaviors have automated verification in the per-task map above.*

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 60s
- [ ] `nyquist_compliant: true` set in frontmatter (only after end-to-end smoke + automated suite all green)

**Approval:** pending
