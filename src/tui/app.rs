//! Elm-style app state for Phase 2 + Phase 3 + Phase 4 + Phase 5.
//!
//! Phase 1 variants (Quit, Task, Tick) are preserved. Phase 2 adds view
//! navigation (OpenCreateModal, CloseModal, ConfirmDelete, ...), instance
//! lifecycle (SelectVersion, RenameInstance, CloneInstance), and background
//! completions (ManifestLoaded, InstancesLoaded, VersionInstalled, ...).
//! Phase 3 adds the launch lifecycle (LaunchInstance, LaunchJobStarted,
//! InstanceLaunched, InstanceExited, LaunchFailed, StopInstance), the
//! running_instances map (slug to CancellationToken), and LaunchFailedModal.
//! Phase 4 adds Microsoft account management (AccountsList, AddAccountDeviceCode,
//! AccountAuthFailed views) and the MSA launch integration via AuthContext.
//! Phase 5 adds the Java picker modal (JavaPickerModal view, OpenJavaPicker,
//! JavaPickerOptionsLoaded, etc.) for per-instance java_override management.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::auth::{Account, AuthContext};
use crate::config::Config;
use crate::domain::platform::{Arch, OsName};
use crate::domain::InstanceManifest;
use crate::java::detect::SystemJava;
use crate::java::types::JavaRuntimeId;
use crate::loader::types::{LoaderInfo, LoaderType, LoaderVersionEntry};
use crate::mods::dep_resolve::ResolvedDepGraph;
use crate::mods::types::{
    DepKind, InstalledModRow, ModBrowserFetchState, ModrinthProjectDetail, ModrinthSearchHit,
    ModrinthVersion, ModrinthVersionEntry, ResolvedDep,
};
use crate::mojang::types::VersionEntry;
use crate::packs::kind::PackKind;
use crate::tasks::{JobId, TaskEvent};

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum VersionFilter {
    #[default]
    Releases,
    All,
}

#[derive(Debug, Clone)]
pub enum CreateStep {
    NameInput {
        current: String,
        error: Option<String>,
    },
    VersionPicker {
        name: String,
        filter: VersionFilter,
        search: String,
        error: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub enum ActiveView {
    InstanceList {
        selected: usize,
    },
    CreateModal(CreateStep),
    DeleteConfirm {
        slug: String,
        display_name: String,
    },
    RenameInline {
        slug: String,
        current: String,
        original: String,
    },
    /// Inline group editor (INST-06 -- mirrors RenameInline).
    GroupInline {
        slug: String,
        buffer: String,
        original: Option<String>,
    },
    /// Launch-failed modal -- shown when Action::LaunchFailed is dispatched.
    /// Esc dismisses and returns to InstanceList.
    LaunchFailedModal {
        slug: String,
        error: String,
        log_tail: String,
    },
    /// AUTH-06 account list view. Entered via `A` from InstanceList.
    AccountsList {
        selected: usize,
    },
    /// AUTH-01 device-code modal. `expires_at` drives the countdown;
    /// the render loop recomputes "seconds remaining" each frame.
    AddAccountDeviceCode {
        user_code: String,
        verification_uri: String,
        expires_at: Instant,
        stage: String,
    },
    /// AUTH-02 error modal (e.g., "No Xbox profile -- visit xbox.com/profile").
    AccountAuthFailed {
        reason: String,
    },
    /// Java picker modal. `options` is populated after FetchSystemJavas completes.
    /// `selected` is the highlighted row index (0 = Auto).
    JavaPickerModal {
        slug: String,
        options: Vec<JavaPickerRow>,
        selected: usize,
    },
    /// Phase 6: choose between None / Fabric / Quilt for an instance.
    LoaderPickerModal {
        slug: String,
        selected: usize,
    },
    /// Phase 6: choose a specific loader version after the user picks Fabric or Quilt.
    LoaderVersionPickerModal {
        slug: String,
        loader: LoaderType,
        versions: Vec<LoaderVersionEntry>,
        filter_stable_only: bool,
        search: String,
        selected: usize,
        current_version: Option<String>,
    },
    /// Phase 6: live progress modal during install.
    LoaderInstallProgressModal {
        slug: String,
        loader: LoaderType,
        version: String,
        step_label: String,
        step_index: usize,
        step_total: usize,
        bytes_done: u64,
        bytes_total: u64,
        cancel_token_key: String,
        /// Phase 7: live log tail from installer subprocess output (D-02).
        log_tail: String,
    },
    /// Phase 6: error modal when install fails (mirrors LaunchFailedModal).
    LoaderInstallFailedModal {
        slug: String,
        loader: LoaderType,
        version: String,
        error: String,
        log_tail: String,
    },
    /// Phase 6: inline confirm overlay when switching loader (mirrors DeleteConfirm).
    LoaderSwitchConfirm {
        slug: String,
        from_loader: Option<String>,
        to_loader: String,
        type_switch: bool,
    },
    /// Phase 8 (MOD-01): full-screen split-pane Modrinth mod browser.
    /// Rendered by `src/tui/views/mod_browser.rs` (added in 08-08).
    ModBrowser {
        slug: String,
        search: String,
        /// Vim-style explicit search mode. `false` = browse: arrows / `j`/`k`
        /// nav, `v`/`l` toggle filter chips, Enter opens versions, `/` enters
        /// search, Esc closes browser. `true` = search: every printable char
        /// types into `search`, Backspace pops, Esc returns to browse mode.
        /// Default `false` so users can press filter shortcuts immediately
        /// after opening the browser. Closes the bug where typing a query
        /// starting with `v`/`l`/`k`/`j` was impossible because those keys
        /// were unconditionally consumed as commands while `search` was empty.
        is_searching: bool,
        /// None = use instance's MC version; Some("any") = no MC filter; Some(version) = explicit override.
        mc_filter_override: Option<String>,
        /// Same shape as mc_filter_override. Default: None (use instance loader).
        loader_filter_override: Option<String>,
        /// Cached search results from the most recent Modrinth query.
        results: Vec<ModrinthSearchHit>,
        /// Index into `results` of the currently highlighted row.
        selected: usize,
        /// "loading" / "ready" / "error" state.
        fetch_state: ModBrowserFetchState,
        /// Cached project detail for the right-pane preview. None while in flight.
        selected_detail: Option<ModrinthProjectDetail>,
    },
    /// Phase 8 (MOD-01): centered modal of available versions for a mod.
    /// Rendered by `src/tui/views/mod_version_picker_modal.rs` (added in 08-08).
    ModVersionPickerModal {
        slug: String,
        project_id: String,
        project_title: String,
        versions: Vec<ModrinthVersionEntry>,
        selected: usize,
    },
    /// Phase 8 (MOD-02): centered modal listing required/optional/incompatible deps.
    /// Rendered by `src/tui/views/dep_confirm_modal.rs` (added in 08-08).
    DepConfirmModal {
        slug: String,
        project_id: String,
        project_title: String,
        version_id: String,
        version_label: String,
        /// Resolved dependencies (required + optional, with already-installed marked).
        deps: Vec<ResolvedDep>,
        /// Total bytes across all NEW downloads (already-satisfied deps excluded).
        total_bytes: u64,
        /// Total file count across all NEW downloads.
        total_files: usize,
        /// True if any dep is `incompatible`. Disables the `y` confirm path.
        has_conflict: bool,
        /// Carry the root_version so `ConfirmModInstall` can pass it to
        /// `Effect::InstallModWithDeps` without re-fetching.
        root_version: Box<ModrinthVersion>,
    },
    /// Phase 8 (MOD-05): full-screen single-column installed-mods table.
    /// Rendered by `src/tui/views/installed_mods_list.rs` (added in 08-08).
    InstalledModsList {
        slug: String,
        mods: Vec<InstalledModRow>,
        selected: usize,
    },
    /// Phase 8 (MOD-07): inline overlay confirm for uninstalling a mod.
    /// Rendered by `src/tui/views/uninstall_mod_confirm.rs` (added in 08-08).
    UninstallModConfirm {
        slug: String,
        /// Modrinth project_id or filesystem hash for manual mods.
        mod_id: String,
        display_name: String,
    },
    /// Phase 8 (MOD-02): error modal when a mod install fails.
    /// Rendered by `src/tui/views/mod_install_failed_modal.rs` (added in 08-08).
    ModInstallFailedModal {
        slug: String,
        mod_title: String,
        version_label: String,
        error: String,
        log_tail: String,
        /// Where to return after dismissal -- depends on which view triggered the install.
        return_to: ModInstallFailedReturnTo,
    },

    // ── Phase 9 (CurseForge Integration) -- see 09-RESEARCH.md §TUI Integration Plumbing ──
    /// Phase 9 (MOD-03): full-screen split-pane CurseForge mod browser.
    /// Mirrors Phase 8's `ModBrowser` shape but binds CurseForge wire types.
    /// Rendered by `src/tui/views/cf_browser.rs` (added in 09-07).
    CfBrowser {
        slug: String,
        search_input: String,
        results: Vec<crate::mods::curseforge::types::CurseForgeSearchHit>,
        selected: usize,
        /// Reused from Phase 8 -- Loading / Ready / Error(String).
        fetch_state: crate::mods::types::ModBrowserFetchState,
        /// None = use instance MC; Some("any") = no MC filter; Some(version) = explicit.
        mc_filter: Option<String>,
        /// CurseForge loader filter is an integer modLoaderType (1=Forge, 4=Fabric, 5=Quilt, 6=NeoForge).
        /// None = use instance loader.
        loader_filter: Option<i32>,
        /// Cached project detail for the right-pane preview. None while in flight.
        selected_detail: Option<crate::mods::curseforge::types::CurseForgeProjectDetail>,
    },
    /// Phase 9 (MOD-03): centered modal of available files for a CurseForge mod.
    /// Mirrors Phase 8's `ModVersionPickerModal`. Rendered by
    /// `src/tui/views/cf_file_picker_modal.rs` (added in 09-07).
    CfFilePickerModal {
        slug: String,
        mod_detail: crate::mods::curseforge::types::CurseForgeProjectDetail,
        files: Vec<crate::mods::curseforge::types::CurseForgeFileEntry>,
        selected: usize,
    },
    /// Phase 9 (MOD-04): error modal shown when an install fails -- most importantly
    /// the FileNotDownloadable case where `web_url` is `Some(url)` so the modal
    /// can render an "Open in browser:" line. Per 09-RESEARCH.md §"downloadUrl
    /// null UX" lines 252-289.
    CfInstallFailedModal {
        slug: String,
        mod_title: String,
        file_label: String,
        error_message: String,
        /// Some(url) iff the failure is FileNotDownloadable -- render the link.
        web_url: Option<String>,
    },

    // ── Phase 10 (Modpack Import) -- see 10-PATTERNS.md ──
    /// Phase 10 (PACK-01): centered text-entry modal for path to .mrpack.
    /// Rendered by `src/tui/views/modpack_import_path_modal.rs`.
    ModpackImportPathInput {
        buffer: String,
        error: Option<String>,
    },
    /// Phase 10 (PACK-01/02/03/06): live progress modal during import.
    /// Mirrors `LoaderInstallProgressModal` shape; see PATTERNS.md §9.
    /// Rendered by `src/tui/views/modpack_import_progress_modal.rs`.
    ModpackImportProgressModal {
        modpack_name: String,
        step_label: String,
        step_index: usize,
        step_total: usize,
        bytes_done: u64,
        bytes_total: u64,
        cancel_token_key: String,
        log_tail: String,
    },
    /// Phase 10: error modal when import fails. Esc dismisses.
    /// Mirrors `LoaderInstallFailedModal`; see PATTERNS.md §10.
    ModpackImportFailedModal {
        modpack_name: String,
        error: String,
        log_tail: String,
    },

    // ── Phase 11 (11-04): Resource + Shader pack views ───────────────────────
    /// Full-screen Modrinth pack browser parameterised by kind.
    PackBrowser {
        slug: String,
        kind: PackKind,
        search: String,
        /// Vim-style explicit search mode (mirrors ModBrowser.is_searching).
        is_searching: bool,
        fetch_state: ModBrowserFetchState,
        results: Vec<ModrinthSearchHit>,
        selected: usize,
    },
    /// Full-screen installed-packs list (Resource or Shader).
    /// The `m` keybind opens `InstalledModsList` (Mod kind); Tab cycles to this view.
    InstalledPacksList {
        slug: String,
        kind: PackKind,
        packs: Vec<InstalledModRow>,
        selected: usize,
        /// Transient single-line status shown below the list.
        /// Set by ShaderToggleNotice; cleared on next move / Tab / key.
        transient_status: Option<String>,
    },
    /// Text-entry modal for drop-from-path install.
    PackDropPathInput {
        slug: String,
        kind: PackKind,
        buffer: String,
        error: Option<String>,
    },
    /// y/N confirm dialog for removing a pack (kind-aware title).
    UninstallPackConfirm {
        slug: String,
        kind: PackKind,
        mod_id: String,
        file_name: String,
    },
    /// Phase 11 follow-up: error modal when a pack install fails.
    /// Mirrors `ModInstallFailedModal` so the failure surfaces in UI rather
    /// than only the log file (previously `PackInstallFailed` only emitted
    /// `tracing::warn`).
    PackInstallFailedModal {
        slug: String,
        kind: PackKind,
        pack_title: String,
        version_label: String,
        error: String,
        return_to: PackInstallFailedReturnTo,
    },
}

/// Where to return after a `PackInstallFailedModal` is dismissed. Mirrors
/// `ModInstallFailedReturnTo` so the back-navigation respects which view
/// the user kicked the install off from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackInstallFailedReturnTo {
    PackBrowser,
    InstalledPacksList,
}

/// Where the `Action::DismissModInstallFailed` arm should return to.
/// Set when the failure modal is opened, based on the previous active view
/// (per UI-SPEC line 626: "returns to whichever view triggered the install").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModInstallFailedReturnTo {
    ModBrowser,
    InstalledModsList,
}

/// A row in the Java picker modal.
#[derive(Debug, Clone)]
pub enum JavaPickerRow {
    /// Use the auto-resolve logic (clears java_override).
    Auto,
    /// A working system Java detected on the host.
    Detected(SystemJava),
    /// Escape hatch: user edits instance.json manually.
    Manual,
}

/// A row in the loader picker modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderPickerRow {
    /// No loader (vanilla -- clears installed loader if any).
    None,
    /// Open the Fabric version picker.
    Fabric,
    /// Open the Quilt version picker.
    Quilt,
}

impl Default for ActiveView {
    fn default() -> Self {
        ActiveView::InstanceList { selected: 0 }
    }
}

#[derive(Default)]
pub struct AppState {
    pub should_quit: bool,
    pub active_view: ActiveView,
    pub instances: Vec<InstanceManifest>,
    pub versions: Vec<VersionEntry>,
    pub versions_filter: VersionFilter,
    pub active_jobs: Vec<(JobId, u8, String)>,
    pub arch: Option<Arch>,
    pub os: Option<OsName>,
    pub last_error: Option<String>,
    /// Tracks slug → CancellationToken for every currently-running launch job.
    /// Populated by Action::LaunchJobStarted; cleared by InstanceExited / LaunchFailed.
    pub running_instances: HashMap<String, CancellationToken>,
    /// Tracks slug → CancellationToken for in-progress loader installs.
    /// Populated by Action::LoaderInstallStarted; cleared by
    /// LoaderInstalled / LoaderInstallFailed (mirrors `running_instances`).
    pub running_loader_installs: HashMap<String, CancellationToken>,
    /// Tracks slug → CancellationToken for in-progress mod installs.
    /// Pitfall 8 (08-RESEARCH.md §Pitfall 8): a second `M` keybind on the same
    /// instance is silently rejected while a previous mod install is in flight.
    /// Populated by Action::ModInstallStarted; cleared by ModInstalled /
    /// ModInstallFailed (mirrors `running_loader_installs`).
    pub running_mod_jobs: HashMap<String, CancellationToken>,
    /// Persisted list of Microsoft accounts.
    pub accounts: Vec<Account>,
    /// id of the currently-active account, if any. Drives whether
    /// Effect::LaunchInstance builds AuthContext::Msa or Offline.
    pub active_account_id: Option<String>,
    /// Cancellation token for the currently-active
    /// start_device_code_auth job. Cleared on Completion/Failure/Cancel.
    pub add_account_cancel: Option<CancellationToken>,
    /// Phase 9 (MOD-03): true iff a CurseForge API key was resolved at startup.
    /// Set by 09-07 in `run.rs` from `cf_service.api_key_present()`. Read by
    /// the `OpenCfBrowser` arm to silently no-op the F keybind when no key is
    /// configured (Pitfall 1 surface -- 09-RESEARCH.md §"Keybind guard").
    pub cf_api_key_present: bool,
    /// Phase 10: tracks slug → CancellationToken for an in-progress modpack import.
    /// Single-entry in v1 (only one modpack import runs at a time); HashMap shape
    /// retained for future-compat with parallel imports.
    /// Populated by Action::ModpackImportStarted; cleared by ModpackImported /
    /// ModpackImportCancelled / ModpackImportFailed.
    pub running_modpack_imports: HashMap<String, CancellationToken>,
    /// Phase 11: tracks (slug, kind) → CancellationToken for in-progress pack installs.
    /// Populated by Effect::DropInstallPack / InstallPackFromModrinth;
    /// cleared by Action::PackInstalled / PackInstallFailed / PackDropFailed.
    pub running_pack_jobs: HashMap<(String, PackKind), CancellationToken>,
    /// Per-instance set of installed Modrinth project_ids (mods).
    /// Source of truth for stamping `already_installed` on ModBrowser results
    /// independent of the search/install timing race. Refreshed from the ledger
    /// via `InstalledModsLoaded`; updated optimistically on
    /// `ModInstalled`/`ModUninstalled`.
    pub installed_mod_project_ids: HashMap<String, HashSet<String>>,
    /// Per-(instance, kind) set of installed Modrinth project_ids (packs).
    /// Mirrors `installed_mod_project_ids` for the pack browsers.
    pub installed_pack_project_ids: HashMap<(String, PackKind), HashSet<String>>,
    /// User-facing configuration: rebindable keys + color palette.
    /// Loaded once at startup from `~/.config/ichr/config.toml` (or
    /// platform equivalent); shared via `Arc` so per-frame render fns
    /// can clone the handle cheaply. Defaults match the historical
    /// hardcoded UX so introducing this field is a no-op until
    /// renderers / handlers migrate to read from it.
    pub config: Arc<Config>,
    /// Phase 13: image-protocol picker, populated at startup if the
    /// terminal supports kitty / sixel / iterm2. Owned here so the
    /// (incoming) IconService can borrow it for `new_protocol(...)`.
    /// `None` on halfblocks-only terminals or when detection failed.
    pub image_picker: Option<ratatui_image::picker::Picker>,
    /// Phase 13: derived flag -- true iff `image_picker` is Some AND the
    /// detected protocol is something better than halfblocks. Detail-pane
    /// renderers consult this before carving an icon Rect; on false they
    /// keep the existing text-only layout (no flicker, no empty box).
    pub icon_rendering_enabled: bool,
    /// Phase 13: shared icon service (HTTP fetch + LRU). Set when
    /// `icon_rendering_enabled` is true. `Arc` so spawned `Effect::FetchIcon`
    /// tasks can share the same LRU view.
    pub icon_service: Option<std::sync::Arc<crate::icons::IconService>>,
}

#[derive(Debug, Clone)]
pub enum Action {
    // Phase 1
    Quit,
    Task(TaskEvent),
    Tick,

    // Navigation
    OpenCreateModal,
    CloseModal,
    OpenDeleteConfirm {
        slug: String,
        display_name: String,
    },
    OpenRenameInline {
        slug: String,
        current: String,
    },
    MoveSelection(isize),

    // Create flow
    TypeName(char),
    /// Pasted text into the CreateModal name-input field -- applies on
    /// terminals that emit `Event::Paste(String)` (bracketed-paste mode,
    /// enabled at terminal init in 08.1-04). Buffer-mutation only; no
    /// downstream effect (mirrors `TypeName` behaviour).
    PasteName(String),
    BackspaceName,
    SubmitInstanceName(String),
    SetVersionFilter(VersionFilter),
    TypeSearch(char),
    BackspaceSearch,
    ClearSearch,
    SelectVersion(String),

    // Rename / clone / delete
    TypeRename(char),
    BackspaceRename,
    SubmitRename(String),
    CloneSelected,
    ConfirmDelete,
    CancelDelete,

    // Group editor (INST-06 -- mirrors rename pattern)
    OpenGroupInput {
        slug: String,
        current: String,
    },
    TypeGroup(char),
    BackspaceGroup,
    SubmitGroup,
    CancelGroupInput,

    // Phase 4: accounts
    OpenAccounts,
    CloseAccounts,
    AccountsLoaded(Vec<Account>),
    AddAccount,
    AccountAuthStarted {
        user_code: String,
        verification_uri: String,
        expires_at: Instant,
    },
    AccountAuthProgress {
        stage: String,
    },
    AccountAdded {
        account: Account,
    },
    AccountAuthFailed {
        reason: String,
    },
    RemoveAccount {
        id: String,
    },
    ActivateAccount {
        id: String,
    },
    CancelAddAccount,
    /// Internal -- stores the CancellationToken created by execute_effects for
    /// the device-code auth task into state.add_account_cancel.
    AddAccountTokenCreated(CancellationToken),

    // Phase 5: Java picker
    /// Open the Java picker for a specific instance (dispatched by `j` keybind).
    OpenJavaPicker {
        slug: String,
    },
    /// Async result: detected system Javas are ready to populate the modal.
    JavaPickerOptionsLoaded {
        slug: String,
        options: Vec<JavaPickerRow>,
    },
    /// Move the picker selection up/down (wrapping).
    JavaPickerMove(isize),
    /// Confirm the highlighted picker row.
    JavaPickerSelect,
    /// Dismiss the picker without mutating anything.
    JavaPickerCancel,
    /// Effect completed: java_override was written successfully.
    JavaOverrideSet {
        slug: String,
    },
    /// Effect failed: java_override write returned an error.
    JavaOverrideFailed {
        slug: String,
        reason: String,
    },

