# Roadmap: mineltui

## Overview

mineltui is a terminal-UI Minecraft Java Edition launcher built in Rust + ratatui, modeled on Prism Launcher. The roadmap follows the dependency graph of the architecture: core infrastructure and platform path resolution first, then the Mojang protocol and instance management layer, then the launcher process, then authentication, then Java runtime management (required before Forge can run), then modloaders in complexity order (Fabric/Quilt before Forge/NeoForge), then the mod and modpack ecosystems, then content packs, and finally Windows polish and distribution. Each phase delivers a coherent, independently verifiable capability.

## Phases

**Phase Numbering:**
- Integer phases (1–12): Planned milestone work

- [x] **Phase 1: Project Scaffold and Core Infrastructure** - Compiling binary with platform paths, domain types, task system, and TUI skeleton (completed 2026-04-20)
- [x] **Phase 2: Mojang Protocol and Instance Management** - Instance CRUD, version manifest, file download pipeline, and asset install (completed 2026-04-20)
- [x] **Phase 3: Launcher Process and Offline Launch** - JVM arg composition, classpath, natives, process spawn, offline mode (completed 2026-04-21)
- [x] **Phase 4: Microsoft Authentication** - Device-code OAuth, full MSA→XSTS→MC chain, token storage, multi-account (completed 2026-04-22)
- [ ] **Phase 5: Java Runtime Management** - Auto-download Mojang/Adoptium JREs, system detection, per-instance selection
- [ ] **Phase 6: Fabric and Quilt Modloaders** - Install Fabric and Quilt per instance, modloader version switching
- [ ] **Phase 7: Forge and NeoForge Modloaders** - Subprocess-driven JVM processor pipeline for Forge and NeoForge
- [ ] **Phase 8: Modrinth Integration** - Browse, install, manage mods from Modrinth with dependency resolution
- [ ] **Phase 9: CurseForge Integration** - Browse and install mods from CurseForge with API key management
- [ ] **Phase 10: Modpack Import** - Import Modrinth .mrpack and CurseForge modpack zip archives
- [ ] **Phase 11: Resource Packs and Shader Packs** - Drop-in and Modrinth-browse install for resource packs and shader packs
- [ ] **Phase 12: Windows Polish and Distribution** - longPathAware manifest, cargo-dist pipeline, release binaries

## Phase Details

### Phase 1: Project Scaffold and Core Infrastructure
**Goal**: A compiling Rust binary exists with platform-correct path resolution, domain types, a task system capable of bounded concurrent async jobs, and a minimal TUI event loop that responds to input — everything subsequent phases build on.
**Depends on**: Nothing (first phase)
**Requirements**: PLAT-01, PLAT-02, PLAT-03, DIST-04
**Success Criteria** (what must be TRUE):
  1. Binary compiles and runs on Linux; pressing `q` exits the TUI cleanly without corrupting the terminal
  2. Data, config, and cache paths resolve to XDG directories on Linux and `%APPDATA%` on Windows without hardcoded strings
  3. Architecture (x86_64 / aarch64) is detected and available to all downstream components
  4. A background job can be spawned, report progress to the TUI, and be cancelled via CancellationToken without a race or deadlock
  5. `rust-toolchain.toml` is present and pins MSRV >= 1.88; `cargo build` succeeds on that toolchain
**Plans**: TBD
**UI hint**: yes

### Phase 2: Mojang Protocol and Instance Management
**Goal**: Users can create, list, clone, rename, delete, and organize game instances; the launcher fetches Minecraft versions from Mojang, downloads and verifies client jar, libraries, asset objects, and natives for any selected version.
**Depends on**: Phase 1
**Requirements**: INST-01, INST-02, INST-03, INST-04, INST-05, INST-06, VERS-01, VERS-02, VERS-03, VERS-04, VERS-05, VERS-06, VERS-07
**Success Criteria** (what must be TRUE):
  1. User can create an instance by choosing a name and Minecraft version (release or snapshot) and see it appear in the instance list with status information
  2. User can clone, rename, and delete an existing instance (delete requires confirmation); instances can be grouped into folders or tags
  3. The version picker shows the full Mojang manifest with release/snapshot toggle and filtering; any listed version can be selected
  4. After selecting a version, all required files (client.jar, libraries, asset objects, natives) are downloaded, SHA1-verified, and stored in the shared data directory; progress is visible in the TUI
  5. `inheritsFrom` chains are resolved correctly and library `rules` are evaluated per platform/arch; legacy natives are extracted per-instance, LWJGL 3.3.1+ embedded natives require no extraction
