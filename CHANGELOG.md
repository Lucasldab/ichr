# Changelog

All notable changes to ichr are recorded here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Pending

- Microsoft AppID approval — once granted, the launcher's client ID will be
  embedded so first-run no longer requires the `ICHR_MSA_CLIENT_ID` env var
- Configurable keybinds and color theme via TOML config file
- Search-bar focus mode in mod / pack browsers (`/` to enter search,
  unblocking searches that start with shadowed letters like `j`, `k`, `v`, `l`)

## [0.1.0] — 2026-05-08

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
  also needs the system-wide `LongPathsEnabled` registry key — see README)
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
  until the project's own AppID is approved by Microsoft (form submitted —
  weekly review queue). See `docs/msa-setup.md` for the workaround.
- **CurseForge integration is in the codebase but disabled by default**:
  the bundled API key was not granted in time for v1; users may supply
  their own via `CURSEFORGE_API_KEY` env var or `[api_keys] curseforge`
  in `config.toml`.
- **Forge / NeoForge launch on a deeply nested Windows `%APPDATA%`** — the
  architectural chain that supports this is shipped (longPathAware manifest,
  `@argfile` classpath on Windows, `\\?\` path prefixing) but end-to-end
  empirical UAT on a real Windows 10/11 desktop with Forge has not yet
  been performed by the maintainer (no Windows access). Tracked.
- **macOS and aarch64**: not in v1 scope.