    // Phase 6: Loader picker
    /// Open the loader type picker for a specific instance (dispatched by `L` keybind).
    OpenLoaderPicker {
        slug: String,
    },
    /// Move the loader picker selection up/down (wrapping over 3 rows: None/Fabric/Quilt).
    LoaderPickerMove(isize),
    /// Confirm the highlighted loader picker row.
    LoaderPickerSelect,
    /// Dismiss the loader picker without mutating anything.
    LoaderPickerCancel,
    /// Async result: loader versions list fetched and ready to display.
    LoaderVersionsLoaded {
        slug: String,
        loader: LoaderType,
        versions: Vec<LoaderVersionEntry>,
    },
    /// Move the loader version picker selection up/down (wrapping over filtered list).
    LoaderVersionPickerMove(isize),
    /// Toggle stable-only filter in the loader version picker.
    ToggleStableFilter,
    /// Type a character into the version search box.
    LoaderVersionTypeSearch(char),
    /// Delete the last character from the version search box.
    LoaderVersionBackspaceSearch,
    /// Confirm the highlighted loader version (install or switch).
    LoaderVersionSelect,
    /// Dismiss the loader version picker and return to loader picker.
    LoaderVersionPickerCancel,
    /// Internal -- emitted by execute_effects inside the spawned task body BEFORE
    /// calling install_loader. Inserts the CancellationToken into running_loader_installs.
    LoaderInstallStarted {
        slug: String,
        token: CancellationToken,
    },
    /// Progress update from the install task -- updates the progress modal fields.
    LoaderInstallProgress {
        slug: String,
        pct: u8,
        step_label: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    /// Phase 7 (D-02): live log-tail line from the installer subprocess -- updates
    /// `LoaderInstallProgressModal.log_tail` without touching the gauge percentage.
    LoaderInstallLogTail {
        slug: String,
        tail: String,
    },
    /// Install completed successfully -- clears running token, refreshes instances.
    LoaderInstalled {
        slug: String,
    },
    /// Install failed -- clears running token, transitions to failed modal.
    LoaderInstallFailed {
        slug: String,
        loader: LoaderType,
        version: String,
        error: String,
        log_tail: String,
    },
    /// Cancel a running loader install (Esc on progress modal).
    CancelLoaderInstall {
        slug: String,
    },
    /// Dismiss the loader install failed modal (Esc).
    DismissLoaderInstallFailed,
    /// Confirm the loader switch (y/Y in switch confirm modal).
    ConfirmLoaderSwitch,
    /// Cancel the loader switch (n/Esc in switch confirm modal).
    CancelLoaderSwitch,

    // Phase 3: launch lifecycle
    /// Dispatch when Enter is pressed on a non-running instance row.
    LaunchInstance {
        slug: String,
    },
    /// Internal -- emitted by execute_effects inside the spawned task body BEFORE
    /// calling launch_instance. Inserts the CancellationToken into running_instances.
    LaunchJobStarted {
        slug: String,
        token: CancellationToken,
    },
    /// Tracing signal: launch_instance has started executing (after token is stored).
    InstanceLaunched {
        slug: String,
    },
    /// Emitted when the game process exits (cleanly or via cancellation).
    InstanceExited {
        slug: String,
        duration_ms: u64,
    },
    /// Emitted when launch_instance returns a non-cancellation error.
    LaunchFailed {
        slug: String,
        error: String,
        log_tail: String,
    },
    /// Dispatch when `s` is pressed on a running instance row.
    StopInstance {
        slug: String,
    },

    // Background completions
    ManifestLoaded(Vec<VersionEntry>),
    InstancesLoaded(Vec<InstanceManifest>),
    VersionInstalled {
        slug: String,
    },
    VersionInstallFailed {
        slug: String,
        error: String,
    },
    InstanceDeleted(String),
    InstanceRenamed {
        slug: String,
        new_name: String,
    },
    InstanceCloned {
        source_slug: String,
        new_slug: String,
    },
    ServiceErrored(String),

    // ── Phase 8 (Modrinth Integration) -- see UI-SPEC §Keybind Contract ──
    // Mod browser
    /// `M` keybind on InstanceList -- opens the Modrinth mod browser for the
    /// given instance. Pitfall 8 guard: silent no-op if a previous mod install
    /// for this slug is still in flight (`state.running_mod_jobs.contains_key`).
    OpenModBrowser {
        slug: String,
    },
    /// Async result: search results loaded for the open ModBrowser.
    ModBrowserSearchLoaded {
        slug: String,
        hits: Vec<ModrinthSearchHit>,
    },
    /// Async result: Modrinth search failed -- drives `fetch_state = Error(message)`.
    /// Mirrors `Action::CfBrowserSearchFailed`. Closes GAP-8-F (Phase 8.1 gap closure):
    /// previously the network-error path dispatched `ModBrowserSearchLoaded { hits: [] }`
    /// which the view rendered as "No mods found", conflating reachability with empty results.
    ModBrowserSearchFailed {
        slug: String,
        message: String,
    },
    /// Move the highlighted row in the ModBrowser results list (saturating).
    ModBrowserMove(isize),
    /// Enter on a ModBrowser row -- opens the version picker for the selected mod.
    ModBrowserOpenVersions,
    /// `v` in ModBrowser -- cycles MC filter (None ↔ Some("any")) and re-emits search.
    ToggleModMcFilter,
    /// `l` in ModBrowser -- cycles loader filter (None ↔ Some("any")) and re-emits search.
    ToggleModLoaderFilter,
    /// Backspace in ModBrowser search input -- pops last char, re-emits search if empty.
    ModBrowserBackspaceSearch,
    /// `/` in ModBrowser browse mode -- enters vim-style search mode so the
    /// next printable character types into the search bar instead of being
    /// consumed as a filter / nav shortcut.
    ModBrowserBeginSearch,
    /// Esc in ModBrowser search mode -- exits search mode (clears `search`
    /// and returns to browse mode without closing the browser).
    ModBrowserExitSearch,
    /// Esc in ModBrowser -- returns to InstanceList.
    ModBrowserCancel,
    /// Printable char into ModBrowser search input. j/k disambiguation lives in the
    /// keymap (08-08): when search is empty, j/k navigate; otherwise they type.
    ModBrowserTypeSearch(char),
    /// Pasted text into ModBrowser search input -- applies on terminals that
    /// emit `Event::Paste(String)` (bracketed-paste mode, enabled at terminal
    /// init in 08.1-04). Mirror of `ModBrowserTypeSearch(char)` but appends
    /// the full pasted string and re-emits the search effect once.
    ModBrowserPasteSearch(String),
    /// Async result: project detail (right-pane preview) loaded.
    ModDetailLoaded {
        slug: String,
        detail: ModrinthProjectDetail,
    },

    // Version picker
    /// Async result: per-project version list loaded for the open version picker.
    ModVersionsLoaded {
        slug: String,
        versions: Vec<ModrinthVersionEntry>,
    },
    /// Move the highlighted version row (saturating).
    ModVersionPickerMove(isize),
    /// Enter on a version row -- fires `Effect::ResolveModDependencies`.
    ModVersionPickerSelect,
    /// Esc on the version picker -- returns to ModBrowser (preserves user's place).
    ModVersionPickerCancel,

    // Dep-confirm modal
    /// Async result: BFS dep resolution finished -- opens DepConfirmModal.
    ModDepsResolved {
        slug: String,
        project_id: String,
        project_title: String,
        version_id: String,
        version_label: String,
        graph: Box<ResolvedDepGraph>,
    },
    /// `y`/`Y` on DepConfirmModal -- fires `Effect::InstallModWithDeps` IFF
    /// `has_conflict == false`.
    ConfirmModInstall,
    /// `n`/`Esc` on DepConfirmModal -- returns to ModVersionPickerModal per
    /// UI-SPEC line 597 (preserves user's place).
    CancelModInstall,

    // Install lifecycle
    /// Internal -- emitted by execute_effects inside the spawned task body BEFORE
    /// the install begins. Inserts the CancellationToken into running_mod_jobs.
    /// Mirrors `LoaderInstallStarted`.
    ModInstallStarted {
        slug: String,
        project_id: String,
        token: CancellationToken,
    },
    /// Install completed successfully -- clears running_mod_jobs row AND walks the
    /// open ModBrowser results (if any) to stamp `already_installed = true` for
    /// the matching `project_id` (Pitfall 10 fix).
    ModInstalled {
        slug: String,
        project_id: String,
    },
    /// Install failed -- clears running_mod_jobs row, transitions to the failed modal.
    ModInstallFailed {
        slug: String,
        mod_title: String,
        version_label: String,
        error: String,
        log_tail: String,
    },
    /// Esc on ModInstallFailedModal -- returns to whichever view triggered the install
    /// (ModBrowser → ModBrowser, anything else → InstalledModsList).
    DismissModInstallFailed,

    // Installed mods list
    /// `m` keybind on InstanceList -- opens the per-instance Installed Mods List.
    /// Also emits `Effect::FetchInstalledMods` to populate the rows.
    OpenInstalledMods {
        slug: String,
    },
    /// Async result: installed-mods ledger rows loaded for the open list.
    InstalledModsLoaded {
        slug: String,
        mods: Vec<InstalledModRow>,
    },
    /// Move the highlighted row in the InstalledModsList (saturating).
    InstalledModsMove(isize),
    /// `e` keybind on InstalledModsList -- fires `Effect::ToggleModEnabledEff`
    /// for the highlighted row (renames `.jar` ↔ `.jar.disabled`).
    ToggleModEnabled,
    /// Async result: toggle finished -- flip the `enabled` field on the matching row.
    ModToggled {
        slug: String,
        mod_id: String,
        enabled: bool,
    },
    /// `x` keybind on InstalledModsList -- opens the uninstall confirm overlay.
    OpenUninstallModConfirm,
    /// `y`/`Y` on UninstallModConfirm -- fires `Effect::UninstallMod` and returns
    /// to InstalledModsList immediately (responsive UX; row removed by ModUninstalled).
    ConfirmUninstallMod,
    /// `n`/`Esc` on UninstallModConfirm -- returns to InstalledModsList.
    CancelUninstallMod,
    /// Async result: uninstall finished -- remove the matching row from the list.
    ModUninstalled {
        slug: String,
        mod_id: String,
    },
    /// Esc on InstalledModsList -- returns to InstanceList.
    CloseInstalledMods,

    // ── Phase 9 (CurseForge Integration) -- see 09-RESEARCH.md §TUI Integration Plumbing ──
    /// `F` keybind on InstanceList -- opens the CurseForge mod browser for the
    /// given instance. Pitfall 1 guard: silent no-op when `cf_api_key_present == false`.
    /// Pitfall 8 guard: silent no-op when `running_mod_jobs.contains_key(&slug)`.
    OpenCfBrowser {
        slug: String,
    },
    /// Spawn a CurseForge search effect for the open CfBrowser.
    CfBrowserSearchStart {
        slug: String,
        query: String,
        mc: Option<String>,
        loader: Option<i32>,
    },
    /// Async result: search results loaded for the open CfBrowser.
    CfBrowserSearchLoaded {
        slug: String,
        hits: Vec<crate::mods::curseforge::types::CurseForgeSearchHit>,
    },
    /// Async result: search failed -- drives `fetch_state = Error(_)`.
    CfBrowserSearchFailed {
        slug: String,
        error: String,
    },
    /// Move the highlighted row in the CfBrowser results list (saturating).
    CfBrowserMoveSelection(i32),
    /// `v` in CfBrowser -- cycles MC filter (None ↔ Some("any")) and re-emits search.
    CfBrowserToggleMcFilter,
    /// `l` in CfBrowser -- cycles loader filter (None ↔ Some(<instance loader>)) and re-emits search.
    CfBrowserToggleLoaderFilter,
    /// Enter on a CfBrowser row -- Action ping-pong half 1: emits `Effect::FetchCfMod`.
    CfBrowserOpenDetail {
        slug: String,
        mod_id: u64,
    },
    /// Async result: project detail loaded -- Action ping-pong half 2: emits `Effect::ListCfFiles`.
    /// Mirrors Phase 8's `ModDetailLoaded → ModVersionsLoaded` chain.
    CfBrowserDetailLoaded {
        slug: String,
        detail: crate::mods::curseforge::types::CurseForgeProjectDetail,
    },
    /// Printable char into CfBrowser search input.
    CfBrowserTypeSearch(char),
    /// Pasted text into CfBrowser search input -- applies on terminals that
    /// emit `Event::Paste(String)` (bracketed-paste mode, enabled at terminal
    /// init in 08.1-04). Mirror of `CfBrowserTypeSearch(char)` but appends
    /// the full pasted string and re-emits the search effect once.
    CfBrowserPasteSearch(String),
    /// Backspace in CfBrowser search input -- pops last char, re-emits search.
    CfBrowserBackspaceSearch,
    /// Async result: file list loaded -- transitions to `CfFilePickerModal`.
    CfFilePickerLoaded {
        slug: String,
        mod_detail: crate::mods::curseforge::types::CurseForgeProjectDetail,
        files: Vec<crate::mods::curseforge::types::CurseForgeFileEntry>,
    },
    /// Move the highlighted file in the CfFilePickerModal (saturating).
    CfFilePickerMove(i32),
    /// Enter on a CfFilePickerModal row -- emits `Effect::InstallCfMod`.
    /// Pitfall 8 guard: silent no-op when `running_mod_jobs.contains_key(&slug)`.
    CfFilePickerConfirm,
    /// Internal -- emitted by execute_effects (09-07) inside the spawned task body
    /// BEFORE install_mod_into_instance begins. Inserts the CancellationToken
    /// into `running_mod_jobs` (single-mutation point shared with Phase 8).
    CfModInstallStarted {
        slug: String,
        mod_id: u64,
        file_id: u64,
        token: CancellationToken,
    },
    /// Install completed successfully -- clears `running_mod_jobs[&slug]`.
    CfModInstalled {
        slug: String,
        mod_id: u64,
    },
    /// Install failed -- clears `running_mod_jobs[&slug]`, transitions to
    /// `CfInstallFailedModal`. `web_url` is `Some(url)` for FileNotDownloadable
    /// (the load-bearing UX path per MOD-04).
    CfModInstallFailed {
        slug: String,
        mod_title: String,
        file_label: String,
        error: String,
        web_url: Option<String>,
    },
    /// Esc on CfInstallFailedModal -- returns to InstanceList.
    CfDismissInstallFailed,

    // ── Phase 10 (Modpack Import) -- see 10-PATTERNS.md ──
    /// `i` (lowercase) keybind on InstanceList -- opens the path-entry modal
    /// for importing a `.mrpack` file from disk.
    OpenModpackImport,
    /// User typed a character in the path-input modal.
    ImportPathTypeSearch(char),
    /// User pasted text into the path-input modal (bracketed-paste).
    ImportPathPasteSearch(String),
    /// User pressed Backspace in the path-input modal.
    ImportPathBackspaceSearch,
    /// User pressed Enter in the path-input modal -- submits the path.
    ImportPathSubmit,
    /// User pressed Esc in the path-input modal -- cancels without importing.
    ImportPathCancel,
    /// Internal -- emitted by the spawned task BEFORE calling import_mrpack.
    /// Inserts the CancellationToken into running_modpack_imports and transitions
    /// ActiveView to ModpackImportProgressModal.
    ModpackImportStarted {
        slug: String,
        modpack_name: String,
        token: CancellationToken,
    },
    /// Progress update from the import task -- updates the progress modal fields.
    ModpackImportProgress {
        slug: String,
        pct: u8,
        step_label: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    /// Import completed successfully -- clears running_modpack_imports[slug],
    /// transitions to InstanceList, emits Effect::FetchInstances.
    ModpackImported {
        slug: String,
    },
    /// Dispatched by the Effect-spawned task when ModpackError::Cancelled bubbles up.
    /// Distinct from ModpackImported so the update arm can `clear()` the
    /// running_modpack_imports map without needing to know the resolved slug (which
    /// the spawned task may not have if cancel preceded create_instance).
    ModpackImportCancelled,
    /// Import failed -- clears running_modpack_imports, transitions to ModpackImportFailedModal.
    ModpackImportFailed {
        modpack_name: String,
        error: String,
        log_tail: String,
    },
    /// User-keystroke action: Esc on progress modal.
    /// No slug arg -- uses the unique current modpack import (HashMap is single-entry in v1).
    CancelModpackImport,
    /// Esc on ModpackImportFailedModal -- returns to InstanceList.
    DismissModpackImportFailed,

    // ── Phase 11 (11-04): Pack browser + installed list + drop-path ──────────
    /// Open the Modrinth pack browser for a given slug + kind.
    OpenPackBrowser {
        slug: String,
        kind: PackKind,
    },
    /// Navigate in the pack browser results list.
    PackBrowserMove(i32),
    /// Append a character to the pack browser search buffer.
    PackBrowserTypeSearch(char),
    /// Delete the last character from the pack browser search buffer.
    PackBrowserBackspaceSearch,
    /// Paste a string into the pack browser search buffer.
    PackBrowserPasteSearch(String),
    /// `/` in PackBrowser browse mode -- enters vim-style search mode
    /// (mirrors ModBrowserBeginSearch).
    PackBrowserBeginSearch,
    /// Esc in PackBrowser search mode -- exits search mode without closing
    /// the browser (mirrors ModBrowserExitSearch).
    PackBrowserExitSearch,
    /// Pack browser search results arrived -- slug+kind match guard applied in update().
    PackBrowserSearchLoaded {
        slug: String,
        kind: PackKind,
        hits: Vec<ModrinthSearchHit>,
    },
    /// Pack browser search failed.
    PackBrowserSearchFailed {
        slug: String,
        kind: PackKind,
        message: String,
    },
    /// Esc on pack browser -- return to InstanceList.
    PackBrowserClose,
    /// `D` inside a pack browser -- open the drop-from-path modal.
    PackDropPathOpen {
        slug: String,
        kind: PackKind,
    },
    /// Append a character to the pack drop path buffer.
    PackDropPathType(char),
    /// Delete the last character from the pack drop path buffer.
    PackDropPathBackspace,
    /// Paste into the pack drop path buffer.
    PackDropPathPaste(String),
    /// Enter -- submit the drop path.
    PackDropPathSubmit,
    /// Esc -- cancel the drop path modal, return to pack browser.
    PackDropPathCancel,
    /// Installed packs list loaded (async).
    InstalledPacksLoaded {
        slug: String,
        kind: PackKind,
        packs: Vec<InstalledModRow>,
    },
    /// Navigate in the installed packs list.
    InstalledPacksMove(i32),
    /// Tab key on InstalledModsList or InstalledPacksList -- cycle Mod→Resource→Shader→Mod.
    InstalledPacksCycleKind,
    /// `e` on an installed pack row (Resource kind) -- toggle enabled.
    TogglePackEnabled,
    /// `e` on an installed pack row (Shader kind) -- show transient notice.
    ShaderToggleNotice,
    /// `x` on an installed pack row -- open uninstall confirm.
    OpenUninstallPackConfirm,
    /// `y` on UninstallPackConfirm.
    ConfirmUninstallPack,
    /// `n`/Esc on UninstallPackConfirm.
    CancelUninstallPack,
    /// Pack installed successfully (drop or Modrinth).
    PackInstalled {
        slug: String,
        kind: PackKind,
    },
    /// Pack install failed (Modrinth path).
    PackInstallFailed {
        slug: String,
        kind: PackKind,
        /// User-facing project title for the failure modal heading.
        /// Empty string is acceptable for tail-routed cases (e.g. version
        /// fetch failures where the title is unknown); the modal renders
        /// "(unknown)" in that case.
        pack_title: String,
        /// User-facing version label for the failure modal subtitle.
        /// Empty when no version was selected yet.
        version_label: String,
        error: String,
    },
    /// Esc on PackInstallFailedModal -- returns to PackBrowser or
    /// InstalledPacksList per `return_to`.
    DismissPackInstallFailed,
    /// Pack drop-from-path install failed.
    PackDropFailed {
        slug: String,
        kind: PackKind,
        error: String,
    },
    /// Pack uninstalled successfully.
    PackUninstalled {
        slug: String,
        kind: PackKind,
        mod_id: String,
    },
    /// Pack enabled/disabled state toggled.
    PackToggled {
        slug: String,
        kind: PackKind,
        mod_id: String,
        new_enabled: bool,
    },
    /// Pack toggle failed (rename failed, file missing, shader rejected, etc.).
    /// Surfaced as `transient_status` on `InstalledPacksList`. (GAP-11-B)
    PackToggleFailed {
        slug: String,
        kind: PackKind,
        error: String,
    },
    /// Install a pack from a Modrinth version (Enter in pack browser).
    InstallPackFromBrowser {
        slug: String,
        kind: PackKind,
    },
    /// Browser auto-pick stage: full `ModrinthVersion` resolved by the
    /// `Effect::FetchPackVersions` task; `update()` arm dispatches the
    /// existing `Effect::InstallPackFromModrinth` (GAP-11-A wiring).
    AutoStartPackInstall {
        slug: String,
        kind: PackKind,
        project_id: String,
        project_slug: String,
        project_title: String,
        version: ModrinthVersion,
    },
    /// Browser auto-pick stage: list-versions or get-version request failed.
    /// Tail-routed to `Action::PackInstallFailed` so the existing failure UI
    /// surfaces uniformly (GAP-11-A wiring).
    PackVersionsFailed {
        slug: String,
        kind: PackKind,
        project_id: String,
        message: String,
    },
    /// Phase 13: an `Effect::FetchIcon` task completed; the IconService
    /// LRU now holds the decoded `Protocol` for `(source, project_id)`.
    /// Carries no payload -- consumers re-read via `service.try_get()`.
    /// State change is a no-op; the variant exists so the receive loop
    /// triggers a re-render frame.
    IconFetched {
        source: crate::icons::IconSource,
        project_id: String,
    },
}

/// Effects requested by `update()`. NOTE: there is deliberately NO
/// `SpawnVersionInstall` variant -- creating an instance and installing its
/// version are performed by a single `CreateInstance` effect, handled
/// atomically by the runtime (02-07-03 `execute_effects`). See 02-07 plan
/// header for the rationale (checker blocker B2).
#[derive(Debug, Clone)]
pub enum Effect {
    Quit,
    FetchManifest,
    FetchInstances,
    DeleteInstance(String),
    RenameInstance {
        slug: String,
        new_name: String,
    },
    CloneInstance {
        source_slug: String,
        new_name: String,
    },
    CreateInstance {
        display_name: String,
        mc_version_id: String,
        version_url: String,
        version_sha1: String,
    },
    SetGroup {
        slug: String,
        group: Option<String>,
    },
    /// Spawn a launch_instance task. auth_ctx built by update() from
    /// state.active_account_id + instance display_name.
    LaunchInstance {
        slug: String,
        auth_ctx: AuthContext,
    },
    /// Cancel the running launch task for the given slug.
    KillProcess {
        slug: String,
    },
    /// Spawn the device-code auth task (AccountService::start_device_code_auth).
    StartDeviceCodeAuth,
    /// Remove account via AccountService::remove_account, then reload list.
    RemoveAccount {
        id: String,
    },
    /// Activate account via AccountService::activate_account, then reload list.
    ActivateAccount {
        id: String,
    },
    /// Reload the account list from store (AccountService::list_accounts).
    FetchAccounts,
    /// Fetch detected system Javas then dispatch JavaPickerOptionsLoaded.
    FetchSystemJavas {
        slug: String,
    },
    /// Atomically write (or clear) java_override on the instance manifest.
    SetJavaOverride {
        slug: String,
        override_id: Option<JavaRuntimeId>,
    },
    /// Phase 6: fetch the list of loader versions for a given LoaderType.
    FetchLoaderVersions {
        slug: String,
        loader_type: LoaderType,
    },
    /// Phase 6: spawn the install_loader pipeline.
    InstallLoader {
        slug: String,
        loader_type: LoaderType,
        mc_version: String,
        loader_version: String,
    },
    /// Phase 6: cancel the running install for the given slug.
    CancelLoaderInstall {
        slug: String,
    },
    /// Phase 6: remove the active loader from an instance.
    RemoveLoader {
        slug: String,
    },

    // ── Phase 8 (Modrinth Integration) -- wired by 08-08 run.rs effect arms ──
    /// MOD-01: search Modrinth for mods matching the query, filtered by the
    /// instance's MC version + loader (with optional UI override).
    SearchModrinth {
        slug: String,
        query: String,
        mc: Option<String>,
        loader: Option<LoaderInfo>,
    },
    /// MOD-01: fetch the project detail (right pane) for the highlighted mod.
    FetchModDetail {
        slug: String,
        project_id: String,
    },
    /// MOD-01: list all versions of a project compatible with the instance's MC + loader.
    ListModVersions {
        slug: String,
        project_id: String,
        project_title: String,
        mc: Option<String>,
        loader: Option<LoaderInfo>,
    },
    /// MOD-02: BFS-resolve dependencies for a chosen version (with installed-set diff).
    ResolveModDependencies {
        slug: String,
        project_id: String,
        project_title: String,
        version_id: String,
        version_label: String,
        mc: String,
        loader: Option<LoaderInfo>,
    },
    /// MOD-02: download + verify + install the root + dep graph; updates ledger.
    InstallModWithDeps {
        slug: String,
        project_slug: String,
        project_title: String,
        root_version: Box<ModrinthVersion>,
        graph: Box<ResolvedDepGraph>,
    },
    /// MOD-06: rename `.jar` ↔ `.jar.disabled` and update the ledger row.
    /// Suffix `Eff` disambiguates from `Action::ToggleModEnabled`.
    ToggleModEnabledEff {
        slug: String,
        mod_id: String,
        want_enabled: bool,
    },
    /// MOD-07: delete the mod file and remove its row from the ledger.
    UninstallMod {
        slug: String,
        mod_id: String,
    },
    /// MOD-05: read the per-instance ledger and dispatch `InstalledModsLoaded`.
    FetchInstalledMods {
        slug: String,
    },

    // ── Phase 9 (CurseForge Integration) -- wired by 09-07 run.rs effect arms ──
    // LOCKED set: `FetchCfMod` + `ListCfFiles` are kept SEPARATE (mirrors
    // Phase 8's ListModVersions / FetchModDetail split). Do NOT add a combined
    // `OpenCfFilePicker` effect -- the design relies on the Action ping-pong
    // pattern: CfBrowserOpenDetail → FetchCfMod → CfBrowserDetailLoaded →
    // ListCfFiles → CfFilePickerLoaded.
    /// MOD-03: search CurseForge mods matching the query, filtered by MC + loader.
    SearchCurseForge {
        slug: String,
        query: String,
        mc: Option<String>,
        loader: Option<i32>,
    },
    /// MOD-03: fetch a single CurseForge project detail (right pane / file picker prep).
    FetchCfMod {
        slug: String,
        mod_id: u64,
    },
    /// MOD-03: list available files for a CurseForge mod, filtered by MC + loader.
    ListCfFiles {
        slug: String,
        mod_id: u64,
        mc: Option<String>,
        loader: Option<i32>,
    },
    /// MOD-03/MOD-04: download + verify + install a CurseForge file into the instance.
    /// FileNotDownloadable surfaces via `Action::CfModInstallFailed { web_url: Some(_), .. }`.
    /// Both wire types are boxed to keep `Effect` small (mirrors Phase 8's
    /// `InstallModWithDeps { root_version: Box<ModrinthVersion>, .. }` pattern).
    InstallCfMod {
        slug: String,
        mod_detail: Box<crate::mods::curseforge::types::CurseForgeProjectDetail>,
        file: Box<crate::mods::curseforge::types::CurseForgeFileEntry>,
    },

    // ── Phase 10 (Modpack Import) ──
    /// PACK-01: spawn the import_mrpack pipeline.
    ImportModpack {
        mrpack_path: std::path::PathBuf,
    },
    /// PACK-01: cancel the running modpack import (no-op hook for symmetry with
    /// CancelLoaderInstall -- the actual token.cancel() happened in update()).
    CancelModpackImport,

    // ── Phase 11 (11-04): Pack browser + install effects ─────────────────────
    /// Search Modrinth for packs of the given kind. `mc` is the MC version filter.
    SearchPacks {
        slug: String,
        kind: PackKind,
        query: String,
        mc: Option<String>,
    },
    /// Read the per-instance pack ledger for the given kind.
    FetchInstalledPacks {
        slug: String,
        kind: PackKind,
    },
    /// Copy-install a pack from a local file path.
    DropInstallPack {
        slug: String,
        kind: PackKind,
        path: std::path::PathBuf,
    },
    /// Download + install a pack from a Modrinth version record.
    InstallPackFromModrinth {
        slug: String,
        kind: PackKind,
        project_id: String,
        project_slug: String,
        project_title: String,
        version: ModrinthVersion,
    },
    /// Browser Enter-key install (GAP-11-A): list versions for a Modrinth pack
    /// project, auto-pick the first `is_latest_stable=true` (fallback
    /// `versions.first()`), fetch the full version body, then dispatch
    /// `Action::AutoStartPackInstall` which fans out to
    /// `Effect::InstallPackFromModrinth`.
    FetchPackVersions {
        slug: String,
        kind: PackKind,
        project_id: String,
        project_slug: String,
        project_title: String,
        mc: Option<String>,
    },
    /// Toggle a pack's enabled/disabled state (rename .zip ↔ .zip.disabled).
    TogglePackEnabledEff {
        slug: String,
        kind: PackKind,
        mod_id: String,
    },
    /// Remove a pack file and its ledger row.
    UninstallPack {
        slug: String,
        kind: PackKind,
        mod_id: String,
    },
    /// Phase 13: fetch a project icon, decode it, and cache the protocol
    /// in `IconService`. Triggered when a browser selection changes and
    /// `state.icon_rendering_enabled` is true. Completion is signalled
    /// via `Action::IconFetched`, which causes the next render frame to
    /// pick up the cached protocol via `IconService::try_get`.
    FetchIcon {
        source: crate::icons::IconSource,
        project_id: String,
        url: String,
    },
}

/// Format a loader for the status cell or switch dialog: "fabric:0.16.9".
fn loader_label_short(kind: crate::domain::instance::ModloaderKind, version: &str) -> String {
    use crate::domain::instance::ModloaderKind;
    let kind_str = match kind {
        ModloaderKind::Fabric => "fabric",
        ModloaderKind::Quilt => "quilt",
        ModloaderKind::Forge => "forge",
        ModloaderKind::NeoForge => "neoforge",
        ModloaderKind::Vanilla => "vanilla",
    };
    format!("{kind_str}:{version}")
}

/// Apply the current LoaderVersionPickerModal filter+search to its `versions`,
/// returning indices (into the original list) of visible rows. Quilt with
/// `filter_stable_only=true` shows all versions but the renderer adds `(beta)`
/// suffix per UI-SPEC §Open Question 3 lock-in. For Fabric, `filter_stable_only`
/// hides unstable versions.
pub fn loader_versions_visible_indices(
    versions: &[LoaderVersionEntry],
    loader: LoaderType,
    filter_stable_only: bool,
    search: &str,
) -> Vec<usize> {
    let s_lc = search.to_ascii_lowercase();
    versions
        .iter()
        .enumerate()
        .filter(|(_, v)| match loader {
            LoaderType::Fabric => !filter_stable_only || v.stable,
            LoaderType::Quilt => true, // see UI-SPEC §Loader Version Picker (Quilt)
            // Forge/NeoForge: apply stable filter same as Fabric (07-02 populates stable field)
            LoaderType::Forge | LoaderType::NeoForge => !filter_stable_only || v.stable,
        })
        .filter(|(_, v)| s_lc.is_empty() || v.version.to_ascii_lowercase().contains(&s_lc))
        .map(|(i, _)| i)
        .collect()
}

/// Apply an `Action`, mutate `state`, and return the side-effects to execute.
///
/// This is the single mutation point for all UI state. Views receive `&AppState`
/// (immutable) -- no other code path mutates `AppState`.
pub fn update(state: &mut AppState, action: Action) -> Vec<Effect> {
    match action {
        Action::Quit => {
            state.should_quit = true;
            vec![Effect::Quit]
        }
        Action::Task(TaskEvent::Progress { id, pct, msg }) => {
            match state.active_jobs.iter_mut().find(|(jid, _, _)| *jid == id) {
                Some(slot) => {
                    slot.1 = pct;
                    slot.2 = msg;
                }
                None => state.active_jobs.push((id, pct, msg)),
            }
            vec![]
        }
        Action::Task(TaskEvent::Completed { id, result }) => {
            state.active_jobs.retain(|(jid, _, _)| *jid != id);
            if let Err(e) = result {
                state.last_error = Some(e);
            }
            vec![Effect::FetchInstances]
        }
        Action::Tick => vec![],

        // Phase 13: icon fetch completed. State doesn't change -- the
        // protocol now sits in `IconService`'s LRU and the next render
        // frame picks it up via `try_get`. The variant exists so the
        // run-loop receives an action and triggers a redraw.
        Action::IconFetched { .. } => vec![],

        Action::OpenCreateModal => {
            state.active_view = ActiveView::CreateModal(CreateStep::NameInput {
                current: String::new(),
                error: None,
            });
            vec![Effect::FetchManifest]
        }
        Action::CloseModal => {
            state.active_view = ActiveView::default();
            vec![]
        }
        Action::OpenDeleteConfirm { slug, display_name } => {
            state.active_view = ActiveView::DeleteConfirm { slug, display_name };
            vec![]
        }
        Action::OpenRenameInline { slug, current } => {
            state.active_view = ActiveView::RenameInline {
                slug,
                current: current.clone(),
                original: current,
            };
            vec![]
        }
        Action::MoveSelection(delta) => {
            match &mut state.active_view {
                ActiveView::InstanceList { selected } => {
                    let len = state.instances.len() as isize;
                    if len > 0 {
                        let new_idx = (*selected as isize + delta).rem_euclid(len);
                        *selected = new_idx as usize;
                    }
                }
                ActiveView::AccountsList { selected } => {
                    let len = state.accounts.len() as isize;
                    if len > 0 {
                        let new_idx = (*selected as isize + delta).rem_euclid(len);
                        *selected = new_idx as usize;
                    }
                }
                _ => {}
            }
            vec![]
        }

        Action::TypeName(c) => {
            if let ActiveView::CreateModal(CreateStep::NameInput { current, error }) =
                &mut state.active_view
            {
                current.push(c);
                *error = None;
            }
            vec![]
        }
        Action::PasteName(s) => {
            // Bracketed-paste payload: append full pasted string in one
            // dispatch (08.1-04 / GAP-8-C). Mirrors TypeName: pure buffer
            // mutation, no effect emitted.
            if let ActiveView::CreateModal(CreateStep::NameInput { current, error }) =
                &mut state.active_view
            {
                current.push_str(&s);
                *error = None;
            }
            vec![]
        }
        Action::BackspaceName => {
            if let ActiveView::CreateModal(CreateStep::NameInput { current, .. }) =
                &mut state.active_view
            {
                current.pop();
            }
            vec![]
        }
        Action::SubmitInstanceName(name) => {
            let trimmed = name.trim().to_string();
            if trimmed.is_empty() {
                if let ActiveView::CreateModal(CreateStep::NameInput { error, .. }) =
                    &mut state.active_view
                {
                    *error = Some("name cannot be empty".into());
                }
                return vec![];
            }
            state.active_view = ActiveView::CreateModal(CreateStep::VersionPicker {
                name: trimmed,
                filter: state.versions_filter,
                search: String::new(),
                error: None,
            });
            vec![]
        }
        Action::SetVersionFilter(f) => {
            state.versions_filter = f;
            if let ActiveView::CreateModal(CreateStep::VersionPicker { filter, .. }) =
                &mut state.active_view
            {
                *filter = f;
            }
            vec![]
        }
        Action::TypeSearch(c) => {
            if let ActiveView::CreateModal(CreateStep::VersionPicker { search, .. }) =
                &mut state.active_view
            {
                search.push(c);
            }
            vec![]
        }
        Action::BackspaceSearch => {
            if let ActiveView::CreateModal(CreateStep::VersionPicker { search, .. }) =
                &mut state.active_view
            {
                search.pop();
            }
            vec![]
        }
        Action::ClearSearch => {
            if let ActiveView::CreateModal(CreateStep::VersionPicker { search, .. }) =
                &mut state.active_view
            {
                search.clear();
            }
            vec![]
        }
        Action::SelectVersion(id) => {
            // Look up the full VersionEntry to pass URL + SHA1 to the Effect.
            let entry = state.versions.iter().find(|v| v.id == id).cloned();
            let eff = match (&state.active_view, entry) {
                (ActiveView::CreateModal(CreateStep::VersionPicker { name, .. }), Some(v)) => {
                    Effect::CreateInstance {
                        display_name: name.clone(),
                        mc_version_id: v.id.clone(),
                        version_url: v.url.clone(),
                        version_sha1: v.sha1.clone(),
                    }
                }
                _ => return vec![],
            };
            state.active_view = ActiveView::default();
            vec![eff]
        }

        Action::TypeRename(c) => {
            if let ActiveView::RenameInline { current, .. } = &mut state.active_view {
                current.push(c);
            }
            vec![]
        }
        Action::BackspaceRename => {
            if let ActiveView::RenameInline { current, .. } = &mut state.active_view {
                current.pop();
            }
            vec![]
        }
        Action::SubmitRename(new_name) => {
            let trimmed = new_name.trim().to_string();
            if let ActiveView::RenameInline { slug, .. } = state.active_view.clone() {
                state.active_view = ActiveView::default();
                if trimmed.is_empty() {
                    return vec![];
                }
                return vec![Effect::RenameInstance {
                    slug,
                    new_name: trimmed,
                }];
            }
            vec![]
        }
        Action::CloneSelected => {
            if let ActiveView::InstanceList { selected } = &state.active_view {
                if let Some(m) = state.instances.get(*selected) {
                    let new_name = format!("{} (Copy)", m.display_name);
                    return vec![Effect::CloneInstance {
                        source_slug: m.slug.clone(),
                        new_name,
                    }];
                }
            }
            vec![]
        }
        Action::ConfirmDelete => {
            if let ActiveView::DeleteConfirm { slug, .. } = state.active_view.clone() {
                state.active_view = ActiveView::default();
                return vec![Effect::DeleteInstance(slug)];
            }
            vec![]
        }
        Action::CancelDelete => {
            state.active_view = ActiveView::default();
            vec![]
        }

        Action::OpenGroupInput { slug, current } => {
            state.active_view = ActiveView::GroupInline {
                slug,
                buffer: current.clone(),
                original: if current.is_empty() {
                    None
                } else {
                    Some(current)
                },
            };
            vec![]
        }
        Action::TypeGroup(c) => {
            if let ActiveView::GroupInline { buffer, .. } = &mut state.active_view {
                buffer.push(c);
            }
            vec![]
        }
        Action::BackspaceGroup => {
            if let ActiveView::GroupInline { buffer, .. } = &mut state.active_view {
                buffer.pop();
            }
            vec![]
        }
        Action::SubmitGroup => {
            if let ActiveView::GroupInline { slug, buffer, .. } = state.active_view.clone() {
                state.active_view = ActiveView::default();
                let trimmed = buffer.trim().to_string();
                let group = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                };
                return vec![Effect::SetGroup { slug, group }];
            }
            vec![]
        }
        Action::CancelGroupInput => {
            state.active_view = ActiveView::default();
            vec![]
        }

        Action::ManifestLoaded(versions) => {
            state.versions = versions;
            vec![]
        }
        Action::InstancesLoaded(list) => {
            state.instances = list;
            vec![]
        }
        Action::VersionInstalled { .. }
        | Action::InstanceDeleted(_)
        | Action::InstanceRenamed { .. }
        | Action::InstanceCloned { .. } => vec![Effect::FetchInstances],
        Action::VersionInstallFailed { error, .. } => {
            state.last_error = Some(error);
            vec![Effect::FetchInstances]
        }
        Action::ServiceErrored(e) => {
            state.last_error = Some(e);
            vec![]
        }

        // Phase 4: accounts
        Action::OpenAccounts => {
            state.active_view = ActiveView::AccountsList { selected: 0 };
            vec![Effect::FetchAccounts]
        }
        Action::CloseAccounts => {
            if let Some(tok) = state.add_account_cancel.take() {
                tok.cancel();
            }
            state.active_view = ActiveView::default();
            vec![]
        }
        Action::AccountsLoaded(list) => {
            state.active_account_id = list.iter().find(|a| a.is_active).map(|a| a.id.clone());
            state.accounts = list;
            vec![]
        }
        Action::AddAccount => {
            vec![Effect::StartDeviceCodeAuth]
        }
        Action::AccountAuthStarted {
            user_code,
            verification_uri,
            expires_at,
        } => {
            state.active_view = ActiveView::AddAccountDeviceCode {
                user_code,
                verification_uri,
                expires_at,
                stage: "waiting for user".into(),
            };
            vec![]
        }
        Action::AccountAuthProgress { stage } => {
            if let ActiveView::AddAccountDeviceCode { stage: s, .. } = &mut state.active_view {
                *s = stage;
            }
            vec![]
        }
        Action::AccountAdded { account } => {
            state.add_account_cancel = None;
            if account.is_active {
                state.active_account_id = Some(account.id.clone());
            }
            if let Some(existing) = state.accounts.iter_mut().find(|a| a.id == account.id) {
                *existing = account;
            } else {
                state.accounts.push(account);
            }
            state.active_view = ActiveView::AccountsList { selected: 0 };
            vec![Effect::FetchAccounts]
        }
        Action::AccountAuthFailed { reason } => {
            state.add_account_cancel = None;
            state.active_view = ActiveView::AccountAuthFailed { reason };
            vec![]
        }
        Action::RemoveAccount { id } => {
            state.accounts.retain(|a| a.id != id);
            if state.active_account_id.as_deref() == Some(id.as_str()) {
                state.active_account_id = None;
            }
            vec![Effect::RemoveAccount { id }]
        }
        Action::ActivateAccount { id } => {
            for a in state.accounts.iter_mut() {
                a.is_active = a.id == id;
            }
            state.active_account_id = Some(id.clone());
            vec![Effect::ActivateAccount { id }]
        }
        Action::CancelAddAccount => {
            if let Some(tok) = state.add_account_cancel.take() {
                tok.cancel();
            }
            state.active_view = ActiveView::AccountsList { selected: 0 };
            vec![]
        }
        Action::AddAccountTokenCreated(token) => {
            state.add_account_cancel = Some(token);
            vec![]
        }

        // Phase 3: launch lifecycle
        Action::LaunchInstance { slug } => {
            if state.running_instances.contains_key(&slug) {
                // Belt-and-suspenders: already running, no-op (T-03-05-01).
                vec![]
            } else {
                let display_name = state
                    .instances
                    .iter()
                    .find(|m| m.slug == slug)
                    .map(|m| m.display_name.clone())
                    .unwrap_or_else(|| slug.clone());
                let auth_ctx = if let Some(aid) = state.active_account_id.clone() {
                    AuthContext::Msa { account_id: aid }
                } else {
                    AuthContext::Offline {
                        username: display_name,
                    }
                };
                vec![Effect::LaunchInstance { slug, auth_ctx }]
            }
        }
        Action::LaunchJobStarted { slug, token } => {
            state.running_instances.insert(slug, token);
            vec![]
        }
        Action::InstanceLaunched { slug: _ } => {
            // Tracing signal only -- token already inserted via LaunchJobStarted.
            vec![]
        }
        Action::InstanceExited {
            slug,
            duration_ms: _,
        } => {
            state.running_instances.remove(&slug);
            vec![Effect::FetchInstances]
        }
        Action::LaunchFailed {
            slug,
            error,
            log_tail,
        } => {
            state.running_instances.remove(&slug);
            state.active_view = ActiveView::LaunchFailedModal {
                slug,
                error,
                log_tail,
            };
            vec![]
        }
        // Phase 5: Java picker
        Action::OpenJavaPicker { slug } => {
            // Block picker for running instances (can't change java mid-game).
            if state.running_instances.contains_key(&slug) {
                return vec![];
            }
            state.active_view = ActiveView::JavaPickerModal {
                slug: slug.clone(),
                options: vec![JavaPickerRow::Auto, JavaPickerRow::Manual],
                selected: 0,
            };
            vec![Effect::FetchSystemJavas { slug }]
        }
        Action::JavaPickerOptionsLoaded { slug, options } => {
            if let ActiveView::JavaPickerModal {
                slug: modal_slug,
                options: ref mut opts,
                selected,
            } = &mut state.active_view
            {
                if *modal_slug == slug {
                    *opts = options;
                    *selected = 0;
                }
            }
            vec![]
        }
        Action::JavaPickerMove(delta) => {
            if let ActiveView::JavaPickerModal {
                options, selected, ..
            } = &mut state.active_view
            {
                let len = options.len() as isize;
                if len > 0 {
                    let new_idx = (*selected as isize + delta).rem_euclid(len);
                    *selected = new_idx as usize;
                }
            }
            vec![]
        }
        Action::JavaPickerSelect => {
            if let ActiveView::JavaPickerModal {
                slug,
                options,
                selected,
            } = state.active_view.clone()
            {
                let override_id = match options.get(selected) {
                    Some(JavaPickerRow::Auto) => None,
                    Some(JavaPickerRow::Detected(sj)) => Some(JavaRuntimeId::System {
                        path: sj.path.clone(),
                        major_version: sj.major_version,
                    }),
                    Some(JavaPickerRow::Manual) | None => {
                        // Manual row: just close, no mutation.
                        state.active_view = ActiveView::default();
                        return vec![];
                    }
                };
                state.active_view = ActiveView::default();
                return vec![Effect::SetJavaOverride { slug, override_id }];
            }
            vec![]
        }
        Action::JavaPickerCancel => {
            state.active_view = ActiveView::default();
            vec![]
        }
        Action::JavaOverrideSet { .. } => {
            vec![Effect::FetchInstances]
        }
        Action::JavaOverrideFailed { reason, .. } => {
            state.last_error = Some(reason);
            state.active_view = ActiveView::default();
            vec![Effect::FetchInstances]
        }

        // Phase 6: Loader picker arms
        Action::OpenLoaderPicker { slug } => {
            // T-06-16: block picker if instance is running.
            if state.running_instances.contains_key(&slug) {
                return vec![];
            }
            state.active_view = ActiveView::LoaderPickerModal { slug, selected: 0 };
            vec![]
        }
        Action::LoaderPickerMove(delta) => {
            if let ActiveView::LoaderPickerModal { selected, .. } = &mut state.active_view {
                const ROWS: isize = 5; // None / Fabric / Quilt / Forge / NeoForge
                let new_idx = (*selected as isize + delta).rem_euclid(ROWS);
                *selected = new_idx as usize;
            }
            vec![]
        }
        Action::LoaderPickerSelect => {
            let (slug, selected) = match &state.active_view {
                ActiveView::LoaderPickerModal { slug, selected } => (slug.clone(), *selected),
                _ => return vec![],
            };
            match selected {
                0 => {
                    // None row: if instance has a loader → open switch confirm to "none".
                    // Otherwise it's already vanilla -- no-op.
                    let has_loader = state
                        .instances
                        .iter()
                        .find(|m| m.slug == slug)
                        .and_then(|m| m.loader.as_ref())
                        .is_some();
                    if has_loader {
                        let from_label = state
                            .instances
                            .iter()
                            .find(|m| m.slug == slug)
                            .and_then(|m| m.loader.as_ref())
                            .map(|l| loader_label_short(l.kind, &l.version));
                        state.active_view = ActiveView::LoaderSwitchConfirm {
                            slug,
                            from_loader: from_label,
                            to_loader: "none".into(),
                            type_switch: false,
                        };
                    }
                    vec![]
                }
                1 => {
                    // Fabric row
                    let current_version = state
                        .instances
                        .iter()
                        .find(|m| m.slug == slug)
                        .and_then(|m| m.loader.as_ref())
                        .filter(|l| l.kind == crate::domain::instance::ModloaderKind::Fabric)
                        .map(|l| l.version.clone());
                    state.active_view = ActiveView::LoaderVersionPickerModal {
                        slug: slug.clone(),
                        loader: LoaderType::Fabric,
                        versions: vec![],
                        filter_stable_only: true,
                        search: String::new(),
                        selected: 0,
                        current_version,
                    };
                    vec![Effect::FetchLoaderVersions {
                        slug,
                        loader_type: LoaderType::Fabric,
                    }]
                }
                2 => {
                    // Quilt row -- show all by default (Open Question 3 lock)
                    let current_version = state
                        .instances
                        .iter()
                        .find(|m| m.slug == slug)
                        .and_then(|m| m.loader.as_ref())
                        .filter(|l| l.kind == crate::domain::instance::ModloaderKind::Quilt)
                        .map(|l| l.version.clone());
                    state.active_view = ActiveView::LoaderVersionPickerModal {
                        slug: slug.clone(),
                        loader: LoaderType::Quilt,
                        versions: vec![],
                        filter_stable_only: false,
                        search: String::new(),
                        selected: 0,
                        current_version,
                    };
                    vec![Effect::FetchLoaderVersions {
                        slug,
                        loader_type: LoaderType::Quilt,
                    }]
                }
                3 => {
                    // Forge row
                    let current_version = state
                        .instances
                        .iter()
                        .find(|m| m.slug == slug)
                        .and_then(|m| m.loader.as_ref())
                        .filter(|l| l.kind == crate::domain::instance::ModloaderKind::Forge)
                        .map(|l| l.version.clone());
                    state.active_view = ActiveView::LoaderVersionPickerModal {
                        slug: slug.clone(),
                        loader: LoaderType::Forge,
                        versions: vec![],
                        filter_stable_only: true,
                        search: String::new(),
                        selected: 0,
                        current_version,
                    };
                    vec![Effect::FetchLoaderVersions {
                        slug,
                        loader_type: LoaderType::Forge,
                    }]
                }
                4 => {
                    // NeoForge row
                    let current_version = state
                        .instances
                        .iter()
                        .find(|m| m.slug == slug)
                        .and_then(|m| m.loader.as_ref())
                        .filter(|l| l.kind == crate::domain::instance::ModloaderKind::NeoForge)
                        .map(|l| l.version.clone());
                    state.active_view = ActiveView::LoaderVersionPickerModal {
                        slug: slug.clone(),
                        loader: LoaderType::NeoForge,
                        versions: vec![],
                        filter_stable_only: true,
                        search: String::new(),
                        selected: 0,
                        current_version,
                    };
                    vec![Effect::FetchLoaderVersions {
                        slug,
                        loader_type: LoaderType::NeoForge,
                    }]
                }
                _ => vec![],
            }
        }
        Action::LoaderPickerCancel => {
            state.active_view = ActiveView::default();
            vec![]
        }
        Action::LoaderVersionsLoaded {
            slug,
            loader,
            versions,
        } => {
            if let ActiveView::LoaderVersionPickerModal {
                slug: modal_slug,
                loader: modal_loader,
                versions: ref mut v,
                selected,
                ..
            } = &mut state.active_view
            {
                if *modal_slug == slug && *modal_loader == loader {
                    *v = versions;
                    *selected = 0;
                }
            }
            vec![]
        }
        Action::LoaderVersionPickerMove(delta) => {
            if let ActiveView::LoaderVersionPickerModal {
                versions,
                loader,
                filter_stable_only,
                search,
                selected,
                ..
            } = &mut state.active_view
            {
                let visible =
                    loader_versions_visible_indices(versions, *loader, *filter_stable_only, search);
                let len = visible.len() as isize;
                if len > 0 {
                    let new_idx = (*selected as isize + delta).rem_euclid(len);
                    *selected = new_idx as usize;
                }
            }
            vec![]
        }
        Action::ToggleStableFilter => {
            if let ActiveView::LoaderVersionPickerModal {
                filter_stable_only,
                selected,
                ..
            } = &mut state.active_view
            {
                *filter_stable_only = !*filter_stable_only;
                *selected = 0;
            }
            vec![]
        }
        Action::LoaderVersionTypeSearch(c) => {
            if let ActiveView::LoaderVersionPickerModal {
                search, selected, ..
            } = &mut state.active_view
            {
                search.push(c);
                *selected = 0;
            }
            vec![]
        }
        Action::LoaderVersionBackspaceSearch => {
            if let ActiveView::LoaderVersionPickerModal {
                search, selected, ..
            } = &mut state.active_view
            {
                search.pop();
                *selected = 0;
            }
            vec![]
        }
        Action::LoaderVersionSelect => {
            let (
                slug,
                loader_type,
                versions,
                filter_stable_only,
                search,
                selected,
                current_version,
            ) = match &state.active_view {
                ActiveView::LoaderVersionPickerModal {
                    slug,
                    loader,
                    versions,
                    filter_stable_only,
                    search,
                    selected,
                    current_version,
                } => (
                    slug.clone(),
                    *loader,
                    versions.clone(),
                    *filter_stable_only,
                    search.clone(),
                    *selected,
                    current_version.clone(),
                ),
                _ => return vec![],
            };
            let visible = loader_versions_visible_indices(
                &versions,
                loader_type,
                filter_stable_only,
                &search,
            );
            let real_idx = match visible.get(selected) {
                Some(&i) => i,
                None => return vec![], // empty list -- no-op
            };
            let chosen = &versions[real_idx];
            let chosen_version = chosen.version.clone();

            // Look up the currently installed loader for this instance, regardless
            // of kind. The `current_version` plumbed through from the version picker
            // is filtered by target kind (used only for the same-kind in-picker
            // "← currently installed" hint), so it is None for cross-kind switches
            // and cannot be used to detect that the user is switching loader TYPE.
            // Compute installed_loader fresh here so cross-kind switches still trip
            // the WARNING confirm modal (UAT Check 5 regression -- major).
            let installed_loader: Option<(crate::domain::instance::ModloaderKind, String)> = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .and_then(|m| m.loader.as_ref())
                .map(|l| (l.kind, l.version.clone()));

            let target_kind = match loader_type {
                LoaderType::Fabric => crate::domain::instance::ModloaderKind::Fabric,
                LoaderType::Quilt => crate::domain::instance::ModloaderKind::Quilt,
                LoaderType::Forge => crate::domain::instance::ModloaderKind::Forge,
                LoaderType::NeoForge => crate::domain::instance::ModloaderKind::NeoForge,
            };

            // Same exact loader+version already installed -- no-op (also covers the
            // historical `current_version.as_deref() == Some(&chosen_version)` case).
            if let Some((installed_kind, ref installed_ver)) = installed_loader {
                if installed_kind == target_kind && installed_ver == &chosen_version {
                    return vec![];
                }
            }

            let mc_version = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| m.mc_version_id.clone())
                .unwrap_or_default();

            match installed_loader {
                Some((installed_kind, installed_ver)) => {
                    // Switching -- same kind (different version) OR cross-kind (TYPE switch).
                    // type_switch flips on cross-kind to render the red WARNING line in
                    // LoaderSwitchConfirm (loader_switch_confirm.rs:25 conditional).
                    let type_switch = installed_kind != target_kind;
                    let from_label = Some(loader_label_short(installed_kind, &installed_ver));
                    let to_label = loader_label_short(target_kind, &chosen_version);
                    state.active_view = ActiveView::LoaderSwitchConfirm {
                        slug,
                        from_loader: from_label,
                        to_loader: to_label,
                        type_switch,
                    };
                    let _ = current_version; // intentionally unused after fix; kept on the picker for the in-modal `← currently installed` hint
                    vec![]
                }
                None => {
                    // Vanilla instance, no installed loader -- emit install effect directly.
                    state.active_view = ActiveView::LoaderInstallProgressModal {
                        slug: slug.clone(),
                        loader: loader_type,
                        version: chosen_version.clone(),
                        step_label: "Fetching meta".into(),
                        step_index: 1,
                        step_total: 4,
                        bytes_done: 0,
                        bytes_total: 0,
                        cancel_token_key: slug.clone(),
                        log_tail: String::new(),
                    };
                    vec![Effect::InstallLoader {
                        slug,
                        loader_type,
                        mc_version,
                        loader_version: chosen_version,
                    }]
                }
            }
        }
        Action::LoaderVersionPickerCancel => {
            // Return to the loader picker (select the row matching the current loader type).
            let (slug, loader) = match &state.active_view {
                ActiveView::LoaderVersionPickerModal { slug, loader, .. } => {
                    (slug.clone(), *loader)
                }
                _ => {
                    state.active_view = ActiveView::default();
                    return vec![];
                }
            };
            let row = match loader {
                LoaderType::Fabric => 1,
                LoaderType::Quilt => 2,
                LoaderType::Forge => 3,
                LoaderType::NeoForge => 4,
            };
            state.active_view = ActiveView::LoaderPickerModal {
                slug,
                selected: row,
            };
            vec![]
        }
        Action::LoaderInstallStarted { slug, token } => {
            state.running_loader_installs.insert(slug, token);
            vec![]
        }
        Action::LoaderInstallProgress {
            slug,
            pct,
            step_label,
            bytes_done,
            bytes_total,
        } => {
            if let ActiveView::LoaderInstallProgressModal {
                slug: modal_slug,
                step_label: ref mut sl,
                bytes_done: ref mut bd,
                bytes_total: ref mut bt,
                step_index: ref mut si,
                ..
            } = &mut state.active_view
            {
                if *modal_slug == slug {
                    *sl = step_label;
                    *bd = bytes_done;
                    *bt = bytes_total;
                    // pct drives step_index: 0-33% = step 1, 34-66% = step 2, 67-99% = step 3, 100% = step 4
                    *si = match pct {
                        0..=33 => 1,
                        34..=66 => 2,
                        67..=99 => 3,
                        _ => 4,
                    };
                }
            }
            vec![]
        }
        Action::LoaderInstallLogTail { slug, tail } => {
            // Phase 7 (D-02): update the live log tail in the progress modal without
            // touching the gauge percentage -- these events come from [log-tail]-prefixed
            // TaskEvent::Progress messages filtered in the run.rs forwarder.
            if let ActiveView::LoaderInstallProgressModal {
                slug: modal_slug,
                log_tail: ref mut lt,
                ..
            } = &mut state.active_view
            {
                if *modal_slug == slug {
                    *lt = tail;
                }
            }
            vec![]
        }
        Action::LoaderInstalled { slug } => {
            state.running_loader_installs.remove(&slug);
            state.active_view = ActiveView::default();
            vec![Effect::FetchInstances]
        }
        Action::LoaderInstallFailed {
            slug,
            loader,
            version,
            error,
            log_tail,
        } => {
            state.running_loader_installs.remove(&slug);
            state.active_view = ActiveView::LoaderInstallFailedModal {
                slug,
                loader,
                version,
                error,
                log_tail,
            };
            vec![]
        }
        Action::CancelLoaderInstall { slug } => {
            if let Some(token) = state.running_loader_installs.remove(&slug) {
                token.cancel();
            }
            state.active_view = ActiveView::default();
            vec![Effect::CancelLoaderInstall { slug }]
        }
        Action::DismissLoaderInstallFailed => {
            state.active_view = ActiveView::default();
            vec![]
        }
        Action::ConfirmLoaderSwitch => {
            let (slug, to_loader) = match &state.active_view {
                ActiveView::LoaderSwitchConfirm {
                    slug, to_loader, ..
                } => (slug.clone(), to_loader.clone()),
                _ => return vec![],
            };
            state.active_view = ActiveView::default();
            if to_loader == "none" {
                return vec![Effect::RemoveLoader { slug }];
            }
            // Parse "kind:version" format.
            let (kind_str, loader_version) = match to_loader.split_once(':') {
                Some(parts) => parts,
                None => return vec![],
            };
            let loader_type = match kind_str {
                "fabric" => LoaderType::Fabric,
                "quilt" => LoaderType::Quilt,
                "forge" => LoaderType::Forge,
                "neoforge" => LoaderType::NeoForge,
                _ => return vec![],
            };
            let mc_version = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| m.mc_version_id.clone())
                .unwrap_or_default();
            let loader_version = loader_version.to_string();
            state.active_view = ActiveView::LoaderInstallProgressModal {
                slug: slug.clone(),
                loader: loader_type,
                version: loader_version.clone(),
                step_label: "Fetching meta".into(),
                step_index: 1,
                step_total: 4,
                bytes_done: 0,
                bytes_total: 0,
                cancel_token_key: slug.clone(),
                log_tail: String::new(),
            };
            vec![Effect::InstallLoader {
                slug,
                loader_type,
                mc_version,
                loader_version,
            }]
        }
        Action::CancelLoaderSwitch => {
            state.active_view = ActiveView::default();
            vec![]
        }

