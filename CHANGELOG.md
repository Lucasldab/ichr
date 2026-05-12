# Changelog

All notable changes to ichr are recorded here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.2] -- 2026-05-12

### Added

- **`frame_idle` palette slot.** Lets users tune the chrome of every
  frame border independently from the `dim` slot (which now only drives
  placeholder text and similar subdued content). Default keeps v0.3.1
  behavior (frame_idle = DarkGray), so existing configs are unaffected
  -- opt in by setting `frame_idle = "..."` in `[colors]`. Motivating
  use-case: kitty's `inactive_border_color = #3A3A3A` is darker than
  the readable placeholder text color, and v0.3.1's single `dim` slot
  forced one to compromise.

## [0.3.1] -- 2026-05-12

### Fixed

- **User palette in `~/.config/ichr/config.toml` now drives every TUI
  view.** Phase 14 introduced the `[colors]` slot system, but only the
  mod_browser and pack_browser search bars consulted it -- every other
  view still hardcoded `Color::Red/Green/Yellow/DarkGray`, and views
  that used only `Modifier::BOLD/DIM/REVERSED` fell through to the
  terminal default fg. Every fg slot now flows through `state.config.colors`
  across all 21 views (`accent`, `dim`, `error`, `success`, `text`).
- **`selected_bg` and the `accent` "running badge"** were defined in
  the palette but read by no view: selection highlights used
  `Modifier::REVERSED` (terminal inversion) and the instance_list
  running cell used `Modifier::BOLD` only. Both slots are now wired --
  every Table/List highlight paints `bg = palette.selected_bg`, and
  the running cell paints `fg = palette.accent + Modifier::BOLD`.
- **Block borders ignored the palette** -- 60 `Block::default().borders(Borders::ALL)`
  sites rendered borders with the default Style, so the user's `dim`
  slot never reached the frames. New `tui::theme::block(palette)`
  helper preassigns the idle border style to `palette.dim`; every
  view's frames now consult the configured color. Focused search-bar
  borders (mod_browser / pack_browser / cf_browser) still flip to
  `accent` on top of the helper.

## [0.3.0] -- 2026-05-10

This is the first download-able successor to v0.2.0. v0.2.1 was tagged
but its release pipeline failed (see Fixed below), so its features
ship under v0.3.0 alongside the in-progress Phase 14 work that landed
on `main` since the v0.2.1 tag.

### Added

- **Project-icon previews in detail panes** (Phase 13). Modrinth
  `icon_url` and CurseForge `logo.url` are fetched on demand, cached on
  disk (`{cache_dir}/icons/{source}/{project_id}.{ext}`), decoded once
  into a `ratatui-image` `Protocol` held in a 64-entry LRU, and rendered
  into a fixed 8×4 avatar slot at the top-left of the detail pane.
  Wired across the Modrinth mod browser, the resource-pack / shader
  browser, and the CurseForge mod browser.
- **Project-icon previews in browser results rows** (Phase 14, partial).
  Each result row in `mod_browser` and `pack_browser` now shows a 3×2
  cell icon to the left of the project name, with hand-rolled scroll
  offset so the selected row stays visible past the viewport. Same
  `IconService` infrastructure as the detail-pane icons; pre-fetched in
  bulk on results-loaded so the column populates as the user reads the
  first row. **Out of scope for this release:** CurseForge browser
  list-row icons, installed-mods/installed-packs list icons -- both
  pipelines are designed and partially wired (`Effect::ResolveInstalledIcons`,
  `Action::InstalledIconsResolved` exist as stubs); they ship in a
  follow-up.
- **Terminal image-protocol detection at startup** -- `Picker::from_query_stdio()`
  runs in cooked mode before `enable_raw_mode()` and stores the result
  on `AppState`. On detection failure the launcher continues normally
  with icons disabled. Logged at DEBUG so `RUST_LOG=ichr=debug` shows
  the detected protocol.

### Changed

- **Minimum supported Rust version (MSRV)** bumped from 1.88 to 1.90.
  Required by `ratatui-image 10.0.8` -> `icy_sixel` -> `quantette 0.5.1`.
  Forks pinned to 1.88 must update `rust-toolchain.toml` and any local
  `rust-version` pin alongside ichr.
- **Halfblocks-only terminals fall back to text-only** -- on
  gnome-terminal / xterm / Konsole / VS Code's integrated terminal, the
  detail pane and browser results render exactly as in v0.2.0 with no
  icon Rect carve. Halfblocks output at TUI row sizes was verified
  unrecognizable in Spike 001, so showing it would be worse than
  nothing.

