---
phase: 5
slug: java-runtime-management
status: draft
nyquist_compliant: false
wave_0_complete: true
created: 2026-04-21
---

# Phase 5 — Validation Strategy

> Per-phase validation contract. Planner populates the per-task map below.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | `cargo test` + `cargo-nextest` |
| **Config file** | `Cargo.toml` |
| **Quick run command** | `cargo test --lib` |
| **Full suite command** | `cargo test --all-targets` |
| **Estimated runtime** | ~45–60s unit + mocked HTTP; `#[ignore]`-gated live Mojang JRE download integration takes ~60–180s over network |

---

## Sampling Rate

- **After every task commit:** `cargo build && cargo test --lib`
- **After every plan wave:** `cargo test --all-targets && cargo clippy --all-targets -- -D warnings`
- **Before `/gsd-verify-work`:** Full suite + human smoke test (fresh data_dir → create instance → launch → confirm JRE auto-download → title screen). Optional: `cargo test --test jre_live -- --ignored`.

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Test Type | Automated Command | Status |
|---------|------|------|-------------|-----------|-------------------|--------|
| 05-01-01 | 05-01 | 1 | JAVA-01,02,04,05 | unit | `cargo test --lib -- persistence::paths::jre_paths_tests error::error_display_tests` | pass |
| 05-01-02 | 05-01 | 1 | JAVA-04 | unit | `cargo test --lib -- java::types::tests domain::instance::tests` | pass |
| 05-02-01 | 05-02 | 2 | JAVA-01,02 | unit | `cargo test --lib -- java::mapping::tests::mojang_linux_x86_64 java::mapping::tests::mojang_linux_aarch64_is_none` | pass |
| 05-02-02 | 05-02 | 2 | JAVA-03,05 | unit | `cargo test --lib -- java::mapping::tests::parse_legacy_1_8 java::mapping::tests::validate_older_is_err` | pass |
| 05-03-01 | 05-03 | 2 | JAVA-01 | integration-httpmock | `cargo test --lib -- java::mojang_jre::tests::test_install_extracts_file_with_sha1_verify` | pass |
| 05-03-02 | 05-03 | 2 | JAVA-01 | integration-httpmock | `cargo test --lib -- java::mojang_jre::tests::test_install_symlink_on_linux` | pass |
| 05-03-03 | 05-03 | 2 | JAVA-01 | integration-httpmock | `cargo test --lib -- java::mojang_jre::tests::test_install_sha1_mismatch_cleans_tmp` | pass |
| 05-04-01 | 05-04 | 2 | JAVA-02 | integration-httpmock | `cargo test --lib -- java::adoptium::tests::test_fetch_latest_release_parses_array_first_element` | pass |
| 05-04-02 | 05-04 | 2 | JAVA-02 | integration-httpmock | `cargo test --lib -- java::adoptium::tests::test_install_adoptium_linux_strips_prefix` | pass |
| 05-04-03 | 05-04 | 2 | JAVA-02 | unit | `cargo test --lib -- java::adoptium::tests::test_install_adoptium_sha256_mismatch_no_tmp_left` | pass |
| 05-05-01 | 05-05 | 2 | JAVA-03 | unit | `cargo test --lib -- java::detect::tests` | pass |
| 05-06-01 | 05-06 | 3 | JAVA-01,02,04,05 | integration-httpmock | `cargo test --lib -- java::service::tests` | pass |
| 05-06-02 | 05-06 | 3 | JAVA-04 | unit | `cargo test --lib -- java::service::tests::test_set_override_for_instance_persists` | pass |
| 05-07-01 | 05-07 | 4 | JAVA-05 | integration | `cargo test --test launch_integration -- test_launch_fails_early_on_java_mismatch` | pass |
| 05-07-02 | 05-07 | 4 | LAUN-01 (regression) | snapshot | `cargo test --test launch_command` | pass |
| 05-08-01 | 05-08 | 5 | JAVA-03,04 | unit | `cargo test --test tui_smoke -- java_picker` | pass |
| 05-09-01 | 05-09 | 6 | JAVA-01 | live-ignored | `cargo test --test jre_live -- --ignored --nocapture` | pending (human opt-in) |

---

## Wave 0 Requirements

- [x] `flate2 = "1.1.9"` added to Cargo.toml — completed in 05-01
- [x] `tar = "0.4.45"` added — completed in 05-01
- [x] Mojang all.json hash pinned as `DEFAULT_MOJANG_JRE_ALL_URL` const with `MINELTUI_JRE_ALL_URL` env-var override documented — completed in 05-03
- [x] `MINELTUI_JAVA` env var documented as final-priority debug override (demoted from primary) — completed in 05-06 (`resolve_jre_for_launch` step 1; `tracing::warn!` emitted)
- [ ] `tests/fixtures/java/` with pinned Mojang all.json snippet + Adoptium response sample + synthetic tiny tar.gz archive + synthetic tiny zip archive — partial: `mojang_all_snippet.json` and `mojang_variant_manifest.json` live under `tests/fixtures/java/`; synthetic archives are generated in-memory inside the test modules (adopted in 05-04 to avoid parallel-test data-race issues with on-disk fixtures — see 05-04 decision log).

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| Auto-download + launch on fresh data_dir | JAVA-01, JAVA-05 | Requires ~200 MB download + real MC run | Delete `~/.local/share/mineltui`; `cargo run --release` → create 1.20.4 instance → Enter → wait for JRE download → Minecraft reaches title screen |
| Adoptium fallback on aarch64 Linux | JAVA-02 | Requires aarch64 hardware | Deferred to ARM CI when available; logically covered by mocked test |
| System Java detection | JAVA-03 | Requires actual system JDKs installed | On a machine with multiple JDKs: run mineltui; detected list matches installed JDKs |
| Per-instance override | JAVA-04 | Requires real JRE + TUI picker | After detection, open Java picker on an instance; select alternate JDK; launch; confirm in log that selected JDK was used |
| Mismatch validation | JAVA-05 | Requires Java 8 + MC 1.20+ | Override 1.20.4 instance to use Java 8; attempt launch; expect error modal before process spawn |
| Windows JRE path | JAVA-01 via PLAT-02 | Windows-specific paths | Deferred to Phase 12 Windows CI |

---

## Validation Sign-Off

- [x] Per-task map populated (17 rows covering all substantive tasks across plans 05-01 through 05-09)
- [x] Wave 0 deps + fixtures present (flate2, tar added; all.json snippet + variant manifest fixtures in tests/fixtures/java/; synthetic archives generated in-memory)
- [x] No watch-mode flags
- [x] Feedback latency < 45s for unit + mocked-HTTP suite
- [x] SHA1 (Mojang) + SHA256 (Adoptium) verification enforced on downloads
- [ ] `nyquist_compliant: true` only after human checkpoint (Task 3 pending)

**Approval:** pending — awaiting human end-to-end smoke launch (Task 3 checkpoint)