        // ── Phase 8 (Modrinth Integration) -- pure update() arms ──
        // All arms below mutate AppState and (optionally) emit Effects. NO arm
        // performs HTTP, file I/O, or task spawning -- those live in 08-08 run.rs.
        Action::OpenModBrowser { slug } => {
            // Pitfall 8 (08-RESEARCH.md §Pitfall 8): silent no-op if a previous
            // mod install for this instance is still in flight. The user is not
            // shown an error -- just nothing happens. Tested by
            // `test_open_mod_browser_blocked_when_install_in_flight`.
            if state.running_mod_jobs.contains_key(&slug) {
                return vec![];
            }
            let (mc, loader) = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| (Some(m.mc_version_id.clone()), m.loader.clone()))
                .unwrap_or((None, None));
            state.active_view = ActiveView::ModBrowser {
                slug: slug.clone(),
                search: String::new(),
                is_searching: false,
                mc_filter_override: None,
                loader_filter_override: None,
                results: Vec::new(),
                selected: 0,
                fetch_state: ModBrowserFetchState::Loading,
                selected_detail: None,
            };
            vec![
                Effect::SearchModrinth {
                    slug: slug.clone(),
                    query: String::new(),
                    mc,
                    loader,
                },
                // Refresh installed-set cache so ModBrowserSearchLoaded can
                // stamp `already_installed` immune to install/search race.
                Effect::FetchInstalledMods { slug },
            ]
        }

        Action::ModBrowserSearchLoaded { slug, hits } => {
            // Snapshot installed-set before mutably borrowing active_view.
            let installed = state.installed_mod_project_ids.get(&slug).cloned();
            let icons_on = state.icon_rendering_enabled;
            let mut effects = Vec::new();
            if let ActiveView::ModBrowser {
                slug: cur_slug,
                results,
                selected,
                fetch_state,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    let new_len = hits.len();
                    *results = hits;
                    // Re-stamp `already_installed` from the in-memory set:
                    // the search result's pre-stamp from the service is a
                    // ledger snapshot taken at search-issue time and may miss
                    // installs that completed during the round-trip.
                    if let Some(set) = installed.as_ref() {
                        for r in results.iter_mut() {
                            if set.contains(&r.project_id) {
                                r.already_installed = true;
                            }
                        }
                    }
                    *fetch_state = ModBrowserFetchState::Ready;
                    *selected = (*selected).min(new_len.saturating_sub(1));
                    // Phase 13: kick off the icon fetch for the initial selection.
                    if icons_on {
                        if let Some(hit) = results.get(*selected) {
                            if let Some(url) = hit.icon_url.clone() {
                                effects.push(Effect::FetchIcon {
                                    source: crate::icons::IconSource::Modrinth,
                                    project_id: hit.project_id.clone(),
                                    url,
                                });
                            }
                        }
                    }
                }
            }
            effects
        }

        Action::ModBrowserSearchFailed { slug, message } => {
            if let ActiveView::ModBrowser {
                slug: cur_slug,
                fetch_state,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    *fetch_state = ModBrowserFetchState::Error(message);
                }
            }
            vec![]
        }

        Action::ModBrowserMove(delta) => {
            let icons_on = state.icon_rendering_enabled;
            let mut effects = Vec::new();
            if let ActiveView::ModBrowser {
                results, selected, ..
            } = &mut state.active_view
            {
                let len = results.len();
                if len > 0 {
                    let new_idx = (*selected as isize + delta).clamp(0, len as isize - 1) as usize;
                    *selected = new_idx;
                    // Phase 13: dispatch icon fetch for the new selection.
                    // The IconService LRU dedupes repeats, so revisiting an
                    // already-cached project is a no-op fetch (cache-hit
                    // disk read + LRU upsert).
                    if icons_on {
                        if let Some(hit) = results.get(new_idx) {
                            if let Some(url) = hit.icon_url.clone() {
                                effects.push(Effect::FetchIcon {
                                    source: crate::icons::IconSource::Modrinth,
                                    project_id: hit.project_id.clone(),
                                    url,
                                });
                            }
                        }
                    }
                }
            }
            effects
        }

        Action::ModBrowserOpenVersions => {
            // Capture (slug, project_id, project_title) from the highlighted row.
            let captured = match &state.active_view {
                ActiveView::ModBrowser {
                    slug,
                    results,
                    selected,
                    ..
                } => results
                    .get(*selected)
                    .map(|hit| (slug.clone(), hit.project_id.clone(), hit.title.clone())),
                _ => None,
            };
            let Some((slug, project_id, project_title)) = captured else {
                return vec![];
            };
            // Look up MC + loader from the instance for the version-list query.
            let (mc, loader) = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| (Some(m.mc_version_id.clone()), m.loader.clone()))
                .unwrap_or((None, None));
            // Transition to the version picker; rows are filled by ModVersionsLoaded.
            state.active_view = ActiveView::ModVersionPickerModal {
                slug: slug.clone(),
                project_id: project_id.clone(),
                project_title: project_title.clone(),
                versions: Vec::new(),
                selected: 0,
            };
            vec![Effect::ListModVersions {
                slug,
                project_id,
                project_title,
                mc,
                loader,
            }]
        }

        Action::ToggleModMcFilter => {
            // Cycle: None ↔ Some("any"). Re-emit search with the new filter.
            let captured = match &mut state.active_view {
                ActiveView::ModBrowser {
                    slug,
                    search,
                    mc_filter_override,
                    loader_filter_override,
                    ..
                } => {
                    *mc_filter_override = match mc_filter_override.as_deref() {
                        None => Some("any".to_string()),
                        Some(_) => None,
                    };
                    Some((
                        slug.clone(),
                        search.clone(),
                        mc_filter_override.clone(),
                        loader_filter_override.clone(),
                    ))
                }
                _ => None,
            };
            let Some((slug, query, mc_override, loader_override)) = captured else {
                return vec![];
            };
            // Resolve effective MC: override "any" -> no filter, else instance MC.
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = match loader_override.as_deref() {
                Some("any") => None,
                _ => inst.and_then(|m| m.loader.clone()),
            };
            vec![Effect::SearchModrinth {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::ToggleModLoaderFilter => {
            let captured = match &mut state.active_view {
                ActiveView::ModBrowser {
                    slug,
                    search,
                    mc_filter_override,
                    loader_filter_override,
                    ..
                } => {
                    *loader_filter_override = match loader_filter_override.as_deref() {
                        None => Some("any".to_string()),
                        Some(_) => None,
                    };
                    Some((
                        slug.clone(),
                        search.clone(),
                        mc_filter_override.clone(),
                        loader_filter_override.clone(),
                    ))
                }
                _ => None,
            };
            let Some((slug, query, mc_override, loader_override)) = captured else {
                return vec![];
            };
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = match loader_override.as_deref() {
                Some("any") => None,
                _ => inst.and_then(|m| m.loader.clone()),
            };
            vec![Effect::SearchModrinth {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::ModBrowserBackspaceSearch => {
            let captured = match &mut state.active_view {
                ActiveView::ModBrowser {
                    slug,
                    search,
                    mc_filter_override,
                    loader_filter_override,
                    ..
                } => {
                    search.pop();
                    Some((
                        slug.clone(),
                        search.clone(),
                        mc_filter_override.clone(),
                        loader_filter_override.clone(),
                    ))
                }
                _ => None,
            };
            let Some((slug, query, mc_override, loader_override)) = captured else {
                return vec![];
            };
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = match loader_override.as_deref() {
                Some("any") => None,
                _ => inst.and_then(|m| m.loader.clone()),
            };
            vec![Effect::SearchModrinth {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::ModBrowserCancel => {
            state.active_view = ActiveView::default();
            vec![]
        }

        Action::ModBrowserBeginSearch => {
            if let ActiveView::ModBrowser { is_searching, .. } = &mut state.active_view {
                *is_searching = true;
            }
            vec![]
        }

        Action::ModBrowserExitSearch => {
            // Exit search mode and clear the buffer. If the buffer had
            // content, re-emit a fresh search so results reflect the
            // empty query (matches ModBrowserBackspaceSearch's contract).
            let captured = match &mut state.active_view {
                ActiveView::ModBrowser {
                    slug,
                    search,
                    is_searching,
                    mc_filter_override,
                    loader_filter_override,
                    ..
                } => {
                    *is_searching = false;
                    let had_query = !search.is_empty();
                    search.clear();
                    if had_query {
                        Some((
                            slug.clone(),
                            mc_filter_override.clone(),
                            loader_filter_override.clone(),
                        ))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let Some((slug, mc_override, loader_override)) = captured else {
                return vec![];
            };
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = match loader_override.as_deref() {
                Some("any") => None,
                _ => inst.and_then(|m| m.loader.clone()),
            };
            vec![Effect::SearchModrinth {
                slug,
                query: String::new(),
                mc,
                loader,
            }]
        }

        Action::ModBrowserTypeSearch(c) => {
            // The j/k disambiguation lives in the keymap (08-08); the update arm
            // unconditionally appends. Re-emit the search with the new query.
            let captured = match &mut state.active_view {
                ActiveView::ModBrowser {
                    slug,
                    search,
                    mc_filter_override,
                    loader_filter_override,
                    ..
                } => {
                    search.push(c);
                    Some((
                        slug.clone(),
                        search.clone(),
                        mc_filter_override.clone(),
                        loader_filter_override.clone(),
                    ))
                }
                _ => None,
            };
            let Some((slug, query, mc_override, loader_override)) = captured else {
                return vec![];
            };
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = match loader_override.as_deref() {
                Some("any") => None,
                _ => inst.and_then(|m| m.loader.clone()),
            };
            vec![Effect::SearchModrinth {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::ModBrowserPasteSearch(s) => {
            // Bracketed-paste payload: append the full pasted string in one
            // dispatch (08.1-04 / GAP-8-C). Mirrors ModBrowserTypeSearch
            // including the search re-fire. Paste in browse mode also flips
            // `is_searching` so the user lands in search mode with the
            // pasted query already applied (no need to press `/` first).
            let captured = match &mut state.active_view {
                ActiveView::ModBrowser {
                    slug,
                    search,
                    is_searching,
                    mc_filter_override,
                    loader_filter_override,
                    ..
                } => {
                    *is_searching = true;
                    search.push_str(&s);
                    Some((
                        slug.clone(),
                        search.clone(),
                        mc_filter_override.clone(),
                        loader_filter_override.clone(),
                    ))
                }
                _ => None,
            };
            let Some((slug, query, mc_override, loader_override)) = captured else {
                return vec![];
            };
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = match loader_override.as_deref() {
                Some("any") => None,
                _ => inst.and_then(|m| m.loader.clone()),
            };
            vec![Effect::SearchModrinth {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::ModDetailLoaded { slug, detail } => {
            if let ActiveView::ModBrowser {
                slug: cur_slug,
                selected_detail,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    *selected_detail = Some(detail);
                }
            }
            vec![]
        }

        Action::ModVersionsLoaded { slug, versions } => {
            if let ActiveView::ModVersionPickerModal {
                slug: cur_slug,
                versions: ref mut v,
                selected,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    *v = versions;
                    *selected = 0;
                }
            }
            vec![]
        }

        Action::ModVersionPickerMove(delta) => {
            if let ActiveView::ModVersionPickerModal {
                versions, selected, ..
            } = &mut state.active_view
            {
                let len = versions.len();
                if len > 0 {
                    let new_idx = (*selected as isize + delta).clamp(0, len as isize - 1) as usize;
                    *selected = new_idx;
                }
            }
            vec![]
        }

        Action::ModVersionPickerSelect => {
            // Capture selected version, then emit ResolveModDependencies. Stay on
            // the version picker until ModDepsResolved arrives.
            let captured = match &state.active_view {
                ActiveView::ModVersionPickerModal {
                    slug,
                    project_id,
                    project_title,
                    versions,
                    selected,
                } => versions.get(*selected).map(|v| {
                    (
                        slug.clone(),
                        project_id.clone(),
                        project_title.clone(),
                        v.version_id.clone(),
                        v.version_label.clone(),
                    )
                }),
                _ => None,
            };
            let Some((slug, project_id, project_title, version_id, version_label)) = captured
            else {
                return vec![];
            };
            // mc + loader for the dep-resolve query come from the instance.
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = inst.map(|m| m.mc_version_id.clone()).unwrap_or_default();
            let loader = inst.and_then(|m| m.loader.clone());
            vec![Effect::ResolveModDependencies {
                slug,
                project_id,
                project_title,
                version_id,
                version_label,
                mc,
                loader,
            }]
        }

        Action::ModVersionPickerCancel => {
            // Return to ModBrowser preserving captured slug. Clear other context;
            // the browser will re-fetch via SearchModrinth on next user input.
            let captured = match &state.active_view {
                ActiveView::ModVersionPickerModal { slug, .. } => Some(slug.clone()),
                _ => None,
            };
            let Some(slug) = captured else {
                state.active_view = ActiveView::default();
                return vec![];
            };
            // Mirror Phase 6's LoaderVersionPickerCancel: re-open browser cleanly.
            // Issue OpenModBrowser semantics inline (without the Pitfall 8 guard,
            // since cancelling a version-pick means there's no install in flight).
            let (mc, loader) = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| (Some(m.mc_version_id.clone()), m.loader.clone()))
                .unwrap_or((None, None));
            state.active_view = ActiveView::ModBrowser {
                slug: slug.clone(),
                search: String::new(),
                is_searching: false,
                mc_filter_override: None,
                loader_filter_override: None,
                results: Vec::new(),
                selected: 0,
                fetch_state: ModBrowserFetchState::Loading,
                selected_detail: None,
            };
            vec![Effect::SearchModrinth {
                slug,
                query: String::new(),
                mc,
                loader,
            }]
        }

        Action::ModDepsResolved {
            slug,
            project_id,
            project_title,
            version_id,
            version_label,
            graph,
        } => {
            let total_bytes = graph.total_new_bytes;
            let total_files = graph.total_new_files;
            let has_conflict = graph
                .deps
                .iter()
                .any(|d| matches!(d.kind, DepKind::Incompatible));
            let deps: Vec<ResolvedDep> = graph.deps.clone();
            let root_version = Box::new(graph.root.clone());
            state.active_view = ActiveView::DepConfirmModal {
                slug,
                project_id,
                project_title,
                version_id,
                version_label,
                deps,
                total_bytes,
                total_files,
                has_conflict,
                root_version,
            };
            vec![]
        }

        Action::ConfirmModInstall => {
            let captured = match &state.active_view {
                ActiveView::DepConfirmModal {
                    slug,
                    project_id,
                    project_title,
                    has_conflict,
                    root_version,
                    deps: _,
                    ..
                } => {
                    if *has_conflict {
                        // T-08-07-04 mitigation: y is a no-op when has_conflict.
                        return vec![];
                    }
                    Some((
                        slug.clone(),
                        project_id.clone(),
                        project_title.clone(),
                        root_version.clone(),
                    ))
                }
                _ => None,
            };
            let Some((slug, _project_id, project_title, root_version)) = captured else {
                return vec![];
            };
            // Rebuild the graph from the modal's stored fields. We only need
            // root + deps + totals for the installer; capture those by cloning.
            let (deps, total_new_bytes, total_new_files) = match &state.active_view {
                ActiveView::DepConfirmModal {
                    deps,
                    total_bytes,
                    total_files,
                    ..
                } => (deps.clone(), *total_bytes, *total_files),
                _ => (Vec::new(), 0u64, 0usize),
            };
            let graph = Box::new(ResolvedDepGraph {
                root: (*root_version).clone(),
                deps,
                total_new_bytes,
                total_new_files,
            });
            // project_slug is NOT stored on the modal; the InstallModWithDeps
            // effect arm in 08-08 will look it up from the root_version.
            // We pass the project_id-as-slug placeholder; 08-08 will resolve.
            let project_slug = root_version.project_id.clone();
            // Stay on the modal until ModInstallStarted arrives (then 08-08
            // transitions to ModBrowser per UI-SPEC §11 background install).
            vec![Effect::InstallModWithDeps {
                slug,
                project_slug,
                project_title,
                root_version,
                graph,
            }]
        }

        Action::CancelModInstall => {
            // Per UI-SPEC line 597 -- return to ModVersionPickerModal preserving
            // slug/project context. We do not have the original versions list cached
            // here; transition to the picker with empty versions and selected=0.
            // (08-08 may choose to re-fetch via ListModVersions; the spec only
            // requires the user lands back on the version picker.)
            let captured = match &state.active_view {
                ActiveView::DepConfirmModal {
                    slug,
                    project_id,
                    project_title,
                    ..
                } => Some((slug.clone(), project_id.clone(), project_title.clone())),
                _ => None,
            };
            let Some((slug, project_id, project_title)) = captured else {
                state.active_view = ActiveView::default();
                return vec![];
            };
            state.active_view = ActiveView::ModVersionPickerModal {
                slug: slug.clone(),
                project_id: project_id.clone(),
                project_title: project_title.clone(),
                versions: Vec::new(),
                selected: 0,
            };
            // mc/loader from the instance for the re-fetch.
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = inst.map(|m| m.mc_version_id.clone());
            let loader = inst.and_then(|m| m.loader.clone());
            vec![Effect::ListModVersions {
                slug,
                project_id,
                project_title,
                mc,
                loader,
            }]
        }

        Action::ModInstallStarted {
            slug,
            project_id: _,
            token,
        } => {
            state.running_mod_jobs.insert(slug.clone(), token);
            // Transition active_view back to ModBrowser so the user can browse
            // more while the install runs in the background (UI-SPEC §11).
            // mc/loader from the instance.
            let (mc, loader) = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| (Some(m.mc_version_id.clone()), m.loader.clone()))
                .unwrap_or((None, None));
            state.active_view = ActiveView::ModBrowser {
                slug: slug.clone(),
                search: String::new(),
                is_searching: false,
                mc_filter_override: None,
                loader_filter_override: None,
                results: Vec::new(),
                selected: 0,
                fetch_state: ModBrowserFetchState::Loading,
                selected_detail: None,
            };
            vec![Effect::SearchModrinth {
                slug,
                query: String::new(),
                mc,
                loader,
            }]
        }

        Action::ModInstalled { slug, project_id } => {
            state.running_mod_jobs.remove(&slug);
            // Update authoritative installed-set first so any subsequent
            // ModBrowserSearchLoaded (e.g. user re-searches) sees the new
            // membership. Closes the install/search race that previously
            // left "Install" stale until another keystroke triggered a
            // ledger-fresh search.
            state
                .installed_mod_project_ids
                .entry(slug.clone())
                .or_default()
                .insert(project_id.clone());
            // Pitfall 10 (08-RESEARCH.md §Pitfall 10): if the user is still in
            // the ModBrowser for this instance, walk results and stamp
            // already_installed = true on every hit matching project_id.
            if let ActiveView::ModBrowser {
                slug: cur_slug,
                results,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    for r in results.iter_mut() {
                        if r.project_id == project_id {
                            r.already_installed = true;
                        }
                    }
                }
            }
            vec![]
        }

        Action::ModInstallFailed {
            slug,
            mod_title,
            version_label,
            error,
            log_tail,
        } => {
            state.running_mod_jobs.remove(&slug);
            // Compute return_to from current active_view (per UI-SPEC line 626).
            let return_to = match &state.active_view {
                ActiveView::ModBrowser { .. } => ModInstallFailedReturnTo::ModBrowser,
                _ => ModInstallFailedReturnTo::InstalledModsList,
            };
            state.active_view = ActiveView::ModInstallFailedModal {
                slug,
                mod_title,
                version_label,
                error,
                log_tail,
                return_to,
            };
            vec![]
        }

        Action::DismissModInstallFailed => {
            let captured = match &state.active_view {
                ActiveView::ModInstallFailedModal {
                    slug, return_to, ..
                } => Some((slug.clone(), *return_to)),
                _ => None,
            };
            match captured {
                Some((slug, ModInstallFailedReturnTo::ModBrowser)) => {
                    let (mc, loader) = state
                        .instances
                        .iter()
                        .find(|m| m.slug == slug)
                        .map(|m| (Some(m.mc_version_id.clone()), m.loader.clone()))
                        .unwrap_or((None, None));
                    state.active_view = ActiveView::ModBrowser {
                        slug: slug.clone(),
                        search: String::new(),
                        is_searching: false,
                        mc_filter_override: None,
                        loader_filter_override: None,
                        results: Vec::new(),
                        selected: 0,
                        fetch_state: ModBrowserFetchState::Loading,
                        selected_detail: None,
                    };
                    vec![Effect::SearchModrinth {
                        slug,
                        query: String::new(),
                        mc,
                        loader,
                    }]
                }
                Some((slug, ModInstallFailedReturnTo::InstalledModsList)) => {
                    state.active_view = ActiveView::InstalledModsList {
                        slug: slug.clone(),
                        mods: Vec::new(),
                        selected: 0,
                    };
                    vec![Effect::FetchInstalledMods { slug }]
                }
                None => {
                    state.active_view = ActiveView::default();
                    vec![]
                }
            }
        }

        Action::OpenInstalledMods { slug } => {
            state.active_view = ActiveView::InstalledModsList {
                slug: slug.clone(),
                mods: Vec::new(),
                selected: 0,
            };
            vec![Effect::FetchInstalledMods { slug }]
        }

        Action::InstalledModsLoaded { slug, mods } => {
            // Refresh the authoritative installed-set from the ledger before
            // mutating active_view. Used by ModBrowserSearchLoaded to stamp
            // `already_installed` immune to install/search race.
            state.installed_mod_project_ids.insert(
                slug.clone(),
                mods.iter().map(|r| r.mod_id.clone()).collect(),
            );
            // If the user is currently in the ModBrowser for this instance,
            // re-stamp existing results from the freshly-loaded set so a late
            // FetchInstalledMods response after SearchLoaded still updates
            // visible flags.
            let installed = state.installed_mod_project_ids.get(&slug).cloned();
            if let ActiveView::ModBrowser {
                slug: cur_slug,
                results,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    if let Some(set) = installed.as_ref() {
                        for r in results.iter_mut() {
                            if set.contains(&r.project_id) {
                                r.already_installed = true;
                            }
                        }
                    }
                }
            }
            if let ActiveView::InstalledModsList {
                slug: cur_slug,
                mods: ref mut m,
                selected,
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    let new_len = mods.len();
                    *m = mods;
                    *selected = (*selected).min(new_len.saturating_sub(1));
                }
            }
            vec![]
        }

        Action::InstalledModsMove(delta) => {
            if let ActiveView::InstalledModsList { mods, selected, .. } = &mut state.active_view {
                let len = mods.len();
                if len > 0 {
                    let new_idx = (*selected as isize + delta).clamp(0, len as isize - 1) as usize;
                    *selected = new_idx;
                }
            }
            vec![]
        }

        Action::ToggleModEnabled => {
            let captured = match &state.active_view {
                ActiveView::InstalledModsList {
                    slug,
                    mods,
                    selected,
                } => mods
                    .get(*selected)
                    .map(|row| (slug.clone(), row.mod_id.clone(), !row.enabled)),
                _ => None,
            };
            let Some((slug, mod_id, want_enabled)) = captured else {
                return vec![];
            };
            vec![Effect::ToggleModEnabledEff {
                slug,
                mod_id,
                want_enabled,
            }]
        }

        Action::ModToggled {
            slug,
            mod_id,
            enabled,
        } => {
            if let ActiveView::InstalledModsList {
                slug: cur_slug,
                mods,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    if let Some(row) = mods.iter_mut().find(|r| r.mod_id == mod_id) {
                        row.enabled = enabled;
                    }
                }
            }
            vec![]
        }

        Action::OpenUninstallModConfirm => {
            let captured = match &state.active_view {
                ActiveView::InstalledModsList {
                    slug,
                    mods,
                    selected,
                } => mods
                    .get(*selected)
                    .map(|row| (slug.clone(), row.mod_id.clone(), row.display_name.clone())),
                _ => None,
            };
            let Some((slug, mod_id, display_name)) = captured else {
                return vec![];
            };
            state.active_view = ActiveView::UninstallModConfirm {
                slug,
                mod_id,
                display_name,
            };
            vec![]
        }

        Action::ConfirmUninstallMod => {
            let captured = match &state.active_view {
                ActiveView::UninstallModConfirm { slug, mod_id, .. } => {
                    Some((slug.clone(), mod_id.clone()))
                }
                _ => None,
            };
            let Some((slug, mod_id)) = captured else {
                return vec![];
            };
            // Responsive UX: return to InstalledModsList immediately; the row will
            // be removed by Action::ModUninstalled when the effect completes.
            state.active_view = ActiveView::InstalledModsList {
                slug: slug.clone(),
                mods: Vec::new(),
                selected: 0,
            };
            vec![
                Effect::UninstallMod {
                    slug: slug.clone(),
                    mod_id,
                },
                Effect::FetchInstalledMods { slug },
            ]
        }

        Action::CancelUninstallMod => {
            let captured = match &state.active_view {
                ActiveView::UninstallModConfirm { slug, .. } => Some(slug.clone()),
                _ => None,
            };
            let Some(slug) = captured else {
                state.active_view = ActiveView::default();
                return vec![];
            };
            state.active_view = ActiveView::InstalledModsList {
                slug: slug.clone(),
                mods: Vec::new(),
                selected: 0,
            };
            vec![Effect::FetchInstalledMods { slug }]
        }

        Action::ModUninstalled { slug, mod_id } => {
            // Drop from authoritative installed-set so a subsequent
            // ModBrowser search reflects the uninstall immediately.
            if let Some(set) = state.installed_mod_project_ids.get_mut(&slug) {
                set.remove(&mod_id);
            }
            // Also clear the cached `already_installed` flag on any open
            // ModBrowser results for this instance (mirrors the optimistic
            // stamp performed by ModInstalled).
            if let ActiveView::ModBrowser {
                slug: cur_slug,
                results,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    for r in results.iter_mut() {
                        if r.project_id == mod_id {
                            r.already_installed = false;
                        }
                    }
                }
            }
            if let ActiveView::InstalledModsList {
                slug: cur_slug,
                mods,
                selected,
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    mods.retain(|r| r.mod_id != mod_id);
                    let new_len = mods.len();
                    *selected = (*selected).min(new_len.saturating_sub(1));
                }
            }
            vec![]
        }

        Action::CloseInstalledMods => {
            state.active_view = ActiveView::default();
            vec![]
        }

        Action::StopInstance { slug } => {
            // Cancel and clear immediately so the running badge disappears on the
            // next render. The async launch task will later dispatch
            // Action::InstanceExited; the arm is idempotent (remove of an absent
            // slug is a no-op), so the double-path is safe.
            if let Some(token) = state.running_instances.remove(&slug) {
                token.cancel();
            }
            vec![Effect::KillProcess { slug }]
        }

        // ── Phase 9 (CurseForge Integration) -- pure update() arms ──
        // All arms below mutate AppState and (optionally) emit Effects. NO arm
        // performs HTTP, file I/O, or task spawning -- those live in 09-07 run.rs.
        // Mirrors Phase 8's modrinth arms 1:1 (CfBrowser ≡ ModBrowser, etc.).
        Action::OpenCfBrowser { slug } => {
            // Pitfall 1 (09-RESEARCH.md §"Keybind guard"): F is silently disabled
            // when no CurseForge API key was resolved at startup. Tested by
            // `test_cf_open_no_op_when_api_key_absent`.
            if !state.cf_api_key_present {
                return vec![];
            }
            // Pitfall 8 inheritance from Phase 8 (running_mod_jobs is the
            // source-agnostic per-instance install lock).
            if state.running_mod_jobs.contains_key(&slug) {
                return vec![];
            }
            let (mc, loader) = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| {
                    (
                        Some(m.mc_version_id.clone()),
                        crate::mods::curseforge::filter::curseforge_loader_type(m.loader.as_ref()),
                    )
                })
                .unwrap_or((None, None));
            state.active_view = ActiveView::CfBrowser {
                slug: slug.clone(),
                search_input: String::new(),
                results: Vec::new(),
                selected: 0,
                fetch_state: ModBrowserFetchState::Loading,
                mc_filter: None,
                loader_filter: None,
                selected_detail: None,
            };
            vec![Effect::SearchCurseForge {
                slug,
                query: String::new(),
                mc,
                loader,
            }]
        }

        Action::CfBrowserSearchStart {
            slug,
            query,
            mc,
            loader,
        } => {
            if let ActiveView::CfBrowser {
                slug: cur_slug,
                fetch_state,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    *fetch_state = ModBrowserFetchState::Loading;
                }
            }
            vec![Effect::SearchCurseForge {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::CfBrowserSearchLoaded { slug, hits } => {
            if let ActiveView::CfBrowser {
                slug: cur_slug,
                results,
                selected,
                fetch_state,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    let new_len = hits.len();
                    *results = hits;
                    *fetch_state = ModBrowserFetchState::Ready;
                    *selected = (*selected).min(new_len.saturating_sub(1));
                }
            }
            vec![]
        }

        Action::CfBrowserSearchFailed { slug, error } => {
            if let ActiveView::CfBrowser {
                slug: cur_slug,
                fetch_state,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    *fetch_state = ModBrowserFetchState::Error(error);
                }
            }
            vec![]
        }

        Action::CfBrowserMoveSelection(delta) => {
            if let ActiveView::CfBrowser {
                results, selected, ..
            } = &mut state.active_view
            {
                let len = results.len();
                if len > 0 {
                    let new_idx = (*selected as i32 + delta).clamp(0, len as i32 - 1) as usize;
                    *selected = new_idx;
                }
            }
            vec![]
        }

        Action::CfBrowserToggleMcFilter => {
            // Cycle: None ↔ Some("any"). Re-emit search with the new filter.
            let captured = match &mut state.active_view {
                ActiveView::CfBrowser {
                    slug,
                    search_input,
                    mc_filter,
                    loader_filter,
                    ..
                } => {
                    *mc_filter = match mc_filter.as_deref() {
                        None => Some("any".to_string()),
                        Some(_) => None,
                    };
                    Some((
                        slug.clone(),
                        search_input.clone(),
                        mc_filter.clone(),
                        *loader_filter,
                    ))
                }
                _ => None,
            };
            let Some((slug, query, mc_override, loader_override)) = captured else {
                return vec![];
            };
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = match loader_override {
                None => inst.and_then(|m| {
                    crate::mods::curseforge::filter::curseforge_loader_type(m.loader.as_ref())
                }),
                Some(v) => Some(v),
            };
            vec![Effect::SearchCurseForge {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::CfBrowserToggleLoaderFilter => {
            // Cycle: None ↔ Some(<instance loader>). Re-emit search.
            let captured = match &mut state.active_view {
                ActiveView::CfBrowser {
                    slug,
                    search_input,
                    mc_filter,
                    loader_filter,
                    ..
                } => {
                    // Look up the instance loader integer to use as the toggle target.
                    let inst_loader =
                        state
                            .instances
                            .iter()
                            .find(|m| m.slug == *slug)
                            .and_then(|m| {
                                crate::mods::curseforge::filter::curseforge_loader_type(
                                    m.loader.as_ref(),
                                )
                            });
                    *loader_filter = match *loader_filter {
                        None => inst_loader,
                        Some(_) => None,
                    };
                    Some((
                        slug.clone(),
                        search_input.clone(),
                        mc_filter.clone(),
                        *loader_filter,
                    ))
                }
                _ => None,
            };
            let Some((slug, query, mc_override, loader_override)) = captured else {
                return vec![];
            };
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = loader_override;
            vec![Effect::SearchCurseForge {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::CfBrowserOpenDetail { slug, mod_id } => {
            // Action ping-pong half 1 (mirrors Phase 8 ModBrowserOpenVersions →
            // FetchModDetail). Detail arrival triggers the chained ListCfFiles.
            vec![Effect::FetchCfMod { slug, mod_id }]
        }

        Action::CfBrowserDetailLoaded { slug, detail } => {
            // Action ping-pong half 2 (mirrors Phase 8 ModDetailLoaded → ModVersionsLoaded
            // chain): cache the detail on CfBrowser.selected_detail AND emit
            // Effect::ListCfFiles to populate the file picker.
            let mod_id = detail.id;
            // Capture mc/loader from the CfBrowser view BEFORE mutating it.
            let (mc, loader) = match &state.active_view {
                ActiveView::CfBrowser {
                    mc_filter,
                    loader_filter,
                    ..
                } => {
                    let inst = state.instances.iter().find(|m| m.slug == slug);
                    let mc = match mc_filter.as_deref() {
                        Some("any") => None,
                        _ => inst.map(|m| m.mc_version_id.clone()),
                    };
                    let loader = match *loader_filter {
                        None => inst.and_then(|m| {
                            crate::mods::curseforge::filter::curseforge_loader_type(
                                m.loader.as_ref(),
                            )
                        }),
                        Some(v) => Some(v),
                    };
                    (mc, loader)
                }
                _ => (None, None),
            };
            if let ActiveView::CfBrowser {
                slug: cur_slug,
                selected_detail,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug {
                    *selected_detail = Some(detail);
                }
            }
            vec![Effect::ListCfFiles {
                slug,
                mod_id,
                mc,
                loader,
            }]
        }

        Action::CfBrowserTypeSearch(c) => {
            let captured = match &mut state.active_view {
                ActiveView::CfBrowser {
                    slug,
                    search_input,
                    mc_filter,
                    loader_filter,
                    ..
                } => {
                    search_input.push(c);
                    Some((
                        slug.clone(),
                        search_input.clone(),
                        mc_filter.clone(),
                        *loader_filter,
                    ))
                }
                _ => None,
            };
            let Some((slug, query, mc_override, loader_override)) = captured else {
                return vec![];
            };
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = match loader_override {
                None => inst.and_then(|m| {
                    crate::mods::curseforge::filter::curseforge_loader_type(m.loader.as_ref())
                }),
                Some(v) => Some(v),
            };
            vec![Effect::SearchCurseForge {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::CfBrowserPasteSearch(s) => {
            // Bracketed-paste payload: append the full pasted string in one
            // dispatch (08.1-04 / GAP-8-C). Mirrors CfBrowserTypeSearch
            // including the search re-fire.
            let captured = match &mut state.active_view {
                ActiveView::CfBrowser {
                    slug,
                    search_input,
                    mc_filter,
                    loader_filter,
                    ..
                } => {
                    search_input.push_str(&s);
                    Some((
                        slug.clone(),
                        search_input.clone(),
                        mc_filter.clone(),
                        *loader_filter,
                    ))
                }
                _ => None,
            };
            let Some((slug, query, mc_override, loader_override)) = captured else {
                return vec![];
            };
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = match loader_override {
                None => inst.and_then(|m| {
                    crate::mods::curseforge::filter::curseforge_loader_type(m.loader.as_ref())
                }),
                Some(v) => Some(v),
            };
            vec![Effect::SearchCurseForge {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::CfBrowserBackspaceSearch => {
            let captured = match &mut state.active_view {
                ActiveView::CfBrowser {
                    slug,
                    search_input,
                    mc_filter,
                    loader_filter,
                    ..
                } => {
                    search_input.pop();
                    Some((
                        slug.clone(),
                        search_input.clone(),
                        mc_filter.clone(),
                        *loader_filter,
                    ))
                }
                _ => None,
            };
            let Some((slug, query, mc_override, loader_override)) = captured else {
                return vec![];
            };
            let inst = state.instances.iter().find(|m| m.slug == slug);
            let mc = match mc_override.as_deref() {
                Some("any") => None,
                _ => inst.map(|m| m.mc_version_id.clone()),
            };
            let loader = match loader_override {
                None => inst.and_then(|m| {
                    crate::mods::curseforge::filter::curseforge_loader_type(m.loader.as_ref())
                }),
                Some(v) => Some(v),
            };
            vec![Effect::SearchCurseForge {
                slug,
                query,
                mc,
                loader,
            }]
        }

        Action::CfFilePickerLoaded {
            slug,
            mod_detail,
            files,
        } => {
            state.active_view = ActiveView::CfFilePickerModal {
                slug,
                mod_detail,
                files,
                selected: 0,
            };
            vec![]
        }

        Action::CfFilePickerMove(delta) => {
            if let ActiveView::CfFilePickerModal {
                files, selected, ..
            } = &mut state.active_view
            {
                let len = files.len();
                if len > 0 {
                    let new_idx = (*selected as i32 + delta).clamp(0, len as i32 - 1) as usize;
                    *selected = new_idx;
                }
            }
            vec![]
        }

        Action::CfFilePickerConfirm => {
            // Pitfall 8 guard: silent no-op when an install is already in flight
            // on this instance (running_mod_jobs is shared with Phase 8).
            let captured = match &state.active_view {
                ActiveView::CfFilePickerModal {
                    slug,
                    mod_detail,
                    files,
                    selected,
                } => {
                    if state.running_mod_jobs.contains_key(slug) {
                        return vec![];
                    }
                    files
                        .get(*selected)
                        .map(|f| (slug.clone(), mod_detail.clone(), f.clone()))
                }
                _ => None,
            };
            let Some((slug, mod_detail, file)) = captured else {
                return vec![];
            };
            vec![Effect::InstallCfMod {
                slug,
                mod_detail: Box::new(mod_detail),
                file: Box::new(file),
            }]
        }

        Action::CfModInstallStarted {
            slug,
            mod_id: _,
            file_id: _,
            token,
        } => {
            // Single-mutation-point invariant for running_mod_jobs (insert site).
            state.running_mod_jobs.insert(slug, token);
            vec![]
        }

        Action::CfModInstalled { slug, mod_id: _ } => {
            // Single-mutation-point invariant for running_mod_jobs (remove site #1).
            state.running_mod_jobs.remove(&slug);
            // If user is still on the file picker for this slug, return to the
            // instance list (matches Phase 8 install-completed UX).
            if let ActiveView::CfFilePickerModal { slug: cur_slug, .. } = &state.active_view {
                if *cur_slug == slug {
                    state.active_view = ActiveView::default();
                }
            }
            vec![]
        }

        Action::CfModInstallFailed {
            slug,
            mod_title,
            file_label,
            error,
            web_url,
        } => {
            // Single-mutation-point invariant for running_mod_jobs (remove site #2).
            state.running_mod_jobs.remove(&slug);
            state.active_view = ActiveView::CfInstallFailedModal {
                slug,
                mod_title,
                file_label,
                error_message: error,
                web_url,
            };
            vec![]
        }

        Action::CfDismissInstallFailed => {
            state.active_view = ActiveView::default();
            vec![]
        }

        // ── Phase 10 (Modpack Import) ──
        Action::OpenModpackImport => {
            state.active_view = ActiveView::ModpackImportPathInput {
                buffer: String::new(),
                error: None,
            };
            vec![]
        }

        Action::ImportPathTypeSearch(c) => {
            if let ActiveView::ModpackImportPathInput { buffer, error } = &mut state.active_view {
                buffer.push(c);
                *error = None;
            }
            vec![]
        }

        Action::ImportPathPasteSearch(s) => {
            if let ActiveView::ModpackImportPathInput { buffer, error } = &mut state.active_view {
                buffer.push_str(&s);
                *error = None;
            }
            vec![]
        }

        Action::ImportPathBackspaceSearch => {
            if let ActiveView::ModpackImportPathInput { buffer, .. } = &mut state.active_view {
                buffer.pop();
            }
            vec![]
        }

        Action::ImportPathSubmit => {
            if let ActiveView::ModpackImportPathInput { buffer, error } = &mut state.active_view {
                let trimmed = buffer.trim().to_string();
                if trimmed.is_empty() {
                    *error = Some("path required".into());
                    return vec![];
                }
                // Expand leading ~/ to the user's home directory.
                // Use directories::BaseDirs (cross-platform) rather than the
                // deprecated std::env::home_dir().
                let expanded = if let Some(rest) = trimmed.strip_prefix("~/") {
                    if let Some(home) =
                        directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf())
                    {
                        home.join(rest)
                    } else {
                        std::path::PathBuf::from(&trimmed)
                    }
                } else {
                    std::path::PathBuf::from(&trimmed)
                };
                return vec![Effect::ImportModpack {
                    mrpack_path: expanded,
                }];
            }
            vec![]
        }

        Action::ImportPathCancel => {
            state.active_view = ActiveView::InstanceList { selected: 0 };
            vec![]
        }

        Action::ModpackImportStarted {
            slug,
            modpack_name,
            token,
        } => {
            state.running_modpack_imports.insert(slug.clone(), token);
            state.active_view = ActiveView::ModpackImportProgressModal {
                modpack_name,
                step_label: "Starting".into(),
                step_index: 0,
                step_total: 7,
                bytes_done: 0,
                bytes_total: 0,
                cancel_token_key: slug,
                log_tail: String::new(),
            };
            vec![]
        }

        Action::ModpackImportProgress {
            slug: _,
            pct,
            step_label,
            bytes_done,
            bytes_total,
        } => {
            if let ActiveView::ModpackImportProgressModal {
                step_label: ref mut sl,
                bytes_done: ref mut bd,
                bytes_total: ref mut bt,
                step_index: ref mut si,
                ..
            } = &mut state.active_view
            {
                *sl = step_label;
                *bd = bytes_done;
                *bt = bytes_total;
                *si = match pct {
                    0..=14 => 0,
                    15..=24 => 1,
                    25..=39 => 2,
                    40..=59 => 3,
                    60..=74 => 4,
                    75..=84 => 5,
                    85..=94 => 6,
                    _ => 7,
                };
            }
            vec![]
        }

        Action::ModpackImported { slug } => {
            state.running_modpack_imports.remove(&slug);
            state.active_view = ActiveView::InstanceList { selected: 0 };
            vec![Effect::FetchInstances]
        }

        Action::ModpackImportCancelled => {
            // Dedicated arm: clear() the map regardless of whether a slug was
            // assigned before cancel was observed. Covers two cases:
            // (a) user pressed Esc on progress modal (CancelModpackImport already
            //     drained the map → clear() is idempotent)
            // (b) cancel was raised by the task system before a slug was assigned
            //     (the spawned task sends ModpackImportCancelled before any slug
            //     is stored → map still empty → clear() is idempotent).
            // This is the HIGH-2 regression fix: dispatching ModpackImported{slug:""}
            // would call remove("") which is a no-op against any real-slug key.
            state.running_modpack_imports.clear();
            state.active_view = ActiveView::InstanceList { selected: 0 };
            vec![]
        }

        Action::ModpackImportFailed {
            modpack_name,
            error,
            log_tail,
        } => {
            // Single-entry in v1: clearing all is correct regardless of slug state.
            state.running_modpack_imports.clear();
            state.active_view = ActiveView::ModpackImportFailedModal {
                modpack_name,
                error,
                log_tail,
            };
            vec![]
        }

        Action::CancelModpackImport => {
            // Drain running_modpack_imports and cancel each token.
            for (_slug, token) in state.running_modpack_imports.drain() {
                token.cancel();
            }
            state.active_view = ActiveView::InstanceList { selected: 0 };
            vec![Effect::CancelModpackImport]
        }

        Action::DismissModpackImportFailed => {
            state.active_view = ActiveView::InstanceList { selected: 0 };
            vec![]
        }

        // ── Phase 11 (11-04): Pack browser + installed list + drop-path ───────
        Action::OpenPackBrowser { slug, kind } => {
            let mc = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| m.mc_version_id.clone());
            state.active_view = ActiveView::PackBrowser {
                slug: slug.clone(),
                kind,
                search: String::new(),
                is_searching: false,
                fetch_state: ModBrowserFetchState::Loading,
                results: Vec::new(),
                selected: 0,
            };
            vec![
                Effect::SearchPacks {
                    slug: slug.clone(),
                    kind,
                    query: String::new(),
                    mc,
                },
                Effect::FetchInstalledPacks { slug, kind },
            ]
        }

        Action::PackBrowserSearchLoaded { slug, kind, hits } => {
            // Snapshot installed-set before mutably borrowing active_view.
            let installed = state
                .installed_pack_project_ids
                .get(&(slug.clone(), kind))
                .cloned();
            // Slug-AND-kind match guard (mirrors Phase 08.1-05 ModBrowserSearchLoaded).
            if let ActiveView::PackBrowser {
                slug: cur_slug,
                kind: cur_kind,
                results,
                selected,
                fetch_state,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug && *cur_kind == kind {
                    let new_len = hits.len();
                    *results = hits;
                    // Re-stamp `already_installed` from the in-memory set
                    // (see ModBrowserSearchLoaded; same install/search race).
                    if let Some(set) = installed.as_ref() {
                        for r in results.iter_mut() {
                            if set.contains(&r.project_id) {
                                r.already_installed = true;
                            }
                        }
                    }
                    *fetch_state = ModBrowserFetchState::Ready;
                    *selected = (*selected).min(new_len.saturating_sub(1));
                }
            }
            vec![]
        }

        Action::PackBrowserSearchFailed {
            slug,
            kind,
            message,
        } => {
            if let ActiveView::PackBrowser {
                slug: cur_slug,
                kind: cur_kind,
                fetch_state,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug && *cur_kind == kind {
                    *fetch_state = ModBrowserFetchState::Error(message);
                }
            }
            vec![]
        }

        Action::PackBrowserMove(delta) => {
            if let ActiveView::PackBrowser {
                results, selected, ..
            } = &mut state.active_view
            {
                let len = results.len();
                if len > 0 {
                    let new_idx =
                        (*selected as isize + delta as isize).clamp(0, len as isize - 1) as usize;
                    *selected = new_idx;
                }
            }
            vec![]
        }

        Action::PackBrowserTypeSearch(c) => {
            if let ActiveView::PackBrowser {
                slug,
                kind,
                search,
                fetch_state,
                ..
            } = &mut state.active_view
            {
                search.push(c);
                *fetch_state = ModBrowserFetchState::Loading;
                let slug2 = slug.clone();
                let kind2 = *kind;
                let query = search.clone();
                let mc = state
                    .instances
                    .iter()
                    .find(|m| m.slug == slug2)
                    .map(|m| m.mc_version_id.clone());
                return vec![Effect::SearchPacks {
                    slug: slug2,
                    kind: kind2,
                    query,
                    mc,
                }];
            }
            vec![]
        }

        Action::PackBrowserBackspaceSearch => {
            if let ActiveView::PackBrowser {
                slug,
                kind,
                search,
                fetch_state,
                ..
            } = &mut state.active_view
            {
                search.pop();
                *fetch_state = ModBrowserFetchState::Loading;
                let slug2 = slug.clone();
                let kind2 = *kind;
                let query = search.clone();
                let mc = state
                    .instances
                    .iter()
                    .find(|m| m.slug == slug2)
                    .map(|m| m.mc_version_id.clone());
                return vec![Effect::SearchPacks {
                    slug: slug2,
                    kind: kind2,
                    query,
                    mc,
                }];
            }
            vec![]
        }

        Action::PackBrowserPasteSearch(s) => {
            if let ActiveView::PackBrowser {
                slug,
                kind,
                search,
                is_searching,
                fetch_state,
                ..
            } = &mut state.active_view
            {
                // Paste in browse mode auto-enters search mode (mirrors
                // ModBrowserPasteSearch).
                *is_searching = true;
                search.push_str(&s);
                *fetch_state = ModBrowserFetchState::Loading;
                let slug2 = slug.clone();
                let kind2 = *kind;
                let query = search.clone();
                let mc = state
                    .instances
                    .iter()
                    .find(|m| m.slug == slug2)
                    .map(|m| m.mc_version_id.clone());
                return vec![Effect::SearchPacks {
                    slug: slug2,
                    kind: kind2,
                    query,
                    mc,
                }];
            }
            vec![]
        }

        Action::PackBrowserClose => {
            state.active_view = ActiveView::InstanceList { selected: 0 };
            vec![]
        }

        Action::PackBrowserBeginSearch => {
            if let ActiveView::PackBrowser { is_searching, .. } = &mut state.active_view {
                *is_searching = true;
            }
            vec![]
        }

        Action::PackBrowserExitSearch => {
            // Mirrors ModBrowserExitSearch: drop search-mode flag, clear
            // buffer, re-emit search if the buffer had content.
            let captured = match &mut state.active_view {
                ActiveView::PackBrowser {
                    slug,
                    kind,
                    search,
                    is_searching,
                    ..
                } => {
                    *is_searching = false;
                    let had_query = !search.is_empty();
                    search.clear();
                    if had_query {
                        Some((slug.clone(), *kind))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let Some((slug, kind)) = captured else {
                return vec![];
            };
            let mc = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| m.mc_version_id.clone());
            vec![Effect::SearchPacks {
                slug,
                kind,
                query: String::new(),
                mc,
            }]
        }

        Action::PackDropPathOpen { slug, kind } => {
            state.active_view = ActiveView::PackDropPathInput {
                slug,
                kind,
                buffer: String::new(),
                error: None,
            };
            vec![]
        }

        Action::PackDropPathType(c) => {
            if let ActiveView::PackDropPathInput { buffer, error, .. } = &mut state.active_view {
                buffer.push(c);
                *error = None;
            }
            vec![]
        }

        Action::PackDropPathBackspace => {
            if let ActiveView::PackDropPathInput { buffer, .. } = &mut state.active_view {
                buffer.pop();
            }
            vec![]
        }

        Action::PackDropPathPaste(s) => {
            if let ActiveView::PackDropPathInput { buffer, error, .. } = &mut state.active_view {
                buffer.push_str(&s);
                *error = None;
            }
            vec![]
        }

        Action::PackDropPathSubmit => {
            let captured = match &state.active_view {
                ActiveView::PackDropPathInput {
                    slug, kind, buffer, ..
                } => {
                    if buffer.is_empty() {
                        None // will set error below
                    } else {
                        Some((
                            slug.clone(),
                            *kind,
                            std::path::PathBuf::from(buffer.clone()),
                        ))
                    }
                }
                _ => return vec![],
            };
            if let Some((slug, kind, path)) = captured {
                // Transition back to browser while install runs (responsive UX).
                state.active_view = ActiveView::PackBrowser {
                    slug: slug.clone(),
                    kind,
                    search: String::new(),
                    is_searching: false,
                    fetch_state: ModBrowserFetchState::Loading,
                    results: Vec::new(),
                    selected: 0,
                };
                vec![Effect::DropInstallPack { slug, kind, path }]
            } else {
                // Empty buffer -- set error message.
                if let ActiveView::PackDropPathInput { error, .. } = &mut state.active_view {
                    *error = Some("Path cannot be empty".to_string());
                }
                vec![]
            }
        }

        Action::PackDropPathCancel => {
            let captured = match &state.active_view {
                ActiveView::PackDropPathInput { slug, kind, .. } => Some((slug.clone(), *kind)),
                _ => None,
            };
            let (slug, kind) = captured.unwrap_or_else(|| (String::new(), PackKind::Resource));
            let mc = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| m.mc_version_id.clone());
            state.active_view = ActiveView::PackBrowser {
                slug: slug.clone(),
                kind,
                search: String::new(),
                is_searching: false,
                fetch_state: ModBrowserFetchState::Loading,
                results: Vec::new(),
                selected: 0,
            };
            vec![Effect::SearchPacks {
                slug,
                kind,
                query: String::new(),
                mc,
            }]
        }

        Action::InstalledPacksLoaded { slug, kind, packs } => {
            // Refresh authoritative installed-set from ledger (parallels
            // InstalledModsLoaded). Used by PackBrowserSearchLoaded to stamp
            // `already_installed` immune to install/search race.
            state.installed_pack_project_ids.insert(
                (slug.clone(), kind),
                packs.iter().map(|r| r.mod_id.clone()).collect(),
            );
            // If the user is currently in the PackBrowser for this
            // (instance, kind), re-stamp existing results from the freshly
            // loaded set.
            let installed = state
                .installed_pack_project_ids
                .get(&(slug.clone(), kind))
                .cloned();
            if let ActiveView::PackBrowser {
                slug: cur_slug,
                kind: cur_kind,
                results,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug && *cur_kind == kind {
                    if let Some(set) = installed.as_ref() {
                        for r in results.iter_mut() {
                            if set.contains(&r.project_id) {
                                r.already_installed = true;
                            }
                        }
                    }
                }
            }
            if let ActiveView::InstalledPacksList {
                slug: cur_slug,
                kind: cur_kind,
                packs: ref mut p,
                selected,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug && *cur_kind == kind {
                    let new_len = packs.len();
                    *p = packs;
                    *selected = (*selected).min(new_len.saturating_sub(1));
                }
            }
            vec![]
        }

        Action::InstalledPacksMove(delta) => {
            if let ActiveView::InstalledPacksList {
                packs,
                selected,
                transient_status,
                ..
            } = &mut state.active_view
            {
                // Clear transient status on any navigation.
                *transient_status = None;
                let len = packs.len();
                if len > 0 {
                    let new_idx =
                        (*selected as isize + delta as isize).clamp(0, len as isize - 1) as usize;
                    *selected = new_idx;
                }
            }
            vec![]
        }

        Action::InstalledPacksCycleKind => {
            // Cycle: InstalledModsList → InstalledPacksList(Resource)
            //         → InstalledPacksList(Shader) → InstalledModsList
            match &state.active_view {
                ActiveView::InstalledModsList { slug, .. } => {
                    let slug2 = slug.clone();
                    state.active_view = ActiveView::InstalledPacksList {
                        slug: slug2.clone(),
                        kind: PackKind::Resource,
                        packs: Vec::new(),
                        selected: 0,
                        transient_status: None,
                    };
                    vec![Effect::FetchInstalledPacks {
                        slug: slug2,
                        kind: PackKind::Resource,
                    }]
                }
                ActiveView::InstalledPacksList {
                    slug,
                    kind: PackKind::Resource,
                    ..
                } => {
                    let slug2 = slug.clone();
                    state.active_view = ActiveView::InstalledPacksList {
                        slug: slug2.clone(),
                        kind: PackKind::Shader,
                        packs: Vec::new(),
                        selected: 0,
                        transient_status: None,
                    };
                    vec![Effect::FetchInstalledPacks {
                        slug: slug2,
                        kind: PackKind::Shader,
                    }]
                }
                ActiveView::InstalledPacksList {
                    slug,
                    kind: PackKind::Shader,
                    ..
                } => {
                    let slug2 = slug.clone();
                    state.active_view = ActiveView::InstalledModsList {
                        slug: slug2.clone(),
                        mods: Vec::new(),
                        selected: 0,
                    };
                    vec![Effect::FetchInstalledMods { slug: slug2 }]
                }
                _ => vec![],
            }
        }

        Action::TogglePackEnabled => {
            let captured = match &state.active_view {
                ActiveView::InstalledPacksList {
                    slug,
                    kind,
                    packs,
                    selected,
                    ..
                } => packs
                    .get(*selected)
                    .map(|row| (slug.clone(), *kind, row.mod_id.clone())),
                _ => None,
            };
            let Some((slug, kind, mod_id)) = captured else {
                return vec![];
            };
            vec![Effect::TogglePackEnabledEff { slug, kind, mod_id }]
        }

        Action::ShaderToggleNotice => {
            if let ActiveView::InstalledPacksList {
                transient_status, ..
            } = &mut state.active_view
            {
                *transient_status =
                    Some("Shaders cannot be toggled -- use Iris/OptiFine in-game".to_string());
            }
            vec![]
        }

        Action::OpenUninstallPackConfirm => {
            let captured = match &state.active_view {
                ActiveView::InstalledPacksList {
                    slug,
                    kind,
                    packs,
                    selected,
                    ..
                } => packs.get(*selected).map(|row| {
                    (
                        slug.clone(),
                        *kind,
                        row.mod_id.clone(),
                        row.file_name.clone(),
                    )
                }),
                _ => None,
            };
            let Some((slug, kind, mod_id, file_name)) = captured else {
                return vec![];
            };
            state.active_view = ActiveView::UninstallPackConfirm {
                slug,
                kind,
                mod_id,
                file_name,
            };
            vec![]
        }

        Action::ConfirmUninstallPack => {
            let captured = match &state.active_view {
                ActiveView::UninstallPackConfirm {
                    slug, kind, mod_id, ..
                } => Some((slug.clone(), *kind, mod_id.clone())),
                _ => None,
            };
            let Some((slug, kind, mod_id)) = captured else {
                return vec![];
            };
            state.active_view = ActiveView::InstalledPacksList {
                slug: slug.clone(),
                kind,
                packs: Vec::new(),
                selected: 0,
                transient_status: None,
            };
            vec![
                Effect::UninstallPack {
                    slug: slug.clone(),
                    kind,
                    mod_id,
                },
                Effect::FetchInstalledPacks { slug, kind },
            ]
        }

        Action::CancelUninstallPack => {
            let captured = match &state.active_view {
                ActiveView::UninstallPackConfirm { slug, kind, .. } => Some((slug.clone(), *kind)),
                _ => None,
            };
            let (slug, kind) = captured.unwrap_or_else(|| (String::new(), PackKind::Resource));
            state.active_view = ActiveView::InstalledPacksList {
                slug: slug.clone(),
                kind,
                packs: Vec::new(),
                selected: 0,
                transient_status: None,
            };
            vec![Effect::FetchInstalledPacks { slug, kind }]
        }

        Action::PackInstalled { slug, kind } => {
            state.running_pack_jobs.remove(&(slug.clone(), kind));
            vec![Effect::FetchInstalledPacks { slug, kind }]
        }

        Action::PackInstallFailed {
            slug,
            kind,
            pack_title,
            version_label,
            error,
        } => {
            state.running_pack_jobs.remove(&(slug.clone(), kind));
            tracing::warn!(?kind, %slug, %error, "PackInstallFailed");
            // Surface to the user. Previously this arm only logged to
            // tracing -- the install would silently revert to the pack
            // browser with no indication of what went wrong.
            let return_to = match &state.active_view {
                ActiveView::PackBrowser { .. } => PackInstallFailedReturnTo::PackBrowser,
                _ => PackInstallFailedReturnTo::InstalledPacksList,
            };
            state.active_view = ActiveView::PackInstallFailedModal {
                slug,
                kind,
                pack_title: if pack_title.is_empty() {
                    "(unknown pack)".to_string()
                } else {
                    pack_title
                },
                version_label,
                error,
                return_to,
            };
            vec![]
        }

        Action::DismissPackInstallFailed => {
            let captured = match &state.active_view {
                ActiveView::PackInstallFailedModal {
                    slug,
                    kind,
                    return_to,
                    ..
                } => Some((slug.clone(), *kind, *return_to)),
                _ => None,
            };
            match captured {
                Some((slug, kind, PackInstallFailedReturnTo::PackBrowser)) => {
                    let mc = state
                        .instances
                        .iter()
                        .find(|m| m.slug == slug)
                        .map(|m| m.mc_version_id.clone());
                    state.active_view = ActiveView::PackBrowser {
                        slug: slug.clone(),
                        kind,
                        search: String::new(),
                        is_searching: false,
                        fetch_state: ModBrowserFetchState::Loading,
                        results: Vec::new(),
                        selected: 0,
                    };
                    vec![Effect::SearchPacks {
                        slug,
                        kind,
                        query: String::new(),
                        mc,
                    }]
                }
                Some((slug, kind, PackInstallFailedReturnTo::InstalledPacksList)) => {
                    state.active_view = ActiveView::InstalledPacksList {
                        slug: slug.clone(),
                        kind,
                        packs: Vec::new(),
                        selected: 0,
                        transient_status: None,
                    };
                    vec![Effect::FetchInstalledPacks { slug, kind }]
                }
                None => {
                    state.active_view = ActiveView::default();
                    vec![]
                }
            }
        }

        Action::PackDropFailed { slug, kind, error } => {
            // Surface error as transient status if currently viewing pack browser.
            if let ActiveView::PackBrowser { .. } | ActiveView::InstalledPacksList { .. } =
                &state.active_view
            {
                if let ActiveView::InstalledPacksList {
                    transient_status, ..
                } = &mut state.active_view
                {
                    *transient_status = Some(format!("Drop failed: {error}"));
                }
            }
            state.running_pack_jobs.remove(&(slug, kind));
            vec![]
        }

        Action::PackUninstalled { slug, kind, mod_id } => {
            // Optimistic drop so the PackBrowser updates before the
            // FetchInstalledPacks round-trip finishes (mirrors ModUninstalled).
            if let Some(set) = state
                .installed_pack_project_ids
                .get_mut(&(slug.clone(), kind))
            {
                set.remove(&mod_id);
            }
            if let ActiveView::PackBrowser {
                slug: cur_slug,
                kind: cur_kind,
                results,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug && *cur_kind == kind {
                    for r in results.iter_mut() {
                        if r.project_id == mod_id {
                            r.already_installed = false;
                        }
                    }
                }
            }
            vec![Effect::FetchInstalledPacks { slug, kind }]
        }

        Action::PackToggleFailed { slug, kind, error } => {
            // GAP-11-B: surface silent toggle failures so the user sees what went wrong.
            if let ActiveView::InstalledPacksList {
                slug: cur_slug,
                kind: cur_kind,
                transient_status,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug && *cur_kind == kind {
                    *transient_status = Some(format!("Toggle failed: {error}"));
                }
            }
            vec![]
        }

        Action::PackToggled {
            slug,
            kind,
            mod_id,
            new_enabled,
        } => {
            if let ActiveView::InstalledPacksList {
                slug: cur_slug,
                kind: cur_kind,
                packs,
                ..
            } = &mut state.active_view
            {
                if *cur_slug == slug && *cur_kind == kind {
                    if let Some(row) = packs.iter_mut().find(|r| r.mod_id == mod_id) {
                        row.enabled = new_enabled;
                    }
                }
            }
            vec![]
        }

        Action::InstallPackFromBrowser { slug, kind } => {
            // GAP-11-A wiring: read the selected hit from the active PackBrowser
            // view, then dispatch the auto-pick install chain via
            // `Effect::FetchPackVersions`. The chain unfolds in run.rs:
            //   list_versions → pick first is_latest_stable → get_version →
            //   `Action::AutoStartPackInstall` → `Effect::InstallPackFromModrinth`.
            let picked = if let ActiveView::PackBrowser {
                results, selected, ..
            } = &state.active_view
            {
                results.get(*selected).cloned()
            } else {
                None
            };
            let mc = state
                .instances
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| m.mc_version_id.clone());
            if let Some(hit) = picked {
                vec![Effect::FetchPackVersions {
                    slug,
                    kind,
                    project_id: hit.project_id,
                    project_slug: hit.slug,
                    project_title: hit.title,
                    mc,
                }]
            } else {
                vec![]
            }
        }

        Action::AutoStartPackInstall {
            slug,
            kind,
            project_id,
            project_slug,
            project_title,
            version,
        } => {
            // Hand off to the existing install effect (run.rs:1891). This arm
            // is the second hop of the GAP-11-A wiring chain -- the auto-pick
            // happens in the `Effect::FetchPackVersions` handler in run.rs.
            vec![Effect::InstallPackFromModrinth {
                slug,
                kind,
                project_id,
                project_slug,
                project_title,
                version,
            }]
        }

        Action::PackVersionsFailed {
            slug,
            kind,
            project_id: _,
            message,
        } => {
            // Tail-route to the existing PackInstallFailed arm so the failure
            // UI surfaces uniformly. project_id is dropped; PackInstallFailed
            // does not carry it (the active PackBrowser view supplies context
            // via slug + kind alone). pack_title/version_label are unknown
            // at this stage (the version list never resolved), so the modal
            // falls back to "(unknown pack)" / "".
            update(
                state,
                Action::PackInstallFailed {
                    slug,
                    kind,
                    pack_title: String::new(),
                    version_label: String::new(),
                    error: message,
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::StorageBackend;

    fn sample_account(id: &str, active: bool) -> Account {
        Account {
            id: id.into(),
            mc_username: format!("P{id}"),
            mc_uuid: format!("{id:0>8}-0000-4000-8000-000000000000"),
            mc_token_expires_at: 0,
            msa_token_expires_at: 0,
            added_at: 0,
            last_refreshed_at: 0,
            is_active: active,
            storage: StorageBackend::EncryptedFile,
        }
    }

    #[test]
    fn test_open_accounts_fetches_list() {
        let mut state = AppState::default();
        let effects = update(&mut state, Action::OpenAccounts);
        assert!(matches!(
            state.active_view,
            ActiveView::AccountsList { selected: 0 }
        ));
        assert!(matches!(effects.as_slice(), [Effect::FetchAccounts]));
    }

    #[test]
    fn test_accounts_loaded_derives_active_id() {
        let mut state = AppState::default();
        let list = vec![sample_account("A", false), sample_account("B", true)];
        let _ = update(&mut state, Action::AccountsLoaded(list));
        assert_eq!(state.active_account_id.as_deref(), Some("B"));
        assert_eq!(state.accounts.len(), 2);
    }

    #[test]
    fn test_activate_account_sets_active_id_exclusively() {
        let mut state = AppState {
            accounts: vec![sample_account("A", true), sample_account("B", false)],
            active_account_id: Some("A".into()),
            ..AppState::default()
        };
        let effects = update(&mut state, Action::ActivateAccount { id: "B".into() });
        assert_eq!(state.active_account_id.as_deref(), Some("B"));
        assert!(
            state
                .accounts
                .iter()
                .find(|a| a.id == "B")
                .unwrap()
                .is_active
        );
        assert!(
            !state
                .accounts
                .iter()
                .find(|a| a.id == "A")
                .unwrap()
                .is_active
        );
        assert!(matches!(
            effects.as_slice(),
            [Effect::ActivateAccount { .. }]
        ));
    }

    #[test]
    fn test_remove_active_account_clears_active_id() {
        let mut state = AppState {
            accounts: vec![sample_account("A", true)],
            active_account_id: Some("A".into()),
            ..AppState::default()
        };
        let _ = update(&mut state, Action::RemoveAccount { id: "A".into() });
        assert!(state.active_account_id.is_none());
        assert!(state.accounts.is_empty());
    }

    #[test]
    fn test_launch_instance_with_active_account_builds_msa_context() {
        let mut state = AppState::default();
        state.instances.push(crate::domain::InstanceManifest::new(
            "s".into(),
            "s".into(),
            "1.21.4".into(),
        ));
        state.active_account_id = Some("id-1".into());
        let effects = update(&mut state, Action::LaunchInstance { slug: "s".into() });
        match effects.as_slice() {
            [Effect::LaunchInstance {
                auth_ctx: AuthContext::Msa { account_id },
                ..
            }] => {
                assert_eq!(account_id, "id-1");
            }
            other => panic!("expected Msa LaunchInstance; got {other:?}"),
        }
    }

    #[test]
    fn test_launch_instance_without_active_account_builds_offline_context() {
        let mut state = AppState::default();
        // new(display_name, slug, mc_version_id)
        let m = crate::domain::InstanceManifest::new("Pretty".into(), "s".into(), "1.21.4".into());
        state.instances.push(m);
        assert!(state.active_account_id.is_none());
        let effects = update(&mut state, Action::LaunchInstance { slug: "s".into() });
        match effects.as_slice() {
            [Effect::LaunchInstance {
                auth_ctx: AuthContext::Offline { username },
                ..
            }] => {
                assert_eq!(username, "Pretty");
            }
            other => panic!("expected Offline LaunchInstance; got {other:?}"),
        }
    }

    #[test]
    fn test_account_auth_started_transitions_to_modal() {
        let mut state = AppState::default();
        let _ = update(
            &mut state,
            Action::AccountAuthStarted {
                user_code: "ABCD".into(),
                verification_uri: "https://ms/link".into(),
                expires_at: std::time::Instant::now() + std::time::Duration::from_secs(900),
            },
        );
        assert!(matches!(
            state.active_view,
            ActiveView::AddAccountDeviceCode { .. }
        ));
    }

    #[test]
    fn test_account_auth_failed_transitions_to_failed_modal() {
        let mut state = AppState::default();
        let _ = update(
            &mut state,
            Action::AccountAuthFailed {
                reason: "no license".into(),
            },
        );
        match &state.active_view {
            ActiveView::AccountAuthFailed { reason } => assert_eq!(reason, "no license"),
            other => panic!("expected AccountAuthFailed modal; got {other:?}"),
        }
    }

    #[test]
    fn test_cancel_add_account_cancels_token_and_returns_to_list() {
        let mut state = AppState::default();
        let t = CancellationToken::new();
        state.add_account_cancel = Some(t.clone());
        let _ = update(&mut state, Action::CancelAddAccount);
        assert!(t.is_cancelled());
        assert!(state.add_account_cancel.is_none());
        assert!(matches!(
            state.active_view,
            ActiveView::AccountsList { selected: 0 }
        ));
    }

    // ── Phase 6: Loader picker tests ──────────────────────────────────────────

    fn fab_versions(n: usize) -> Vec<LoaderVersionEntry> {
        (0..n)
            .map(|i| LoaderVersionEntry {
                version: format!("0.16.{i}"),
                stable: i % 2 == 0,
                build: Some(500 + i as u32),
            })
            .collect()
    }

    fn vanilla_state_with(slug: &str, mc: &str) -> AppState {
        let mut s = AppState::default();
        s.instances.push(crate::domain::InstanceManifest::new(
            slug.into(),
            slug.into(),
            mc.into(),
        ));
        s
    }

    #[test]
    fn test_open_loader_picker_sets_active_view() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        let _ = update(&mut s, Action::OpenLoaderPicker { slug: "ti".into() });
        assert!(matches!(
            s.active_view,
            ActiveView::LoaderPickerModal { selected: 0, .. }
        ));
    }

    #[test]
    fn test_open_loader_picker_blocks_running_instance() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        s.running_instances
            .insert("ti".into(), CancellationToken::new());
        let _ = update(&mut s, Action::OpenLoaderPicker { slug: "ti".into() });
        // No transition -- instance is running
        assert!(matches!(s.active_view, ActiveView::InstanceList { .. }));
    }

    #[test]
    fn test_loader_picker_move_wraps_five_rows() {
        // Now 5 rows: None / Fabric / Quilt / Forge / NeoForge
        let mut s = vanilla_state_with("ti", "1.21.4");
        let _ = update(&mut s, Action::OpenLoaderPicker { slug: "ti".into() });
        let _ = update(&mut s, Action::LoaderPickerMove(1));
        let _ = update(&mut s, Action::LoaderPickerMove(1));
        let _ = update(&mut s, Action::LoaderPickerMove(1));
        // 3 moves → index 3 (Forge row)
        if let ActiveView::LoaderPickerModal { selected, .. } = &s.active_view {
            assert_eq!(*selected, 3);
        } else {
            panic!("wrong view");
        }
        let _ = update(&mut s, Action::LoaderPickerMove(1));
        let _ = update(&mut s, Action::LoaderPickerMove(1));
        // 5 moves total → index 5 mod 5 = 0 (wraps back to None row)
        if let ActiveView::LoaderPickerModal { selected, .. } = s.active_view {
            assert_eq!(selected, 0);
        } else {
            panic!("wrong view");
        }
    }

    #[test]
    fn test_loader_picker_select_fabric_emits_fetch_effect() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        let _ = update(&mut s, Action::OpenLoaderPicker { slug: "ti".into() });
        let _ = update(&mut s, Action::LoaderPickerMove(1)); // Fabric (index 1)
        let effects = update(&mut s, Action::LoaderPickerSelect);
        assert!(matches!(
            effects.as_slice(),
            [Effect::FetchLoaderVersions {
                loader_type: LoaderType::Fabric,
                ..
            }]
        ));
        assert!(matches!(
            s.active_view,
            ActiveView::LoaderVersionPickerModal {
                loader: LoaderType::Fabric,
                ..
            }
        ));
    }

    #[test]
    fn test_loader_picker_select_none_with_no_loader_is_noop() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        let _ = update(&mut s, Action::OpenLoaderPicker { slug: "ti".into() });
        // selected = 0 = None
        let effects = update(&mut s, Action::LoaderPickerSelect);
        assert!(effects.is_empty());
    }

    #[test]
    fn test_loader_versions_loaded_replaces_versions() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        s.active_view = ActiveView::LoaderVersionPickerModal {
            slug: "ti".into(),
            loader: LoaderType::Fabric,
            versions: vec![],
            filter_stable_only: true,
            search: String::new(),
            selected: 0,
            current_version: None,
        };
        let _ = update(
            &mut s,
            Action::LoaderVersionsLoaded {
                slug: "ti".into(),
                loader: LoaderType::Fabric,
                versions: fab_versions(3),
            },
        );
        if let ActiveView::LoaderVersionPickerModal {
            versions, selected, ..
        } = &s.active_view
        {
            assert_eq!(versions.len(), 3);
            assert_eq!(*selected, 0);
        } else {
            panic!()
        }
    }

    #[test]
    fn test_toggle_stable_filter_flips_bool() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        s.active_view = ActiveView::LoaderVersionPickerModal {
            slug: "ti".into(),
            loader: LoaderType::Fabric,
            versions: fab_versions(2),
            filter_stable_only: true,
            search: String::new(),
            selected: 0,
            current_version: None,
        };
        let _ = update(&mut s, Action::ToggleStableFilter);
        if let ActiveView::LoaderVersionPickerModal {
            filter_stable_only, ..
        } = &s.active_view
        {
            assert!(!filter_stable_only);
        } else {
            panic!()
        }
    }

    #[test]
    fn test_loader_version_select_no_current_emits_install_effect() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        s.active_view = ActiveView::LoaderVersionPickerModal {
            slug: "ti".into(),
            loader: LoaderType::Fabric,
            versions: fab_versions(3),
            filter_stable_only: false,
            search: String::new(),
            selected: 0,
            current_version: None,
        };
        let effects = update(&mut s, Action::LoaderVersionSelect);
        match effects.as_slice() {
            [Effect::InstallLoader {
                loader_type: LoaderType::Fabric,
                mc_version,
                loader_version,
                ..
            }] => {
                assert_eq!(mc_version, "1.21.4");
                assert_eq!(loader_version, "0.16.0");
            }
            other => panic!("expected InstallLoader, got {other:?}"),
        }
        assert!(matches!(
            s.active_view,
            ActiveView::LoaderInstallProgressModal { .. }
        ));
    }

    /// Phase 6 UAT Check 5 regression pin: when an instance has Fabric installed
    /// and the user picks a Quilt version, the LoaderSwitchConfirm must surface
    /// `type_switch: true` so the red WARNING line renders. Prior bug: the
    /// version-picker plumbed `current_version` was filtered by target kind, so
    /// a cross-kind switch saw `current_version: None` and dropped to the direct
    /// install branch -- bypassing the safety warning entirely.
    #[test]
    fn test_loader_version_select_cross_kind_emits_warning_confirm() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        // Install Fabric on the instance.
        s.instances[0].loader = Some(crate::loader::types::LoaderInfo {
            kind: crate::domain::instance::ModloaderKind::Fabric,
            version: "0.16.9".into(),
            version_id: "fabric-loader-0.16.9-1.21.4".into(),
        });
        // Open Quilt version picker -- current_version is None because the picker
        // filters by target kind (Quilt) and the installed kind is Fabric.
        s.active_view = ActiveView::LoaderVersionPickerModal {
            slug: "ti".into(),
            loader: LoaderType::Quilt,
            versions: fab_versions(3),
            filter_stable_only: false,
            search: String::new(),
            selected: 0,
            current_version: None,
        };
        let effects = update(&mut s, Action::LoaderVersionSelect);
        assert!(effects.is_empty(),
            "cross-kind switch must NOT emit InstallLoader directly; it must show the WARNING confirm first");
        match &s.active_view {
            ActiveView::LoaderSwitchConfirm {
                from_loader,
                to_loader,
                type_switch,
                ..
            } => {
                assert!(*type_switch,
                    "cross-kind switch (Fabric → Quilt) MUST set type_switch: true so the red WARNING line renders (UAT Check 5)");
                assert_eq!(from_loader.as_deref(), Some("fabric:0.16.9"),
                    "from_loader must reflect the actually-installed loader, not the target-kind filter");
                assert!(
                    to_loader.starts_with("quilt:"),
                    "to_loader must reflect the chosen target: {to_loader}"
                );
            }
            other => panic!("expected LoaderSwitchConfirm with type_switch: true, got {other:?}"),
        }
    }

    /// Same-kind different-version switch: confirm shows but type_switch stays false
    /// (no red WARNING). Pins the non-warning branch of the 3-way decision.
    #[test]
    fn test_loader_version_select_same_kind_diff_version_confirm_no_warning() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        s.instances[0].loader = Some(crate::loader::types::LoaderInfo {
            kind: crate::domain::instance::ModloaderKind::Fabric,
            version: "0.16.5".into(),
            version_id: "fabric-loader-0.16.5-1.21.4".into(),
        });
        s.active_view = ActiveView::LoaderVersionPickerModal {
            slug: "ti".into(),
            loader: LoaderType::Fabric,
            versions: fab_versions(3),
            filter_stable_only: false,
            search: String::new(),
            selected: 1,
            current_version: Some("0.16.5".into()),
        };
        let effects = update(&mut s, Action::LoaderVersionSelect);
        assert!(
            effects.is_empty(),
            "same-kind diff-version must show confirm, not install directly"
        );
        match &s.active_view {
            ActiveView::LoaderSwitchConfirm { type_switch, .. } => {
                assert!(
                    !*type_switch,
                    "same-kind switch must NOT set type_switch (no WARNING)"
                );
            }
            other => panic!("expected LoaderSwitchConfirm, got {other:?}"),
        }
    }

    /// Same-kind same-version (already installed) is a no-op. Pins that the new
    /// 3-way logic preserves the original short-circuit.
    #[test]
    fn test_loader_version_select_already_installed_is_noop() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        s.instances[0].loader = Some(crate::loader::types::LoaderInfo {
            kind: crate::domain::instance::ModloaderKind::Fabric,
            version: "0.16.0".into(),
            version_id: "fabric-loader-0.16.0-1.21.4".into(),
        });
        s.active_view = ActiveView::LoaderVersionPickerModal {
            slug: "ti".into(),
            loader: LoaderType::Fabric,
            versions: fab_versions(3),
            filter_stable_only: false,
            search: String::new(),
            selected: 0,
            current_version: Some("0.16.0".into()),
        };
        let prev_view = s.active_view.clone();
        let effects = update(&mut s, Action::LoaderVersionSelect);
        assert!(effects.is_empty(), "already-installed must be no-op");
        assert!(
            matches!(&s.active_view, ActiveView::LoaderVersionPickerModal { .. }),
            "already-installed must leave active_view unchanged: was {prev_view:?}, now {:?}",
            s.active_view
        );
    }

    #[test]
    fn test_loader_install_started_inserts_token() {
        let mut s = AppState::default();
        let t = CancellationToken::new();
        let _ = update(
            &mut s,
            Action::LoaderInstallStarted {
                slug: "ti".into(),
                token: t.clone(),
            },
        );
        assert!(s.running_loader_installs.contains_key("ti"));
        assert!(!t.is_cancelled());
    }

    #[test]
    fn test_loader_installed_clears_token_and_returns_to_list() {
        let mut s = AppState::default();
        s.running_loader_installs
            .insert("ti".into(), CancellationToken::new());
        let effects = update(&mut s, Action::LoaderInstalled { slug: "ti".into() });
        assert!(s.running_loader_installs.is_empty());
        assert!(matches!(s.active_view, ActiveView::InstanceList { .. }));
        assert!(matches!(effects.as_slice(), [Effect::FetchInstances]));
    }

    #[test]
    fn test_loader_install_failed_routes_to_failed_modal() {
        let mut s = AppState::default();
        s.running_loader_installs
            .insert("ti".into(), CancellationToken::new());
        let _ = update(
            &mut s,
            Action::LoaderInstallFailed {
                slug: "ti".into(),
                loader: LoaderType::Fabric,
                version: "0.16.9".into(),
                error: "network".into(),
                log_tail: "GET ...".into(),
            },
        );
        assert!(s.running_loader_installs.is_empty());
        assert!(matches!(
            s.active_view,
            ActiveView::LoaderInstallFailedModal { .. }
        ));
    }

    #[test]
    fn test_cancel_loader_install_cancels_token() {
        let mut s = AppState::default();
        let t = CancellationToken::new();
        s.running_loader_installs.insert("ti".into(), t.clone());
        let effects = update(&mut s, Action::CancelLoaderInstall { slug: "ti".into() });
        assert!(t.is_cancelled());
        assert!(s.running_loader_installs.is_empty());
        assert!(matches!(
            effects.as_slice(),
            [Effect::CancelLoaderInstall { .. }]
        ));
    }

    #[test]
    fn test_dismiss_loader_install_failed_returns_to_list() {
        let mut s = AppState {
            active_view: ActiveView::LoaderInstallFailedModal {
                slug: "ti".into(),
                loader: LoaderType::Fabric,
                version: "0.16.9".into(),
                error: "x".into(),
                log_tail: "y".into(),
            },
            ..AppState::default()
        };
        let _ = update(&mut s, Action::DismissLoaderInstallFailed);
        assert!(matches!(s.active_view, ActiveView::InstanceList { .. }));
    }

    #[test]
    fn test_confirm_loader_switch_emits_remove_when_to_none() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        s.active_view = ActiveView::LoaderSwitchConfirm {
            slug: "ti".into(),
            from_loader: Some("fabric:0.16.9".into()),
            to_loader: "none".into(),
            type_switch: false,
        };
        let effects = update(&mut s, Action::ConfirmLoaderSwitch);
        assert!(matches!(effects.as_slice(), [Effect::RemoveLoader { .. }]));
        assert!(matches!(s.active_view, ActiveView::InstanceList { .. }));
    }

    #[test]
    fn test_confirm_loader_switch_emits_install_for_to_loader() {
        let mut s = vanilla_state_with("ti", "1.21.4");
        s.active_view = ActiveView::LoaderSwitchConfirm {
            slug: "ti".into(),
            from_loader: Some("fabric:0.16.8".into()),
            to_loader: "fabric:0.16.9".into(),
            type_switch: false,
        };
        let effects = update(&mut s, Action::ConfirmLoaderSwitch);
        match effects.as_slice() {
            [Effect::InstallLoader {
                loader_type: LoaderType::Fabric,
                loader_version,
                mc_version,
                ..
            }] => {
                assert_eq!(loader_version, "0.16.9");
                assert_eq!(mc_version, "1.21.4");
            }
            other => panic!("expected InstallLoader, got {other:?}"),
        }
    }

    #[test]
    fn test_loader_versions_visible_indices_quilt_shows_all_when_filter_on() {
        // Open Question 3 lock: Quilt always shows all versions; UI suffix renders (beta)
        let versions = vec![
            LoaderVersionEntry {
                version: "0.30.0-beta.7".into(),
                stable: false,
                build: Some(120),
            },
            LoaderVersionEntry {
                version: "0.27.2".into(),
                stable: true,
                build: Some(50),
            },
        ];
        let visible = loader_versions_visible_indices(&versions, LoaderType::Quilt, true, "");
        assert_eq!(
            visible,
            vec![0, 1],
            "Quilt always shows all (per UI-SPEC Open Q3)"
        );
    }

    #[test]
    fn test_loader_versions_visible_indices_fabric_filters_unstable() {
        let versions = vec![
            LoaderVersionEntry {
                version: "0.16.9".into(),
                stable: true,
                build: Some(509),
            },
            LoaderVersionEntry {
                version: "0.17.0-beta.1".into(),
                stable: false,
                build: Some(600),
            },
        ];
        let visible = loader_versions_visible_indices(&versions, LoaderType::Fabric, true, "");
        assert_eq!(visible, vec![0]);
        let all = loader_versions_visible_indices(&versions, LoaderType::Fabric, false, "");
        assert_eq!(all, vec![0, 1]);
    }

    #[test]
    fn test_loader_install_log_tail_updates_modal_log_tail() {
        let mut state = AppState {
            active_view: ActiveView::LoaderInstallProgressModal {
                slug: "test".into(),
                loader: LoaderType::Forge,
                version: "47.4.20".into(),
                step_label: "Running installer".into(),
                step_index: 1,
                step_total: 5,
                bytes_done: 0,
                bytes_total: 0,
                cancel_token_key: "test-key".into(),
                log_tail: String::new(),
            },
            ..AppState::default()
        };
        let _ = update(
            &mut state,
            Action::LoaderInstallLogTail {
                slug: "test".into(),
                tail: "Running Processor 3/7".into(),
            },
        );
        if let ActiveView::LoaderInstallProgressModal { log_tail, .. } = &state.active_view {
            assert_eq!(log_tail, "Running Processor 3/7");
        } else {
            panic!("active view changed unexpectedly");
        }
    }

    #[test]
    fn test_loader_install_log_tail_ignored_for_other_slug() {
        let mut state = AppState {
            active_view: ActiveView::LoaderInstallProgressModal {
                slug: "alpha".into(),
                loader: LoaderType::Forge,
                version: "1".into(),
                step_label: "x".into(),
                step_index: 0,
                step_total: 1,
                bytes_done: 0,
                bytes_total: 0,
                cancel_token_key: "k".into(),
                log_tail: "original".into(),
            },
            ..AppState::default()
        };
        let _ = update(
            &mut state,
            Action::LoaderInstallLogTail {
                slug: "beta".into(),
                tail: "different".into(),
            },
        );
        if let ActiveView::LoaderInstallProgressModal { log_tail, .. } = &state.active_view {
            assert_eq!(log_tail, "original", "tail must not change for other slug");
        } else {
            panic!("active view changed");
        }
    }

    // ------------------------------------------------------------------------
    // GAP-8-F (08.1-05): Modrinth search-failure dispatch.
    //
    // When Effect::SearchModrinth's Err arm fires, the action chain must transition
    // ModBrowser fetch_state to Error(message) -- NOT to Ready with empty hits, which
    // the view would render as "No mods found" (masking unreachable Modrinth).
    // Mirrors the existing CfBrowserSearchFailed pattern.
    // ------------------------------------------------------------------------

    #[test]
    fn mod_browser_search_failed_sets_error_fetch_state() {
        use crate::mods::types::{ModBrowserFetchState, ModrinthSearchHit};
        let mut state = AppState {
            active_view: ActiveView::ModBrowser {
                slug: "myinst".into(),
                search: String::new(),
                is_searching: false,
                mc_filter_override: None,
                loader_filter_override: None,
                results: Vec::<ModrinthSearchHit>::new(),
                selected: 0,
                fetch_state: ModBrowserFetchState::Loading,
                selected_detail: None,
            },
            ..AppState::default()
        };
        let effects = update(
            &mut state,
            Action::ModBrowserSearchFailed {
                slug: "myinst".into(),
                message: "connection refused".into(),
            },
        );
        assert!(effects.is_empty());
        match &state.active_view {
            ActiveView::ModBrowser { fetch_state, .. } => match fetch_state {
                ModBrowserFetchState::Error(m) => assert_eq!(m, "connection refused"),
                other => panic!("expected Error, got {other:?}"),
            },
            other => panic!("expected ModBrowser view, got {other:?}"),
        }
    }

    #[test]
    fn mod_browser_search_failed_for_other_slug_is_noop() {
        use crate::mods::types::{ModBrowserFetchState, ModrinthSearchHit};
        let mut state = AppState {
            active_view: ActiveView::ModBrowser {
                slug: "myinst".into(),
                search: String::new(),
                is_searching: false,
                mc_filter_override: None,
                loader_filter_override: None,
                results: Vec::<ModrinthSearchHit>::new(),
                selected: 0,
                fetch_state: ModBrowserFetchState::Loading,
                selected_detail: None,
            },
            ..AppState::default()
        };
        let _ = update(
            &mut state,
            Action::ModBrowserSearchFailed {
                slug: "OTHER".into(),
                message: "irrelevant".into(),
            },
        );
        // Slug mismatch: fetch_state stays Loading.
        match &state.active_view {
            ActiveView::ModBrowser { fetch_state, .. } => {
                assert!(matches!(fetch_state, ModBrowserFetchState::Loading));
            }
            other => panic!("expected ModBrowser view, got {other:?}"),
        }
    }

    // ------------------------------------------------------------------------
    // GAP-11-A (11-06): Pack browser Enter-key install wiring.
    //
    // The install dispatch chain has 3 hops:
    //   1. Action::InstallPackFromBrowser   → Effect::FetchPackVersions
    //   2. (run.rs task: list_versions → pick latest stable → get_version)
    //      then Action::AutoStartPackInstall → Effect::InstallPackFromModrinth
    //   3. on failure, Action::PackVersionsFailed → tail-routes through
    //      Action::PackInstallFailed (existing arm)
    //
    // Tests below pin hops 1 + 2 + 3 in src/tui/app.rs (run.rs hop is
    // covered by tests/packs_integration.rs).
    // ------------------------------------------------------------------------

    fn pack_browser_state_with_one_hit(slug: &str, mc: &str, kind: PackKind) -> AppState {
        use crate::mods::types::{ModBrowserFetchState, ModrinthSearchHit};
        let manifest = crate::domain::instance::InstanceManifest::new(
            slug.to_string(),
            slug.to_string(),
            mc.to_string(),
        );
        AppState {
            instances: vec![manifest],
            active_view: ActiveView::PackBrowser {
                slug: slug.into(),
                kind,
                search: "faithful".into(),
                is_searching: true,
                fetch_state: ModBrowserFetchState::Ready,
                results: vec![ModrinthSearchHit {
                    project_id: "w0TnApzs".into(),
                    slug: "faithful-32x".into(),
                    title: "Faithful 32x".into(),
                    description: "fixture".into(),
                    downloads: 0,
                    already_installed: false,
                    icon_url: None,
                }],
                selected: 0,
            },
            ..AppState::default()
        }
    }

    #[test]
    fn test_install_pack_from_browser_resource_dispatches_fetch_versions() {
        let mut state = pack_browser_state_with_one_hit("test-inst", "1.20.4", PackKind::Resource);
        let effects = update(
            &mut state,
            Action::InstallPackFromBrowser {
                slug: "test-inst".into(),
                kind: PackKind::Resource,
            },
        );
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::FetchPackVersions {
                slug,
                kind,
                project_id,
                project_slug,
                project_title,
                mc,
            } => {
                assert_eq!(slug, "test-inst");
                assert_eq!(*kind, PackKind::Resource);
                assert_eq!(project_id, "w0TnApzs");
                assert_eq!(project_slug, "faithful-32x");
                assert_eq!(project_title, "Faithful 32x");
                assert_eq!(mc.as_deref(), Some("1.20.4"));
            }
            other => panic!("expected FetchPackVersions, got {other:?}"),
        }
    }

    #[test]
    fn test_install_pack_from_browser_shader_dispatches_fetch_versions() {
        // Symmetric path for SPAK-02.
        let mut state = pack_browser_state_with_one_hit("test-inst", "1.20.4", PackKind::Shader);
        let effects = update(
            &mut state,
            Action::InstallPackFromBrowser {
                slug: "test-inst".into(),
                kind: PackKind::Shader,
            },
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::FetchPackVersions {
                kind: PackKind::Shader,
                ..
            }
        ));
    }

    #[test]
    fn test_install_pack_from_browser_with_no_results_emits_no_effect() {
        use crate::mods::types::{ModBrowserFetchState, ModrinthSearchHit};
        let mut state = AppState {
            active_view: ActiveView::PackBrowser {
                slug: "test-inst".into(),
                kind: PackKind::Resource,
                search: String::new(),
                is_searching: false,
                fetch_state: ModBrowserFetchState::Ready,
                results: Vec::<ModrinthSearchHit>::new(),
                selected: 0,
            },
            ..AppState::default()
        };
        let effects = update(
            &mut state,
            Action::InstallPackFromBrowser {
                slug: "test-inst".into(),
                kind: PackKind::Resource,
            },
        );
        assert!(effects.is_empty(), "no selected hit -> no effect");
    }

    #[test]
    fn test_auto_start_pack_install_dispatches_install_from_modrinth() {
        use crate::mods::types::{ModrinthFile, ModrinthHashes};
        let version = ModrinthVersion {
            id: "kIpbQNcv".into(),
            project_id: "w0TnApzs".into(),
            name: "Faithful 32x for MC 1.20.4".into(),
            version_number: "1.20.4".into(),
            version_type: "release".into(),
            game_versions: vec!["1.20.4".into()],
            loaders: vec!["vanilla".into()],
            downloads: 0,
            date_published: "2026-01-01T00:00:00Z".into(),
            dependencies: vec![],
            files: vec![ModrinthFile {
                url: "https://example/faithful.zip".into(),
                filename: "faithful.zip".into(),
                primary: true,
                size: 100,
                hashes: ModrinthHashes {
                    sha1: "deadbeef".into(),
                    sha512: "deadbeef".into(),
                },
            }],
        };
        let mut state = AppState::default();
        let effects = update(
            &mut state,
            Action::AutoStartPackInstall {
                slug: "test-inst".into(),
                kind: PackKind::Resource,
                project_id: "w0TnApzs".into(),
                project_slug: "faithful-32x".into(),
                project_title: "Faithful 32x".into(),
                version: version.clone(),
            },
        );
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::InstallPackFromModrinth {
                slug,
                kind,
                project_id,
                project_slug,
                project_title,
                version: v,
            } => {
                assert_eq!(slug, "test-inst");
                assert_eq!(*kind, PackKind::Resource);
                assert_eq!(project_id, "w0TnApzs");
                assert_eq!(project_slug, "faithful-32x");
                assert_eq!(project_title, "Faithful 32x");
                assert_eq!(v.id, "kIpbQNcv");
            }
            other => panic!("expected InstallPackFromModrinth, got {other:?}"),
        }
    }

    #[test]
    fn test_pack_versions_failed_routes_to_install_failed() {
        // PackVersionsFailed must tail into PackInstallFailed (clears running
        // job book-keeping + logs warning); both arms emit no effects.
        let mut state = AppState {
            running_pack_jobs: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    ("test-inst".to_string(), PackKind::Resource),
                    tokio_util::sync::CancellationToken::new(),
                );
                m
            },
            ..AppState::default()
        };
        let effects = update(
            &mut state,
            Action::PackVersionsFailed {
                slug: "test-inst".into(),
                kind: PackKind::Resource,
                project_id: "w0TnApzs".into(),
                message: "boom".into(),
            },
        );
        assert!(effects.is_empty());
        assert!(
            !state
                .running_pack_jobs
                .contains_key(&("test-inst".to_string(), PackKind::Resource)),
            "PackInstallFailed tail-call must drop the running-job entry"
        );
    }
}
