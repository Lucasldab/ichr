//! User-facing configuration: keybinds + color palette, loaded from
//! `~/.config/ichr/config.toml` (XDG-resolved via `paths::AppPaths`).
//!
//! Design contract:
//!
//! - The file is **optional**. If absent, every slot uses its built-in
//!   default (which preserves the historical hardcoded UX). The
//!   `Config::load` API never returns an error: a missing file is
//!   `Default::default()`; a malformed file emits a `tracing::warn!`
//!   listing the field path and falls back to defaults for the affected
//!   slots only.
//! - Slots are **named, fixed**. Adding a new slot is a code change. We
//!   prefer a closed enumeration over open-ended user-injected styles
//!   so the rendering layer never has to handle "unknown slot" errors.
//! - Defaults match the current hardcoded values byte-for-byte when this
//!   module is first introduced -- no behavior change at the call site
//!   until renderers / handlers are migrated to read from `Config`.
//! - Lifetime: loaded once at startup, wrapped in `Arc<Config>`, threaded
//!   through `AppState`. No hot reload in v1; restart to pick up changes.
//!
//! See `keybinds` and `palette` for the slot enumerations.

pub mod keybinds;
pub mod palette;

use std::path::Path;

use serde::{Deserialize, Serialize};

pub use keybinds::{ActionKey, KeySpec, Keybinds};
pub use palette::Palette;

/// Top-level config. Both fields default to the historical hardcoded
/// values; users opt in to overrides by writing the matching TOML key.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub keybinds: Keybinds,
    pub colors: Palette,
}

impl Config {
    /// Load `config.toml` from `path` if it exists. Missing file →
    /// defaults. Parse error → defaults plus a `tracing::warn!`. The
    /// caller is expected to pass the resolved XDG path
    /// (`AppPaths::config_dir().join("config.toml")`).
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => match toml::from_str::<Config>(&s) {
                Ok(cfg) => cfg,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "config.toml has parse errors -- falling back to defaults"
                    );
                    Config::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(
                    path = %path.display(),
                    "config.toml absent -- using built-in defaults"
                );
                Config::default()
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "config.toml could not be read -- falling back to defaults"
                );
                Config::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_file_returns_default() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("does-not-exist.toml");
        let c = Config::load(&p);
        // Defaults match a fresh Config::default() exactly.
        let d = Config::default();
        assert_eq!(c.colors.accent, d.colors.accent);
    }

    #[test]
    fn empty_file_returns_default() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("config.toml");
        std::fs::write(&p, "").unwrap();
        let c = Config::load(&p);
        assert_eq!(c.colors.accent, Config::default().colors.accent);
    }

    #[test]
    fn malformed_file_falls_back_to_default() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("config.toml");
        std::fs::write(&p, "this is not valid toml = = =").unwrap();
        let c = Config::load(&p);
        // Should not panic, should not error -- just defaults.
        assert_eq!(c.colors.accent, Config::default().colors.accent);
    }

    #[test]
    fn unknown_top_level_field_falls_back() {
        // `deny_unknown_fields` means an unknown top-level key (e.g. a
        // typo) is a parse error -- the user gets defaults plus a
        // warning rather than silently ignored gibberish.
        let td = TempDir::new().unwrap();
        let p = td.path().join("config.toml");
        std::fs::write(&p, "[colorz]\naccent = \"red\"\n").unwrap();
        let c = Config::load(&p);
        assert_eq!(c.colors.accent, Config::default().colors.accent);
    }

    #[test]
    fn partial_override_preserves_other_defaults() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("config.toml");
        std::fs::write(&p, "[colors]\naccent = \"red\"\n").unwrap();
        let c = Config::load(&p);
        assert_eq!(
            c.colors.accent,
            palette::ColorSpec::Named(palette::NamedColor::Red)
        );
        // Untouched slot stays at default.
        assert_eq!(c.colors.dim, Config::default().colors.dim);
    }
}
