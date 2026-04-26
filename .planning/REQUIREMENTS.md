# Requirements: mineltui

**Defined:** 2026-04-20
**Core Value:** A user can create an instance, install a modloader and mods, and launch a working modded Minecraft — entirely from the TUI.

## v1 Requirements

Requirements for initial release. Each maps to a roadmap phase.

### Instance Management

- [x] **INST-01**: User can create a new instance by choosing a name and Minecraft version
- [x] **INST-02**: User can view a list of all existing instances with status (version, modloader, last played)
- [x] **INST-03**: User can clone an existing instance (copies config, mods, modloader; fresh saves optional)
- [x] **INST-04**: User can rename an existing instance
- [x] **INST-05**: User can delete an instance with confirmation (removes instance directory)
- [x] **INST-06**: User can group instances into folders/tags for organization

### Minecraft Versions

- [ ] **VERS-01**: User can view the Mojang version manifest (releases + snapshots, filterable by type)
- [x] **VERS-02**: User can select any release or snapshot version when creating an instance
- [x] **VERS-03**: Launcher resolves `inheritsFrom` chains correctly when installing a version
- [x] **VERS-04**: Launcher correctly parses both `minecraftArguments` (<1.13) and `arguments` (>=1.13) formats
- [x] **VERS-05**: Launcher downloads and stores client jar, libraries, and asset index per Mojang manifest
- [x] **VERS-06**: Launcher extracts legacy classifier-style natives (<1.19) and handles embedded natives (>=1.19)
- [x] **VERS-07**: Launcher evaluates library `rules` (os / name / arch / features) correctly per-platform

### Java Runtime

- [x] **JAVA-01**: Launcher auto-downloads Mojang-blessed JRE matching the target MC version when available
- [x] **JAVA-02**: Launcher falls back to Adoptium for versions without a Mojang-blessed runtime
- [x] **JAVA-03**: Launcher scans PATH and common install locations for system-installed Java runtimes
- [x] **JAVA-04**: User can override Java per instance (choose auto-downloaded or system-detected JDK)
- [x] **JAVA-05**: Launcher validates Java major version against MC requirements before launch (fail loud, not silent)

### Accounts

- [x] **AUTH-01**: User can authenticate with a Microsoft account via the OAuth device-code flow
- [x] **AUTH-02**: Launcher completes the full MSA → XSTS → Minecraft services chain and stores entitlement state
- [x] **AUTH-03**: Launcher refreshes expired access tokens transparently on next launch
- [x] **AUTH-04**: Launcher stores refresh tokens securely (OS keychain: libsecret / DPAPI; fallback with explicit warning)
- [x] **AUTH-05**: User can launch in offline mode with an arbitrary username, no Microsoft account required
- [x] **AUTH-06**: User can sign in with multiple Microsoft accounts and switch between them per launch

### Modloaders

- [ ] **LOAD-01**: User can install Fabric Loader on an instance (select loader version; JSON manifest merge)
- [x] **LOAD-02**: User can install Quilt Loader on an instance
- [ ] **LOAD-03**: User can install Forge on an instance (runs installer JAR via tokio subprocess; streams progress)
- [ ] **LOAD-04**: User can install NeoForge on an instance (same subprocess flow, NeoForge-specific endpoints)
- [ ] **LOAD-05**: User can switch loader or loader version on an existing instance
- [ ] **LOAD-06**: Launcher surfaces installer subprocess failures with captured stdout/stderr.
  - **Note (Phase 6 substitution for Fabric/Quilt):** Fabric and Quilt are pure-HTTP loaders with no installer subprocess. For these loaders the requirement is realized by `LoaderError::Display` populating the failure modal headline plus the HTTP/IO error detail populating the `log_tail` field — capturing the spirit of "surface failures with detail" without literal stdout/stderr. Literal subprocess stdout/stderr capture lives in Phase 7 for Forge/NeoForge.

### Mod Management

- [ ] **MOD-01**: User can browse Modrinth mods with search, MC-version filter, and loader filter
- [ ] **MOD-02**: User can install a Modrinth mod into an instance (with automatic dependency resolution)
- [ ] **MOD-03**: User can browse CurseForge mods with search, MC-version filter, and loader filter
- [ ] **MOD-04**: User can install a CurseForge mod into an instance (handles `downloadUrl: null` with fallback UX)
- [ ] **MOD-05**: User can view installed mods per instance (name, version, source, enabled state)
- [ ] **MOD-06**: User can enable/disable an installed mod (toggles .jar ↔ .jar.disabled)
- [ ] **MOD-07**: User can uninstall a mod from an instance
- [ ] **MOD-08**: Launcher stores a CurseForge API key (compiled-in default + env-var override)

