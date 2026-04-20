//! Platform path resolution. The **only** module in the crate that calls
//! `directories::ProjectDirs`.
//!
//! All other modules consume paths via `AppPaths` methods — never via
//! environment variables, `cfg!(target_os = ...)`, or the `directories`
//! crate directly.
//!
//! ## Windows `\data` suffix
//!
//! On Windows, `ProjectDirs::from("", "", "mineltui").data_dir()` returns
//! `%APPDATA%\mineltui\data` (a `\data` subfolder is appended by the
//! `directories` crate). We accept this suffix rather than stripping it —
//! stripping risks diverging from the crate's own `config_dir`/`cache_dir`
//! layout and ratatui apps never expose these paths to end users.
//! See `.planning/research/PITFALLS.md` (Pitfall 6) for the rationale.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

/// Snapshot of all platform-relevant base directories for `mineltui`.
///
/// Construct once at startup via `AppPaths::resolve()` and clone / pass
/// references to everything downstream.
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// OS-specific data root. Linux: `~/.local/share/mineltui`.
    /// Windows: `%APPDATA%\mineltui\data`.
    pub data_dir: PathBuf,
    /// OS-specific config root. Linux: `~/.config/mineltui`.
    /// Windows: `%APPDATA%\mineltui\config`.
    pub config_dir: PathBuf,
    /// OS-specific cache root. Linux: `~/.cache/mineltui`.
    /// Windows: `%LOCALAPPDATA%\mineltui\cache`.
    pub cache_dir: PathBuf,
}

impl AppPaths {
    /// Resolve platform paths. Returns `None` only if the platform cannot
    /// determine a valid home directory (extremely rare; treat as fatal).
    pub fn resolve() -> Option<Self> {
        let proj = ProjectDirs::from("", "", "mineltui")?;
        Some(Self {
            data_dir: proj.data_dir().to_path_buf(),
            config_dir: proj.config_dir().to_path_buf(),
            cache_dir: proj.cache_dir().to_path_buf(),
        })
    }

    /// Construct `AppPaths` from explicit directories — for tests that
    /// want to redirect all paths into a `tempfile::TempDir`.
    pub fn with_roots(data_dir: PathBuf, config_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self { data_dir, config_dir, cache_dir }
    }

    /// Absolute path to the mineltui log file (single-file; rotation deferred).
    pub fn log_file(&self) -> PathBuf {
        self.data_dir.join("mineltui.log")
    }

    /// Absolute path to the global app config file.
    pub fn app_config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Directory where per-instance subdirectories live.
    pub fn instances_dir(&self) -> PathBuf {
        self.data_dir.join("instances")
    }

    /// Shared assets tree (Mojang asset objects, asset indexes).
    pub fn assets_dir(&self) -> PathBuf {
        self.data_dir.join("assets")
    }

    /// Shared Maven-layout libraries tree.
    pub fn libraries_dir(&self) -> PathBuf {
        self.data_dir.join("libraries")
    }

    /// Per-version client.jar + version JSON tree.
    pub fn versions_dir(&self) -> PathBuf {
        self.data_dir.join("versions")
    }

    /// Root for Mojang-managed Java runtimes.
    pub fn runtime_dir(&self) -> PathBuf {
        self.data_dir.join("runtime")
    }
}

/// Convenience: return `true` if `child` starts with `parent` after
/// normalizing trailing separators. Keeps the tests readable.
#[doc(hidden)]
pub fn path_starts_with(child: &Path, parent: &Path) -> bool {
    child.starts_with(parent)
}
