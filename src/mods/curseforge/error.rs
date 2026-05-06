//! Typed errors for the CurseForge integration module.
//!
//! Library-layer errors. Convert to `AppError` at the `execute_effects`
//! boundary in `src/tui/run.rs` (or surface directly via
//! `Action::CfModInstallFailed { error: e.to_string(), web_url: ..., .. }`).
//!
//! Variants enumerated in 09-PATTERNS.md §`src/mods/curseforge/error.rs` deltas
//! (lines 188-198).

/// All failure modes for `CurseForgeService` operations.
#[derive(Debug, thiserror::Error)]
pub enum CurseForgeError {
    /// HTTP fetch failed (network / non-2xx status / body read).
    /// String body carries the URL and reqwest error message.
    #[error("CurseForge HTTP error: {0}")]
    Http(String),

    /// JSON parse of a CurseForge API response failed (unexpected shape).
    #[error("CurseForge response parse failed: {0}")]
    Parse(String),

    /// Hash verification of a downloaded mod file failed (Pitfall 3 — case-insensitive compare).
    /// `algo` discriminates SHA-1 (CurseForge default) vs SHA-256 (rare).
    #[error("{algo} mismatch for {url}: expected {expected}, got {got}")]
    ShaMismatch {
        algo: &'static str,
        url: String,
        expected: String,
        got: String,
    },

    /// CurseForge file is not downloadable: `downloadUrl` is null AND the
    /// dedicated `/files/{fileId}/download-url` endpoint also returned 403/404.
    /// Carries the constructed CurseForge web URL so the failed-install modal
    /// can show a copy-paste-able link directing the user to the browser.
    /// Per 09-RESEARCH.md §"downloadUrl null UX" lines 252-289.
    #[error("Mod file is not downloadable from CurseForge ({mod_slug}, file {file_id}): {web_url}")]
    FileNotDownloadable {
        web_url: String,
        mod_slug: String,
        file_id: u64,
    },

    /// `/v1/mods/{modId}` returned 404 — the mod has been deleted from CurseForge.
    /// Distinct from FileNotDownloadable per 09-RESEARCH.md Pitfall 5 line 287.
    #[error("CurseForge mod not found: {mod_id}")]
    ModNotFound { mod_id: u64 },

    /// API key resolution failed at `CurseForgeService::new()` — no env var,
    /// no config.toml entry, no compiled-in default. The launcher continues
    /// to function for everything else; the `F` keybind is silently disabled.
    /// Per 09-RESEARCH.md §"API Key Strategy" line 178 + Pitfall 1 line 936.
    #[error("No CurseForge API key configured. Set CURSEFORGE_API_KEY env var, or [api_keys] curseforge in config.toml.")]
    NoApiKey,

    /// CurseForge returned 429 — rate limit hit. v1 surfaces only; no auto-retry
    /// per 09-RESEARCH.md §Pitfall 7 line 972 (defer to v2).
    #[error("CurseForge rate limit hit — retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    /// User cancelled install via Esc / CancellationToken.
    /// Treated as a clean return at the run.rs forwarder per Phase 6 / Phase 8 precedent.
    #[error("CurseForge mod install cancelled")]
    Cancelled,

    /// Underlying I/O error (filesystem, atomic_write, fs::rename, fs::remove_file).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_display_contains_message_and_url() {
        let e = CurseForgeError::Http(
            "GET https://api.curseforge.com/v1/mods/search: connect timeout".into(),
        );
        let s = e.to_string();
        assert!(s.contains("connect timeout"), "missing reqwest err: {s}");
        assert!(s.contains("api.curseforge.com"), "missing URL: {s}");
    }

    #[test]
    fn test_parse_display() {
        let e = CurseForgeError::Parse("missing field 'data' at line 1".into());
        let s = e.to_string();
        assert!(s.contains("data"), "missing reason: {s}");
    }

    #[test]
    fn test_sha_mismatch_display_includes_algo() {
        let e = CurseForgeError::ShaMismatch {
            algo: "sha1",
            url: "https://edge.forgecdn.net/files/x/sodium.jar".into(),
            expected: "AABBCCDD".into(),
            got: "ffeeddcc".into(),
        };
        let s = e.to_string();
        assert!(s.contains("sha1"), "algo missing: {s}");
        assert!(s.contains("AABBCCDD"), "expected hash missing: {s}");
        assert!(s.contains("ffeeddcc"), "got hash missing: {s}");
        assert!(s.contains("sodium.jar"), "url missing: {s}");
    }

    #[test]
    fn test_file_not_downloadable_carries_web_url() {
        let e = CurseForgeError::FileNotDownloadable {
            web_url: "https://www.curseforge.com/minecraft/mc-mods/wonderful-world-mod/files/4567890"
                .into(),
            mod_slug: "wonderful-world-mod".into(),
            file_id: 4567890,
        };
        let s = e.to_string();
        assert!(s.contains("wonderful-world-mod"), "slug missing: {s}");
        assert!(s.contains("4567890"), "file_id missing: {s}");
        assert!(
            s.contains(
                "https://www.curseforge.com/minecraft/mc-mods/wonderful-world-mod/files/4567890"
            ),
            "web_url missing: {s}"
        );
    }

    #[test]
    fn test_mod_not_found_display() {
        let e = CurseForgeError::ModNotFound { mod_id: 443959 };
        let s = e.to_string();
        assert!(s.contains("443959"), "mod_id missing: {s}");
    }

    #[test]
    fn test_no_api_key_display_lists_overrides() {
        let e = CurseForgeError::NoApiKey;
        let s = e.to_string();
        assert!(s.contains("CURSEFORGE_API_KEY"), "env var name missing: {s}");
        assert!(s.contains("config.toml"), "config file mention missing: {s}");
    }

    #[test]
    fn test_rate_limited_display() {
        let e = CurseForgeError::RateLimited { retry_after_secs: 60 };
        let s = e.to_string();
        assert!(s.contains("60"), "retry_after missing: {s}");
        assert!(s.contains("rate limit"), "headline missing: {s}");
    }

    #[test]
    fn test_cancelled_display() {
        let e = CurseForgeError::Cancelled;
        assert_eq!(e.to_string(), "CurseForge mod install cancelled");
    }

    #[test]
    fn test_io_from_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let cfe: CurseForgeError = io_err.into();
        let s = cfe.to_string();
        assert!(s.contains("I/O error"), "I/O headline missing: {s}");
        assert!(s.contains("denied"), "io message missing: {s}");
    }
}