### Fixed

- **Windows + Linux release builds** failed in v0.2.1 because
  `ratatui-image`'s default features include `chafa-dyn`, which links
  to libchafa via pkg-config -- neither pkg-config nor libchafa is
  available on the Windows GitHub runner, and the Linux runner lacks
  libchafa-dev. ichr never used chafa (icon rendering goes through
  kitty / sixel / iterm2 / halfblocks instead), so the dep declaration
  is corrected to `default-features = false` with only the features we
  actually use (`crossterm`, `image-defaults`). Both targets now build.
- **Lists scroll past the viewport** in the installed-mods,
  installed-packs, instances, and accounts views. Previously the
  highlighted row could scroll off-screen with no way to bring it back;
  the views now wrap their `Table` widgets in `render_stateful_widget`
  + `TableState` so ratatui computes the scroll offset and keeps the
  selection visible.
- **Offline launch with a cached JRE** no longer fails with
  `GET jre all.json` HTTP errors. `JreService::resolve_jre_for_launch`
  now probes `paths.jre_executable(component)` before contacting
  `piston-meta.mojang.com`; on a cache hit it returns the cached
  executable and skips the network fetch entirely.

### Note on v0.2.1

v0.2.1 was tagged but never released. The chafa-dyn build failure
prevented release artifacts from being produced. Anyone who saw the
v0.2.1 tag should pull v0.3.0 instead -- it ships everything v0.2.1
intended plus the in-progress Phase 14 work.

## [0.2.1] -- 2026-05-10

### Added

- **Project-icon previews in detail panes** (Phase 13). Modrinth `icon_url`
  and CurseForge `logo.url` are fetched on demand, cached on disk
  (`{cache_dir}/icons/{source}/{project_id}.{ext}`), decoded once into a
  `ratatui-image` `Protocol` held in a 64-entry LRU, and rendered into a
  fixed 8×4 avatar slot at the top-left of the detail pane. Wired across
  the Modrinth mod browser, the resource-pack / shader browser, and the
  CurseForge mod browser; the same shape applies to all three.
- **Terminal image-protocol detection at startup** -- `Picker::from_query_stdio()`
  runs in cooked mode before `enable_raw_mode()` and stores the result on
  `AppState`. On detection failure (timeout, non-supporting terminal,
  missing terminfo) the launcher continues normally with icons disabled.
  Logged at DEBUG so `RUST_LOG=ichr=debug` shows the detected protocol.

### Changed

- **Minimum supported Rust version (MSRV)** bumped from 1.88 to 1.90 to
  pull in `ratatui-image 10.0.8` for the icon-preview pipeline. The
  transitive constraint comes from `icy_sixel` → `quantette 0.5.1`,
  which requires Rust 1.90+. Forks pinned to 1.88 must update
  `rust-toolchain.toml` and any local `rust-version` pin alongside ichr.
- **Halfblocks-only terminals fall back to text-only** -- on
  gnome-terminal / xterm / Konsole / VS Code's integrated terminal, the
  detail pane renders as before with no icon Rect carve. Halfblocks
  output at TUI row sizes was verified unrecognizable in Spike 001, so
  showing it would be worse than nothing.

### Fixed

- **Lists scroll past the viewport** in the installed-mods, installed-packs,
  instances, and accounts views. Previously the highlighted row could
  scroll off-screen with no way to bring it back; the views now wrap their
  `Table` widgets in `render_stateful_widget` + `TableState` so ratatui
  computes the scroll offset and keeps the selection visible.
- **Offline launch with a cached JRE** no longer fails with `GET jre all.json`
  HTTP errors. `JreService::resolve_jre_for_launch` now probes
  `paths.jre_executable(component)` before contacting `piston-meta.mojang.com`;
  on a cache hit it returns the cached executable and skips the network
  fetch entirely. Online launches are unchanged (cache-miss still fetches).

## [0.2.0] -- 2026-05-09

### Added

- **Embedded Microsoft AppID** (Mojang-approved 2026-05-08) -- end users
  no longer need to register their own Azure AD app or set
  `ICHR_MSA_CLIENT_ID`. Forks still override via the env var; see
  `docs/msa-setup.md`.