### Modpacks

- [ ] **PACK-01**: User can import a Modrinth `.mrpack` file as a new instance
- [ ] **PACK-02**: `.mrpack` import honors `env.client` filter and downloads all files
- [ ] **PACK-03**: `.mrpack` import applies override files from the archive correctly
- [ ] **PACK-04**: User can import a CurseForge-format modpack zip as a new instance
- [ ] **PACK-05**: CurseForge modpack import resolves mods by project/file ID via the CurseForge API
- [ ] **PACK-06**: Launcher cancels modpack imports cleanly mid-stream (no half-installed instance)

### Resource Packs

- [ ] **RPAK-01**: User can drop a resource pack `.zip` into an instance via file picker or path input
- [ ] **RPAK-02**: User can browse and install resource packs from Modrinth for an instance
- [ ] **RPAK-03**: User can view, enable, and remove installed resource packs per instance

### Shader Packs

- [ ] **SPAK-01**: User can drop a shader pack `.zip` into an instance via file picker or path input
- [ ] **SPAK-02**: User can browse and install shader packs from Modrinth for an instance
- [ ] **SPAK-03**: User can view and remove installed shader packs per instance

### Launch

- [x] **LAUN-01**: User can launch an instance; launcher composes full JVM command (args, classpath, assets, main class)
- [x] **LAUN-02**: Launcher uses a Java 9+ `@argfile` on Windows to avoid MAX_PATH / command-length limits
- [x] **LAUN-03**: Launcher drains Minecraft stdout/stderr asynchronously to a per-instance log file (prevents pipe deadlock)
- [x] **LAUN-04**: Launcher surfaces launch failures with captured log tail (even without a live log viewer UI)
- [x] **LAUN-05**: Launcher cleans up natives directory and process handles on instance exit
- [x] **LAUN-06**: User can cancel a long-running install/download at any point (CancellationToken throughout)

### Platform Support

- [x] **PLAT-01**: Launcher runs on Linux with XDG-compliant paths (`~/.local/share/mineltui`, `~/.config/mineltui`, cache dir)
- [x] **PLAT-02**: Launcher runs on Windows with `%APPDATA%\mineltui` paths and Windows-style classpath separators
- [x] **PLAT-03**: Launcher detects architecture (x86_64 / aarch64) and applies Mojang library rules accordingly
- [ ] **PLAT-04**: Launcher binary declares `longPathAware` on Windows to tolerate deeply nested instance paths

### Distribution

- [ ] **DIST-01**: Users can install via `cargo install mineltui` from crates.io (or the repo until publish)
- [ ] **DIST-02**: Each GitHub release publishes prebuilt Linux x86_64 and Windows x86_64 binaries
- [ ] **DIST-03**: Release pipeline uses cargo-dist (or equivalent) wired to GitHub Actions
- [ ] **DIST-04**: `rust-toolchain.toml` pins MSRV (>= 1.88 per zip crate requirement)

## v2 Requirements

Deferred to future release. Tracked but not in current roadmap.

### Observability

- **LOG-01**: Live log viewer pane (stream stdout/stderr during play, scrollback, search)
- **LOG-02**: Crash log surfacing with highlighted stack frames
- **LOG-03**: In-TUI log filtering (level, source, pattern)

### Servers

- **SRV-01**: Per-instance server list (add / edit / remove entries)
- **SRV-02**: Server status ping (online/offline, players, MOTD)
- **SRV-03**: Quick-join from instance detail view

### Worlds

- **WORLD-01**: Per-instance world browser (list, rename, delete)
- **WORLD-02**: World backup / restore
- **WORLD-03**: World import from `.zip` / other instance

### Platform Expansion

- **PLAT-v2-01**: macOS support (aarch64 + x86_64)
- **PLAT-v2-02**: aarch64 Linux binaries in releases

### Misc

- **MISC-01**: Launcher self-update via GitHub Releases
- **MISC-02**: Import from existing official/Prism launchers
- **MISC-03**: Screenshot browser

