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

    /// Path for per-version client jar: `versions/{id}/{id}.jar`.
    pub fn version_jar(&self, version_id: &str) -> PathBuf {
        self.versions_dir().join(version_id).join(format!("{version_id}.jar"))
    }

    /// Path for per-version JSON: `versions/{id}/{id}.json`.
    pub fn version_json(&self, version_id: &str) -> PathBuf {
        self.versions_dir().join(version_id).join(format!("{version_id}.json"))
    }

    /// Shared library path (Maven-layout). `maven_path` is the full relative path
    /// from a library `downloads.artifact.path` field
    /// (e.g. `"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar"`).
    pub fn library_path(&self, maven_path: &str) -> PathBuf {
        self.libraries_dir().join(maven_path)
    }

    /// Asset index JSON: `assets/indexes/{id}.json`.
    pub fn asset_index(&self, id: &str) -> PathBuf {
        self.assets_dir().join("indexes").join(format!("{id}.json"))
    }

    /// Asset object: `assets/objects/{hash[0..2]}/{hash}`.
    /// Caller is responsible for validating that `hash` is a 40-char lowercase hex string.
    pub fn asset_object(&self, hash: &str) -> PathBuf {
        self.assets_dir().join("objects").join(&hash[..2]).join(hash)
    }

    /// Legacy virtual asset: `assets/virtual/{index_id}/{virtual_path}`.
    pub fn asset_virtual(&self, index_id: &str, virtual_path: &str) -> PathBuf {
        self.assets_dir().join("virtual").join(index_id).join(virtual_path)
    }

    /// Per-instance directory: `instances/{slug}/`.
    pub fn instance_dir(&self, slug: &str) -> PathBuf {
        self.instances_dir().join(slug)
    }

    /// Per-instance `.minecraft` working directory.
    pub fn instance_minecraft_dir(&self, slug: &str) -> PathBuf {
        self.instance_dir(slug).join(".minecraft")
    }

    /// Per-instance natives directory.
    pub fn instance_natives_dir(&self, slug: &str) -> PathBuf {
        self.instance_dir(slug).join("natives")
    }

    /// Per-instance manifest: `instances/{slug}/instance.json`.
    pub fn instance_manifest(&self, slug: &str) -> PathBuf {
        self.instance_dir(slug).join("instance.json")
    }

    /// Per-instance log file for Minecraft stdout/stderr drain.
    /// Path: `{data_dir}/instances/{slug}/logs/mineltui.log`.
    /// The file is APPENDED at each launch; sessions are separated by a
    /// timestamped header written by `launcher::spawn` (plan 03-03).
    /// Minecraft's own `logs/latest.log` is a separate file managed by
    /// Minecraft's own log4j config — do not conflate.
    pub fn instance_log_file(&self, slug: &str) -> PathBuf {
        self.instance_dir(slug).join("logs").join("mineltui.log")
    }
}

/// Convenience: return `true` if `child` starts with `parent` after
/// normalizing trailing separators. Keeps the tests readable.
#[doc(hidden)]
pub fn path_starts_with(child: &Path, parent: &Path) -> bool {
    child.starts_with(parent)
}
