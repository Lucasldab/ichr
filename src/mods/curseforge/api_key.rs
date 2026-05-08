//! CurseForge API key resolver -- PURE 4-tier precedence chain.
//!
//! No I/O, no async -- every test passes literal Some/None values without
//! touching std::env or the filesystem. The thin runtime wrapper that
//! reads env+config lives at the call site (`CurseForgeService::new`),
//! which then hands the three Option strings to `resolve_api_key`.
//!
//! Precedence (per 09-RESEARCH.md §"API Key Strategy" lines 168-218):
//! 1. CURSEFORGE_API_KEY env var (runtime override)
//! 2. [api_keys] curseforge in ~/.config/ichr/config.toml
//! 3. option_env!("ICHR_CURSEFORGE_API_KEY_DEFAULT") compiled-in
//! 4. Err(ApiKeyError::NoApiKey)
//!
//! Empty strings at any tier are treated as absent (skipped to the next tier).
//! Per 09-RESEARCH.md §Pitfall 1 lines 936-940: the launcher MUST NOT crash
//! at startup when no key is configured -- `CurseForgeService::new` returns
//! Ok with `api_key_present=false` and the F keybind is silently disabled.
//!
//! SECURITY INVARIANT (09-RESEARCH.md §Pitfall 6 lines 966-970): the api_key
//! value is NEVER passed to a structured-log macro field. CI grep guards
//! enforce this -- see 09-02-PLAN.md acceptance criteria.

use thiserror::Error;

/// Compile-time injected default key. CI sets ICHR_CURSEFORGE_API_KEY_DEFAULT
/// before `cargo build`; local builds without the env var get `None` and the
/// user must supply the key via runtime sources.
/// Per 09-RESEARCH.md §"API Key Strategy" line 186.
pub const COMPILED_IN_DEFAULT: Option<&str> = option_env!("ICHR_CURSEFORGE_API_KEY_DEFAULT");

/// API-key resolution failure. Single variant -- the caller surfaces it as
/// `api_key_present=false` rather than crashing the launcher.
#[derive(Debug, Error)]
pub enum ApiKeyError {
    #[error("No CurseForge API key configured. Set CURSEFORGE_API_KEY env var, or [api_keys] curseforge in config.toml.")]
    NoApiKey,
}

/// Pure precedence resolver. All inputs are `Option<&str>`; empty strings
/// at any tier are treated as absent.
/// Per 09-RESEARCH.md lines 198-211.
pub fn resolve_api_key(
    env_var: Option<&str>,
    config_value: Option<&str>,
    compiled_in_default: Option<&str>,
) -> Result<String, ApiKeyError> {
    let pick = |s: &str| {
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    };
    if let Some(k) = env_var.and_then(pick) {
        return Ok(k);
    }
    if let Some(k) = config_value.and_then(pick) {
        return Ok(k);
    }
    if let Some(k) = compiled_in_default.and_then(pick) {
        return Ok(k);
    }
    Err(ApiKeyError::NoApiKey)
}

/// Thin runtime wrapper used by `CurseForgeService::new()`. Reads the env var
/// directly; the caller is responsible for parsing config.toml and supplying
/// the value (avoids coupling api_key.rs to the AppConfig schema).
pub fn resolve_runtime(config_value: Option<&str>) -> Result<String, ApiKeyError> {
    let env_value = std::env::var("CURSEFORGE_API_KEY").ok();
    resolve_api_key(env_value.as_deref(), config_value, COMPILED_IN_DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_wins_over_config_and_default() {
        let r = resolve_api_key(Some("env-key"), Some("config-key"), Some("default-key")).unwrap();
        assert_eq!(r, "env-key");
    }

    #[test]
    fn test_config_wins_when_no_env() {
        let r = resolve_api_key(None, Some("config-key"), Some("default-key")).unwrap();
        assert_eq!(r, "config-key");
    }

    #[test]
    fn test_default_wins_when_no_env_and_no_config() {
        let r = resolve_api_key(None, None, Some("default-key")).unwrap();
        assert_eq!(r, "default-key");
    }

    #[test]
    fn test_all_empty_returns_no_api_key_error() {
        let r = resolve_api_key(None, None, None);
        assert!(matches!(r, Err(ApiKeyError::NoApiKey)));
    }

    #[test]
    fn test_empty_string_at_env_skipped_to_config() {
        // Empty CURSEFORGE_API_KEY env var must NOT win over a non-empty config value.
        let r = resolve_api_key(Some(""), Some("config-key"), Some("default-key")).unwrap();
        assert_eq!(r, "config-key");
    }

    #[test]
    fn test_empty_string_at_config_skipped_to_default() {
        let r = resolve_api_key(None, Some(""), Some("default-key")).unwrap();
        assert_eq!(r, "default-key");
    }

    #[test]
    fn test_empty_string_at_default_returns_no_api_key() {
        let r = resolve_api_key(None, None, Some(""));
        assert!(matches!(r, Err(ApiKeyError::NoApiKey)));
    }

    #[test]
    fn test_empty_string_at_every_tier_returns_no_api_key() {
        let r = resolve_api_key(Some(""), Some(""), Some(""));
        assert!(matches!(r, Err(ApiKeyError::NoApiKey)));
    }

    #[test]
    fn test_no_api_key_error_message_names_env_var_and_config() {
        let s = ApiKeyError::NoApiKey.to_string();
        assert!(
            s.contains("CURSEFORGE_API_KEY"),
            "env var name in message: {s}"
        );
        assert!(
            s.contains("config.toml"),
            "config file mention in message: {s}"
        );
    }
}