**Plans**: TBD
**UI hint**: yes

### Phase 3: Launcher Process and Offline Launch
**Goal**: A vanilla Minecraft instance can be launched from the TUI in offline mode; JVM arguments, classpath, and natives are composed correctly for both Linux and Windows; stdout/stderr is drained asynchronously; any long-running install or download can be cancelled.
**Depends on**: Phase 2
**Requirements**: LAUN-01, LAUN-02, LAUN-03, LAUN-04, LAUN-05, LAUN-06, AUTH-05
**Success Criteria** (what must be TRUE):
  1. User can launch a vanilla Minecraft instance on Linux; the game reaches the title screen
  2. User can launch in offline mode with an arbitrary username without a Microsoft account
  3. Minecraft's stdout/stderr is drained to a per-instance log file asynchronously; the TUI remains responsive while the game runs
  4. When launch fails, the TUI surfaces the log tail with a human-readable error message; no silent failures
  5. User can cancel any active download or install operation; partial state is cleaned up and the TUI reflects the cancelled status
**Plans**: 6 plans
- [x] 03-01-launcher-scaffold-PLAN.md — Cargo md-5 dep + AppError launch variants + src/launcher module skeleton + AppPaths::instance_log_file
- [x] 03-02-pure-command-composition-PLAN.md — SubstitutionContext, classpath builder, offline UUID, Windows @argfile writer, LaunchCommand composer with 1.21.4/1.12.2 fixture snapshots
- [x] 03-03-spawn-and-drain-PLAN.md — async tokio::process spawner with 2 drain tasks, kill_on_drop, CancellationToken, ring-buffer log tail, #[ignore] smoke test
- [x] 03-04-launch-service-PLAN.md — launch_instance orchestrator: manifest load, disk-only inheritsFrom walk, Java probe, Windows @argfile wiring, mark_launch_started + update_play_time
- [x] 03-05-tui-wiring-PLAN.md — Enter launches, s stops, d blocked on running, LaunchFailedModal view, running_instances HashMap<String, CancellationToken>, 7 new tui_smoke tests
- [x] 03-06-integration-and-validation-PLAN.md — pure launch_command.rs snapshot + #[ignore] launch_integration.rs end-to-end + 03-VALIDATION.md populate + human checkpoint to flip nyquist_compliant
**UI hint**: yes

### Phase 4: Microsoft Authentication
**Goal**: Users can authenticate with Microsoft accounts via device-code flow, stay authenticated across sessions via stored refresh tokens, and switch between multiple accounts.
**Depends on**: Phase 1
**Requirements**: AUTH-01, AUTH-02, AUTH-03, AUTH-04, AUTH-06
**Success Criteria** (what must be TRUE):
  1. User can initiate device-code auth; the TUI displays the code, URL, and a countdown timer; completing auth on a browser signs the user in
  2. The full MSA→Xbox Live→XSTS→Minecraft token chain completes; entitlement is verified and a Minecraft profile is fetched
  3. XSTS error codes (e.g., no Xbox profile, child account) produce user-readable messages rather than raw error codes
  4. Refresh tokens are stored in the OS keychain (libsecret / DPAPI) with encrypted-file fallback; tokens survive a launcher restart without re-authenticating
  5. User can add multiple Microsoft accounts and select which account to use when launching an instance
