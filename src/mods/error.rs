//! Typed errors for the Modrinth integration module.
//!
//! Library-layer errors. Convert to `AppError` at the `execute_effects`
//! boundary in `src/tui/run.rs` (or surface directly via
//! `Action::ModInstallFailed { error: e.to_string(), .. }`).
//!
//! Variants enumerated in 08-PATTERNS.md §`src/mods/error.rs` deltas.

/// All failure modes for `ModrinthService` operations.
#[derive(Debug, thiserror::Error)]
pub enum ModrinthError {
    /// HTTP fetch failed (network / non-2xx status / body read).
    /// String body carries the URL and reqwest error message.
    #[error("Modrinth HTTP error: {0}")]
    Http(String),

    /// JSON parse of a Modrinth API response failed (unexpected shape).
    #[error("Modrinth response parse failed: {0}")]
    Parse(String),

    /// SHA-512 verification of a downloaded mod file failed (Pitfall 3 — case-insensitive compare).
    /// UI-SPEC line 676: "Downloaded file SHA-512 did not match Modrinth's manifest".
    #[error("SHA-512 mismatch for {url}: expected {expected}, got {got}")]
    Sha512Mismatch {
        url: String,
        expected: String,
        got: String,
    },

    /// BFS dep resolution found an `incompatible` dep that is currently installed.
    #[error("Dependency conflict: project {conflicting_project_id} is incompatible with installed mod (required by {requested_by})")]
    DependencyConflict {
        conflicting_project_id: String,
        requested_by: String,
    },

    /// No version of a required dep matches the instance's MC version + loader.
    #[error("No compatible version of project {project_id} for MC {mc} + loaders {loaders:?}")]
    NoCompatibleVersion {
        project_id: String,
        mc: String,
        loaders: Vec<String>,
    },

    /// Ledger lookup miss for toggle/uninstall.
    #[error("Mod not found in instance ledger: {0}")]
    ModNotFound(String),

    /// TOML decode failure on installed-mods.toml.
    #[error("Failed to parse installed-mods.toml: {0}")]
    LedgerParse(String),

    /// Modrinth returned 429 — rate limit (300 req/min anonymous).
    /// v1 surfaces only; no auto-retry per 08-RESEARCH.md line 174.
    #[error("Modrinth rate limit hit — retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    /// Defensive (Pitfall 5): mod file URL is null/empty/non-HTTPS.
    /// Phase 8 in practice always receives a valid URL; Phase 9 (CurseForge) reuses for `downloadUrl: null`.
    #[error("Mod file is not downloadable from Modrinth: {project_slug}")]
    FileNotDownloadable { project_slug: String },

    /// User cancelled install via Esc / CancellationToken (Pitfall 8).
    /// Treated as a clean return at the run.rs forwarder per Phase 6 precedent.
    #[error("Mod install cancelled")]
    Cancelled,

    /// Underlying I/O error (filesystem, atomic_write, fs::rename, fs::remove_file).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_display_contains_message() {
        let e =
            ModrinthError::Http("GET https://api.modrinth.com/v2/search: connect timeout".into());
        let s = e.to_string();
        assert!(s.contains("connect timeout"), "missing reqwest err: {s}");
        assert!(s.contains("api.modrinth.com"), "missing URL: {s}");
    }

    #[test]
    fn test_parse_display() {
        let e = ModrinthError::Parse("missing field 'sha512' at line 1".into());
        let s = e.to_string();
        assert!(s.contains("sha512"), "missing reason: {s}");
    }

    #[test]
    fn test_sha512_mismatch_display() {
        let e = ModrinthError::Sha512Mismatch {
            url: "https://cdn.modrinth.com/data/abc/sodium.jar".into(),
            expected: "aabbccdd".into(),
            got: "ffeeddcc".into(),
        };
        let s = e.to_string();
        assert!(s.contains("aabbccdd"), "expected hash missing: {s}");
        assert!(s.contains("ffeeddcc"), "got hash missing: {s}");
        assert!(s.contains("sodium.jar"), "url missing: {s}");
    }

    #[test]
    fn test_dependency_conflict_display() {
        let e = ModrinthError::DependencyConflict {
            conflicting_project_id: "AABBCCDD".into(),
            requested_by: "WXYZ1234".into(),
        };
        let s = e.to_string();
        assert!(s.contains("AABBCCDD"), "conflicting id missing: {s}");
        assert!(s.contains("WXYZ1234"), "requested_by missing: {s}");
    }

    #[test]
    fn test_no_compatible_version_display() {
        let e = ModrinthError::NoCompatibleVersion {
            project_id: "AABBCCDD".into(),
            mc: "1.20.4".into(),
            loaders: vec!["fabric".into()],
        };
        let s = e.to_string();
        assert!(s.contains("AABBCCDD"), "project_id missing: {s}");
        assert!(s.contains("1.20.4"), "mc version missing: {s}");
        assert!(s.contains("fabric"), "loader missing: {s}");
    }

    #[test]
    fn test_mod_not_found_display() {
        let e = ModrinthError::ModNotFound("AABBCCDD".into());
        let s = e.to_string();
        assert!(s.contains("AABBCCDD"), "mod_id missing: {s}");
    }

    #[test]
    fn test_ledger_parse_display() {
        let e = ModrinthError::LedgerParse("invalid type at line 5".into());
        let s = e.to_string();
        assert!(s.contains("installed-mods.toml"), "filename missing: {s}");
        assert!(s.contains("invalid type"), "reason missing: {s}");
    }

    #[test]
    fn test_rate_limited_display() {
        let e = ModrinthError::RateLimited {
            retry_after_secs: 42,
        };
        let s = e.to_string();
        assert!(s.contains("42"), "retry_after missing: {s}");
        assert!(s.contains("rate limit"), "headline missing: {s}");
    }

    #[test]
    fn test_file_not_downloadable_display() {
        let e = ModrinthError::FileNotDownloadable {
            project_slug: "sodium".into(),
        };
        let s = e.to_string();
        assert!(s.contains("sodium"), "slug missing: {s}");
        assert!(s.contains("not downloadable"), "headline missing: {s}");
    }

    #[test]
    fn test_cancelled_display() {
        let e = ModrinthError::Cancelled;
        assert_eq!(e.to_string(), "Mod install cancelled");
    }

    #[test]
    fn test_io_from_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let me: ModrinthError = io_err.into();
        let s = me.to_string();
        assert!(s.contains("I/O error"), "I/O headline missing: {s}");
        assert!(s.contains("denied"), "io message missing: {s}");
    }
}
