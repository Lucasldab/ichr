---
phase: "06"
slug: fabric-and-quilt-modloaders
status: partial
created: 2026-04-26
gate: nyquist_compliant
blocks_phase_completion: true
---

# Phase 6 — Human UAT

> Manual reproduction steps for the modloader install flow.
> Run AFTER `cargo test && cargo test --test '*'` is fully green AND
> `cargo test --test loader_live -- --ignored` succeeds with internet access.

## Prerequisites

- Linux or Windows with a working terminal at ≥80 columns
- Internet access
- At least one MC instance with version `1.21.4` (create via `c` if absent)

## Run Order

1. `cargo test` — full automated suite must be green
2. `cargo test --test loader_live -- --ignored --nocapture` — both Fabric + Quilt live tests must pass
3. `cargo run --release` — proceed through manual checks below

---

## Check 1 — Instance List Header Shows /L Keybind (LOAD-01,LOAD-02)

1. `cargo run --release`
2. Observe the instance list block title.
3. Expected: `Instances (c/r/x/d/g/Enter/s/A/L)` (note the trailing `/L`).
4. ✅ Pass / ❌ Fail (note any title rendering issues)

## Check 2 — L Opens Loader Picker on Vanilla Instance (LOAD-01)

1. Navigate (↑/↓) to a vanilla 1.21.4 instance.
2. Press uppercase `L`.
3. Expected: Loader picker modal appears with three rows:
   - `None (vanilla — remove installed loader)` (highlighted REVERSED)
   - `Fabric Loader ▶`
   - `Quilt Loader ▶`
4. Footer hint shows `↑/k up  ↓/j down  Enter select  Esc cancel`.
5. Press Esc. Modal closes; cursor returns to instance list.
6. ✅ Pass / ❌ Fail

## Check 3 — Install Fabric on a Vanilla Instance (LOAD-01)

1. Press `L` on the vanilla 1.21.4 instance.
2. Move to Fabric (↓ once), press Enter.
3. Expected: Loader Version Picker modal appears titled `Fabric Loader versions — {slug}`.
4. Filter bar shows `/ to filter...` (DarkGray placeholder).
5. Block title hint shows `stable only (s for all)`.
6. List shows Fabric versions; first row REVERSED.
7. Press Enter on the first row.
8. Expected: Install Progress modal appears titled `Installing Fabric {ver} — {slug}`.
9. Progress increments visibly through `Fetching Fabric meta`, `Downloading loader libraries`, `Writing version JSON`. Green LineGauge fills.
10. Modal closes when complete; instance list shows status cell `fabric:{ver_short}` (where `ver_short` is first 6 chars of the loader version).
11. ✅ Pass / ❌ Fail (note any progress / final state issues)

## Check 4 — Switch Fabric Version on Same Instance (LOAD-05)

1. Press `L` on the now-Fabric instance.
2. Move to Fabric, press Enter.
3. Expected: a row is suffixed `← currently installed` (DIM) — the version you just installed.
4. Move to a DIFFERENT row and press Enter.
5. Expected: `Switch loader?` confirm appears with `Switch {slug} from fabric:{cur} to fabric:{new}?`.
6. Press `y`. Expected: install progress; final status cell shows new version.
7. ✅ Pass / ❌ Fail

## Check 5 — Switch Loader Type Fabric → Quilt (LOAD-05, type warning)

1. Press `L` on the Fabric instance.
2. Move to Quilt, press Enter.
3. Expected: Quilt Loader Version Picker, with header note `(all versions are pre-release)`.
4. Press Enter on the first version.
5. Expected: `Switch loader?` confirm appears with the RED + BOLD warning line:
   `WARNING: switching loader type may break installed mods.`
6. Press `y`. Install progress modal; eventually status cell shows `quilt:{ver}`.
7. ✅ Pass / ❌ Fail

## Check 6 — Remove Loader (Back to Vanilla) (LOAD-05)

1. Press `L` on the now-Quilt instance.
2. Cursor is on `None (vanilla — remove installed loader)` (default selected).
3. Press Enter.
4. Expected: `Remove loader?` confirm with `Remove loader from {slug}?`.
5. Press `y`. Expected: status cell reverts to last-played-date (no loader prefix).
6. ✅ Pass / ❌ Fail

## Check 7 — Cancel Install Mid-Stream (LOAD-06)

1. Press `L` on a vanilla instance, choose Fabric, choose any version.
2. While progress modal shows < 50%, press Esc.
3. Expected: Modal closes; instance list status cell does NOT change (stays at last-played).
4. (Verify by re-pressing `L` — picker should NOT show `← currently installed` anywhere.)
5. ✅ Pass / ❌ Fail

## Check 8 — Install Failure UX (LOAD-06)

1. Disable network (airplane mode / `iptables` / disconnect WiFi).
2. Press `L`, choose Fabric, attempt install.
3. Expected: `LoaderInstallFailedModal` titled `Install failed: {slug}   (Esc to dismiss)`.
4. Body shows `Fabric {ver} installation failed: Failed to fetch fabric meta: ...`.
5. Press Esc. Modal closes; instance list shown.
6. Restore network for cleanup.
7. ✅ Pass / ❌ Fail

## Check 9 — Loader-Status Cell at 80-col and 120-col (LOAD-01,02)

1. Resize terminal to exactly 80 columns. `cargo run --release`.
2. Verify status cell `fabric:0.16.9` (or similar) renders without truncating column boundaries.
3. Resize to 120 columns. Re-render.
4. Verify same — column does not overflow.
5. ✅ Pass / ❌ Fail

---

## Sign-off

- [ ] All 9 checks pass
- [ ] `cargo test` full suite green
- [ ] `cargo test --test loader_live -- --ignored` green (both Fabric + Quilt)

After all three boxes are checked, edit `06-VALIDATION.md` frontmatter:
```
nyquist_compliant: true
wave_0_complete: true
```

Then update this file:
- `status: partial` → `status: resolved`

## Resolution

When all non-skipped checks pass:
1. Flip `nyquist_compliant: true` and `status: validated` in `06-VALIDATION.md` frontmatter
2. Update `06-09-integration-validation-SUMMARY.md`: `status: partial` → `status: complete`, `tasks_completed: 2` → `3`
3. Update this file: `status: partial` → `status: resolved`