**Plans**: 10 plans
- [x] 04-01-auth-scaffold-PLAN.md — Cargo deps (keyring, aes-gcm, httpmock) + AppError auth variants + `pub mod auth;` module skeleton + Account struct + AppPaths::accounts_file
- [x] 04-02-xsts-errors-PLAN.md — Pure map_xerr function for all 7 documented XErr codes + unknown-code fallback + 11 unit tests
- [x] 04-03-xbox-mc-services-PLAN.md — Hand-rolled Xbox Live + XSTS + Minecraft services reqwest clients (RpsTicket d= prefix, XSTS 401 XErr parsing, identityToken XBL3.0 format, entitlement check) with httpmock tests
- [x] 04-04-device-code-PLAN.md — MSA OAuth device-code flow (RFC 8628) with polling state machine (authorization_pending, slow_down +5s, expired_token, access_denied) + refresh_access_token + CancellationToken support
- [x] 04-05-chain-orchestrator-PLAN.md — run_full_auth + ensure_valid_mc_token composing XBL/XSTS/MC + end-to-end httpmock tests
- [x] 04-06-account-store-PLAN.md — keyring-first + AES-256-GCM encrypted-file fallback (machine-id key, 0600 perms) + accounts.json metadata persistence
- [x] 04-07-account-service-PLAN.md — AccountService facade: start_device_code_auth / list / remove / activate / resolve_auth_context_for_launch / resolve_msa_tokens_for_launch
- [x] 04-08-launcher-integration-PLAN.md — launch_instance signature change (AuthContext enum + Option<AccountService>), compose_msa, MsaAuth adapter, offline path byte-identical
- [x] 04-09-tui-wiring-PLAN.md — AccountsList + AddAccountDeviceCode + AccountAuthFailed views; capital A keybind; 10 new Actions + 4 new Effects; 10 new tui_smoke tests
- [x] 04-10-integration-validation-PLAN.md — tests/auth_chain.rs mocked end-to-end + tests/msa_chain_live.rs live smoke (#[ignore]) + VALIDATION.md populate + human checkpoint
**UI hint**: yes

### Phase 5: Java Runtime Management
**Goal**: The launcher automatically downloads and manages the correct Java runtime for each Minecraft version, detects system-installed Java, and allows per-instance override — so users never have to manually install or locate Java.
**Depends on**: Phase 1
**Requirements**: JAVA-01, JAVA-02, JAVA-03, JAVA-04, JAVA-05
**Success Criteria** (what must be TRUE):
  1. Launcher downloads the Mojang-blessed JRE for the target MC version and uses it automatically; no manual Java setup is required
  2. When no Mojang-blessed runtime is available, launcher falls back to Adoptium and downloads an appropriate JRE
  3. System-installed Java runtimes in PATH and common locations are detected and listed as selectable options per instance
  4. User can override the Java runtime for a specific instance and the selection persists in instance config
  5. Launcher validates the selected Java major version against MC requirements before launch; mismatches produce a clear error, not a silent crash
**Plans**: 9 plans
- [x] 05-01-java-scaffold-PLAN.md — Cargo deps (flate2, tar), AppError Java variants, JavaRuntimeId enum, AppPaths jre_* accessors, InstanceManifest.java_override field, src/java/ module skeleton
- [x] 05-02-pure-mapping-PLAN.md — Pure platform-key mapping (Mojang + Adoptium), parse_java_major, validate_java_major — no I/O
- [x] 05-03-mojang-jre-PLAN.md — MojangJreClient: fetch all.json, select variant, fetch per-variant manifest, stream+SHA1-verify files, honor executable flag + unix symlinks, atomic .tmp rename
- [x] 05-04-adoptium-PLAN.md — AdoptiumClient: fetch latest release for {major,arch,os}, SHA256-verified download, spawn_blocking .tar.gz/.zip extraction with top-level prefix strip
- [x] 05-05-system-detect-PLAN.md — scan_system_javas: PATH iteration + common dirs scan, dedupe by canonical path, java -version stderr parse, 5s timeout
- [x] 05-06-java-service-PLAN.md — JavaService facade: resolve_jre_for_launch 5-step precedence (env → override → Mojang → Adoptium → validate) + install_mojang/install_adoptium/list_system_javas/set_override_for_instance + instance::store::set_java_override
- [x] 05-07-launcher-integration-PLAN.md — Replace resolve_java_bin stub in launcher/service.rs with JavaService::resolve_jre_for_launch; Phase 3 snapshot regression guard preserved
- [ ] 05-08-tui-java-picker-PLAN.md — j keybind on instance list → JavaPickerModal listing Auto + detected system Javas + Manual escape hatch; persist via SetJavaOverride effect (human-verify checkpoint)
- [ ] 05-09-integration-validation-PLAN.md — tests/jre_live.rs (#[ignore] real Mojang download), 05-VALIDATION.md populate (17 rows), human checkpoint for fresh-data_dir launch → nyquist_compliant flip

### Phase 6: Fabric and Quilt Modloaders
**Goal**: Users can install Fabric Loader or Quilt Loader on any instance, select loader versions, switch between loaders, and see installer errors surfaced clearly — validating the modloader install abstraction before tackling Forge.
**Depends on**: Phase 2, Phase 3
**Requirements**: LOAD-01, LOAD-02, LOAD-05, LOAD-06
**Success Criteria** (what must be TRUE):
  1. User can install Fabric Loader on an instance by selecting a loader version from the fetched version list; installation completes without errors on a compatible MC version
  2. User can install Quilt Loader on an instance using the Quilt meta API; the Quilt library list is correctly merged into the instance manifest
  3. User can switch the loader or loader version on an existing instance
  4. When a modloader installation fails, the TUI surfaces captured stdout/stderr from the installer process with a clear error message
**Plans**: 9 plans
- [x] 06-01-loader-scaffold-PLAN.md — lib.rs + loader/ module skeleton + LoaderType/LoaderError/LoaderInfo + maven path-traversal-safe coord parser
- [x] 06-02-domain-loader-field-PLAN.md — InstanceManifest.loader: Option<LoaderInfo> with forward-compat tests
- [x] 06-03-fabric-client-PLAN.md — FabricMetaClient: list/profile fetch + LoaderLibrary with sha1/sha256 + httpmock unit tests
- [x] 06-04-quilt-client-PLAN.md — QuiltMetaClient: v3 API + is_quilt_stable + no-hash library invariant
- [x] 06-05-loader-service-PLAN.md — LoaderService 4-step install pipeline + idempotent re-attach + remove + switch + cancellation
- [x] 06-06-tui-state-PLAN.md — 5 ActiveView + 19 Action + 4 Effect variants + LoaderPickerRow + update arms (pure)
- [ ] 06-07-tui-views-PLAN.md — 5 new view files (picker / version-picker / progress / failed / switch confirm) + view.rs dispatch + views/mod.rs
- [ ] 06-08-tui-wiring-PLAN.md — run.rs LoaderService Arc + 4 effect arms + L keybind + instance_list status cell + 11 tui_smoke tests
- [ ] 06-09-integration-validation-PLAN.md — tests/loader_live.rs (#[ignore] Fabric+Quilt live) + 06-VALIDATION.md fill + 06-HUMAN-UAT.md + nyquist checkpoint
**UI hint**: yes

### Phase 7: Forge and NeoForge Modloaders
**Goal**: Users can install Forge and NeoForge on compatible instances; the JVM processor pipeline runs correctly, processor outputs are SHA1-verified, and subprocess failures are surfaced with captured output.
**Depends on**: Phase 5, Phase 6
**Requirements**: LOAD-03, LOAD-04
**Success Criteria** (what must be TRUE):
  1. User can install Forge on a post-1.12.2 instance; the JVM processor pipeline completes, processor output SHA1s match, and the resulting version JSON is merged into the instance
  2. User can install NeoForge on a compatible instance using NeoForge-specific Maven coordinates and version naming
  3. Forge/NeoForge installer subprocess runs with `-Djava.awt.headless=true`; progress steps ("Running processor 3/7") are visible in the TUI
  4. When the Forge processor pipeline fails (including zlib-ng hash mismatch), the TUI shows captured stderr and an actionable error message
**Plans**: TBD
**UI hint**: yes

### Phase 8: Modrinth Integration
**Goal**: Users can search and browse Modrinth mods, install mods with automatic dependency resolution, and manage (view, enable/disable, uninstall) the mod list per instance.
**Depends on**: Phase 2, Phase 6
**Requirements**: MOD-01, MOD-02, MOD-05, MOD-06, MOD-07
**Success Criteria** (what must be TRUE):
  1. User can search Modrinth mods by name with MC version and loader filters; results render in a split-pane browser showing name, description, and download count
  2. User can install a Modrinth mod into an instance; dependencies are resolved automatically and downloaded in parallel with a semaphore-bounded task pool
  3. User can view all installed mods for an instance with name, version, and source displayed
  4. User can enable or disable an installed mod (toggling between `.jar` and `.jar.disabled`) and uninstall a mod from an instance
**Plans**: TBD
**UI hint**: yes

### Phase 9: CurseForge Integration
**Goal**: Users can search and install mods from CurseForge using an API key that ships as a compiled-in default with user override; restricted downloads surface a clear error with the CurseForge web URL rather than silently failing.
**Depends on**: Phase 2, Phase 6
**Requirements**: MOD-03, MOD-04, MOD-08
**Success Criteria** (what must be TRUE):
  1. User can search CurseForge mods by name with MC version and loader filters; results render consistently with the Modrinth browser UI
  2. User can install a CurseForge mod into an instance; the download completes and the JAR is placed in the instance mods directory
  3. When a mod's `downloadUrl` is null (author-restricted), the TUI displays a user-friendly error with the CurseForge web URL — no panic or silent skip
  4. The CurseForge API key ships as a compiled-in default; users can override it via `CURSEFORGE_API_KEY` env var or `config.toml`
**Plans**: TBD
**UI hint**: yes

### Phase 10: Modpack Import
**Goal**: Users can import Modrinth `.mrpack` files and CurseForge modpack zips as new instances; imports can be cancelled cleanly mid-stream without leaving a half-installed instance.
**Depends on**: Phase 6, Phase 7, Phase 8, Phase 9
**Requirements**: PACK-01, PACK-02, PACK-03, PACK-04, PACK-05, PACK-06
**Success Criteria** (what must be TRUE):
  1. User can import a Modrinth `.mrpack` file by providing a file path; a new instance is created with the correct MC version and modloader, all mods are downloaded and SHA-512 verified
  2. `.mrpack` import respects `env.client` filter (skipping unsupported files) and applies `client-overrides/` on top of `overrides/` correctly
  3. User can import a CurseForge modpack zip; mods are resolved by project/file ID via the CurseForge API and placed in the instance
  4. CurseForge modpack import reports per-file download failures with actionable messages rather than aborting silently
  5. User can cancel a modpack import mid-stream; no partially-installed instance is left on disk
**Plans**: TBD
**UI hint**: yes

### Phase 11: Resource Packs and Shader Packs
**Goal**: Users can add resource packs and shader packs to instances via file path or Modrinth browse, and manage (view, enable, remove) what is installed.
**Depends on**: Phase 2, Phase 8
**Requirements**: RPAK-01, RPAK-02, RPAK-03, SPAK-01, SPAK-02, SPAK-03
**Success Criteria** (what must be TRUE):
  1. User can add a resource pack to an instance by providing a file path or selecting a zip; the pack appears in the instance's resourcepacks directory
  2. User can browse and install resource packs from Modrinth filtered by MC version; installed packs are listed with enable/disable and remove actions
  3. User can add a shader pack by file path or path input; the pack is placed in the instance's shaderpacks directory
  4. User can browse and install shader packs from Modrinth; installed shader packs are listed with a remove action
**Plans**: TBD
**UI hint**: yes

### Phase 12: Windows Polish and Distribution
**Goal**: The launcher runs correctly on unmodified Windows (MAX_PATH handled, longPathAware binary manifest present); prebuilt binaries for Linux x86_64 and Windows x86_64 are published on each GitHub release; `cargo install mineltui` works.
**Depends on**: Phase 3, Phase 11
**Requirements**: PLAT-04, DIST-01, DIST-02, DIST-03
**Success Criteria** (what must be TRUE):
  1. A Forge-modded instance installs and launches correctly on an unmodified Windows 10/11 machine with a deeply nested `%APPDATA%` path
  2. The launcher binary's Windows manifest includes `longPathAware`; classpath is passed via `@argfile` on Windows
  3. Each GitHub release publishes downloadable Linux x86_64 and Windows x86_64 binaries built by the CI pipeline
  4. `cargo install mineltui` succeeds from crates.io (or the repo); the installed binary runs and displays the TUI
**Plans**: TBD
**UI hint**: yes

## Progress

**Execution Order:**
Phases execute in numeric order. Phases 3 and 4 have no interdependency after Phase 1 and may be worked in parallel. Phase 5 must complete before Phase 7 begins.

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Project Scaffold and Core Infrastructure | 6/6 | Complete   | 2026-04-20 |
| 2. Mojang Protocol and Instance Management | 8/8 | Complete   | 2026-04-20 |
| 3. Launcher Process and Offline Launch | 6/6 | Complete   | 2026-04-21 |
| 4. Microsoft Authentication | 10/10 | Complete   | 2026-04-22 |
| 5. Java Runtime Management | 7/9 | In Progress|  |
| 6. Fabric and Quilt Modloaders | 6/9 | In Progress|  |
| 7. Forge and NeoForge Modloaders | 0/? | Not started | - |
| 8. Modrinth Integration | 0/? | Not started | - |
| 9. CurseForge Integration | 0/? | Not started | - |
| 10. Modpack Import | 0/? | Not started | - |
| 11. Resource Packs and Shader Packs | 0/? | Not started | - |
| 12. Windows Polish and Distribution | 0/? | Not started | - |