## Out of Scope

| Feature | Reason |
|---------|--------|
| Minecraft Bedrock Edition | Entirely separate launcher surface; not Java |
| Old beta / alpha versions | Manifest filter excludes; niche, launcher complexity not worth it |
| Realms subscription management | Launching into Realms is fine via online auth; managing subscription is out |
| Skin rendering in TUI | TUI medium fights image rendering; open in external tool if needed |
| Screenshot thumbnails in TUI | Same as above |
| In-TUI chat / rich media | Not a launcher concern |
| Custom asset modding (e.g., texture editing) | Out of launcher scope |
| Backend proxy for CurseForge requests | Infrastructure scope; use compiled-in key + env override instead |

## Traceability

Which phases cover which requirements. Populated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| PLAT-01 | Phase 1 | Complete |
| PLAT-02 | Phase 1 | Complete |
| PLAT-03 | Phase 1 | Complete |
| DIST-04 | Phase 1 | Pending |
| INST-01 | Phase 2 | Complete |
| INST-02 | Phase 2 | Complete |
| INST-03 | Phase 2 | Complete |
| INST-04 | Phase 2 | Complete |
| INST-05 | Phase 2 | Complete |
| INST-06 | Phase 2 | Complete |
| VERS-01 | Phase 2 | Pending |
| VERS-02 | Phase 2 | Complete |
| VERS-03 | Phase 2 | Complete |
| VERS-04 | Phase 2 | Complete |
| VERS-05 | Phase 2 | Complete |
| VERS-06 | Phase 2 | Complete |
| VERS-07 | Phase 2 | Complete |
| LAUN-01 | Phase 3 | Complete |
| LAUN-02 | Phase 3 | Complete |
| LAUN-03 | Phase 3 | Complete |
| LAUN-04 | Phase 3 | Complete |
| LAUN-05 | Phase 3 | Complete |
| LAUN-06 | Phase 3 | Complete |
| AUTH-05 | Phase 3 | Complete |
| AUTH-01 | Phase 4 | Complete |
| AUTH-02 | Phase 4 | Complete |
| AUTH-03 | Phase 4 | Complete |
| AUTH-04 | Phase 4 | Complete |
| AUTH-06 | Phase 4 | Complete |
| JAVA-01 | Phase 5 | Complete |
| JAVA-02 | Phase 5 | Complete |
| JAVA-03 | Phase 5 | Complete |
| JAVA-04 | Phase 5 | Complete |
| JAVA-05 | Phase 5 | Complete |
| LOAD-01 | Phase 6 | Pending |
| LOAD-02 | Phase 6 | Complete |
| LOAD-05 | Phase 6 | Pending |
| LOAD-06 | Phase 6 | Pending |
| LOAD-03 | Phase 7 | Pending |
| LOAD-04 | Phase 7 | Pending |
| MOD-01 | Phase 8 | Pending |
| MOD-02 | Phase 8 | Pending |
| MOD-05 | Phase 8 | Pending |
| MOD-06 | Phase 8 | Pending |
| MOD-07 | Phase 8 | Pending |
| MOD-03 | Phase 9 | Pending |
| MOD-04 | Phase 9 | Pending |
| MOD-08 | Phase 9 | Pending |
| PACK-01 | Phase 10 | Pending |
| PACK-02 | Phase 10 | Pending |
| PACK-03 | Phase 10 | Pending |
| PACK-04 | Phase 10 | Pending |
| PACK-05 | Phase 10 | Pending |
| PACK-06 | Phase 10 | Pending |
| RPAK-01 | Phase 11 | Pending |
| RPAK-02 | Phase 11 | Pending |
| RPAK-03 | Phase 11 | Pending |
| SPAK-01 | Phase 11 | Pending |
| SPAK-02 | Phase 11 | Pending |
| SPAK-03 | Phase 11 | Pending |
| PLAT-04 | Phase 12 | Pending |
| DIST-01 | Phase 12 | Pending |
| DIST-02 | Phase 12 | Pending |
| DIST-03 | Phase 12 | Pending |

**Coverage:**
- v1 requirements: 64 total (note: REQUIREMENTS.md previously stated 56; actual count from requirement IDs is 64)
- Mapped to phases: 64
- Unmapped: 0 ✓

---
*Requirements defined: 2026-04-20*
*Last updated: 2026-04-20 after roadmap creation*