- **User configuration** at `~/.config/ichr/config.toml` (`docs/config.md`):
  - `[colors]` palette with seven semantic slots (accent / dim / error /
    success / info / text / selected_bg). Accepts named ANSI colors or
    `#RRGGBB` hex literals.
  - `[keybinds]` table for fifteen action slots covering global and
    instance-list actions plus the browser search-mode trigger. Wire
    format supports modifier chains (`"Ctrl+Shift+L"`) and treats
    uppercase letters as `Shift+letter` automatically.
  - On-screen hint text (search-bar placeholder, account-management
    footer, browser titles) reads the live keybind label so prompts
    track user overrides.
- **Vim-style search mode** in mod / resource pack / shader pack browsers:
  `/` enters search mode (every printable char types into the buffer),
  Esc exits search mode without closing the browser. Closes the bug
  where queries could not start with `v`, `l`, `j`, or `k` because
  those letters were consumed as filter / nav shortcuts.
- **Pack-install failure modal** mirroring the mod-install failure modal,
  so failures (e.g. a Modrinth filename containing Minecraft formatting
  codes) surface in the UI instead of only in `ichr.log`.

### Fixed

- Modrinth pack filenames containing Minecraft formatting codes (`§6`,
  `§r`) and bracket characters are now sanitized at install time
  instead of rejected. Path-traversal protection is preserved -- inputs
  containing `/`, `\`, or `..` are still refused outright.
- "Installed" tag in mod / pack browser results now appears immediately
  after install completes, rather than only after the user types another
  search character. Backed by an in-memory installed-set on AppState
  that decouples the stamp from the install/search round-trip race.
- Microsoft Account refresh tokens persist correctly. Previously
  `keyring` 3.x was compiled without a platform backend feature, so
  `set_password` returned `Ok` against a stub while later
  `get_password` calls failed with `AccountNotFound` at launch.
  Cargo features now opt into the real OS secret service / kernel
  keyutils backends with the encrypted-file fallback for headless
  hosts.

## [0.1.0] -- 2026-05-08

First public release.

### Added

- Per-instance management with isolated `.minecraft/` directories and
  optional per-instance Java override
- Modloader install pipeline for Fabric, Quilt, Forge, and NeoForge via the
  official installer JARs
- Modrinth integration: search with MC-version + loader filtering, install
  with automatic dependency resolution, version picker per mod
- Modpack import for `.mrpack` (Modrinth) including override-file extraction
  and dependency manifest resolution
- Resource pack and shader pack management (drop-in + Modrinth browse-and-install)
- Java runtime management: Mojang JRE per-version auto-resolution, Adoptium
  fallback for platforms without a Mojang variant, system-Java override
- Microsoft Account authentication via device-code OAuth, with token storage
  in the OS keychain (libsecret on Linux, DPAPI on Windows) or an AES-256-GCM
  encrypted file as fallback
- Windows `longPathAware` manifest embedded in the binary, so the launcher
  itself tolerates deeply nested `%APPDATA%` paths (the JVM child process
  also needs the system-wide `LongPathsEnabled` registry key -- see README)
- Single-binary distribution via `cargo-dist`: Linux x86_64 and Windows x86_64
  release archives published on every tag push, plus shell + PowerShell
  installer scripts
- `cargo install --git` install path verified end-to-end against fresh
  `CARGO_HOME` on Linux and CI-automated on `windows-latest`
- PR-time CI matrix on `ubuntu-latest` + `windows-latest` running fmt,
  clippy, full nextest, plus `cargo publish --dry-run` and an MSRV-parity
  guard between `rust-toolchain.toml` and `Cargo.toml`

### Known limitations

- **Microsoft Account sign-in requires manual Azure AD app registration**
  until the project's own AppID is approved by Microsoft (form submitted --
  weekly review queue). See `docs/msa-setup.md` for the workaround.
- **CurseForge integration is in the codebase but disabled by default**:
  the bundled API key was not granted in time for v1; users may supply
  their own via `CURSEFORGE_API_KEY` env var or `[api_keys] curseforge`
  in `config.toml`.
- **Forge / NeoForge launch on a deeply nested Windows `%APPDATA%`** -- the
  architectural chain that supports this is shipped (longPathAware manifest,
  `@argfile` classpath on Windows, `\\?\` path prefixing) but end-to-end
  empirical UAT on a real Windows 10/11 desktop with Forge has not yet
  been performed by the maintainer (no Windows access). Tracked.
- **macOS and aarch64**: not in v1 scope.
