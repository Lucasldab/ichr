//! ForgeWrapper.jar — bundled MS-PL binary asset extracted on first install.
//!
//! Vendored from https://github.com/ZekerZhayard/ForgeWrapper at tag `1.6.0`
//! (see `assets/forge_wrapper/README.md` for SHA-256 + license attribution).
//!
//! At install time, the wrapper is invoked via
//! `java -jar ForgeWrapper.jar --installer=<installer.jar> --instance=<staging>`
//! to bypass the Forge installer's `SimpleInstaller.main()` GUI/server-only branch
//! (07-RESEARCH.md §Critical Finding).
//!
//! At launch time, the wrapper's main class is injected into the version JSON
//! classpath via `FORGE_WRAPPER_MAIN_CLASS`.

use std::path::PathBuf;

use crate::loader::error::LoaderError;
use crate::persistence::paths::AppPaths;

/// Compile-time-embedded ForgeWrapper JAR (~29KB).
/// SHA-256 documented in `assets/forge_wrapper/README.md`.
pub const FORGE_WRAPPER_JAR: &[u8] =
    include_bytes!("../../assets/forge_wrapper/ForgeWrapper-mmc4.jar");

/// Expected SHA-256 (lowercase hex) — must match the file in
/// `assets/forge_wrapper/`. Updated by the vendor task.
///
/// [Rule 1 deviation note]: The plan used a fictional "mmc4" tag (~85KB);
/// actual ForgeWrapper 1.6.0 is 28,679 bytes. SHA-256 updated accordingly.
pub const FORGE_WRAPPER_SHA256: &str =
    "1dabf6d0fdb376fbae0f8db61de17ab73fb0d5b19b104d14d4eb29906a1c2cd6";

/// Filename used both for the embedded jar and the on-disk extracted copy.
pub const FORGE_WRAPPER_FILENAME: &str = "ForgeWrapper-mmc4.jar";

/// Fully-qualified main class to invoke from the wrapper JAR at launch time.
///
/// **Sourced from the upstream `1.6.0` release JAR contents at vendor time.**
/// Verified via `unzip -l ForgeWrapper-1.6.0.jar | grep installer/Main.class`
/// AND from the upstream README usage section.
///
/// This constant is injected into the Forge/NeoForge version JSON `libraries[]`
/// classpath entry at harvest time (07-03 plan), so the Phase 3 launcher can
/// invoke ForgeWrapper without modification to existing launch code.
pub const FORGE_WRAPPER_MAIN_CLASS: &str =
    "io.github.zekerzhayard.forgewrapper.installer.Main";

/// Path where the wrapper JAR lives after extraction:
/// `<cache_dir>/forge_wrapper/ForgeWrapper-mmc4.jar`.
pub fn wrapper_path(paths: &AppPaths) -> PathBuf {
    paths
        .cache_dir
        .join("forge_wrapper")
        .join(FORGE_WRAPPER_FILENAME)
}

/// Extract the embedded JAR to `wrapper_path` if absent. Idempotent.
/// On first call: writes via `atomic_write`. On subsequent calls: returns
/// the existing path without re-writing.
#[tracing::instrument(skip_all)]
pub async fn ensure_extracted(paths: &AppPaths) -> Result<PathBuf, LoaderError> {
    let dest = wrapper_path(paths);
    if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
        return Ok(dest);
    }
    crate::mojang::cache::atomic_write(&dest, FORGE_WRAPPER_JAR)
        .await
        .map_err(|e| LoaderError::StagingPopulate {
            reason: format!("ForgeWrapper extract to {}: {e}", dest.display()),
        })?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    #[test]
    fn test_embedded_bytes_non_empty() {
        // Rule 1 deviation: plan asserted >50_000 bytes but actual ForgeWrapper
        // 1.6.0 is 28,679 bytes. Threshold adjusted to >20_000 to match reality.
        assert!(
            FORGE_WRAPPER_JAR.len() > 20_000,
            "ForgeWrapper jar suspiciously small: {} bytes",
            FORGE_WRAPPER_JAR.len()
        );
    }

    #[test]
    fn test_embedded_bytes_starts_with_pk() {
        assert_eq!(
            &FORGE_WRAPPER_JAR[..2],
            b"PK",
            "ForgeWrapper jar missing ZIP/JAR magic bytes (PK)"
        );
    }

    #[test]
    fn test_sha256_constant_is_lowercase_hex_64_chars() {
        assert_eq!(FORGE_WRAPPER_SHA256.len(), 64, "SHA256 must be 64 hex chars");
        assert!(
            FORGE_WRAPPER_SHA256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "SHA256 must be lowercase hex"
        );
    }

    #[tokio::test]
    async fn test_ensure_extracted_creates_file_on_first_call() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let dest = wrapper_path(&paths);
        assert!(!dest.exists());
        let result = ensure_extracted(&paths).await.unwrap();
        assert_eq!(result, dest);
        assert!(dest.is_file());
        let size = tokio::fs::metadata(&dest).await.unwrap().len();
        assert_eq!(size as usize, FORGE_WRAPPER_JAR.len());
    }

    #[tokio::test]
    async fn test_ensure_extracted_is_idempotent() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        // First call writes
        let _ = ensure_extracted(&paths).await.unwrap();
        // Capture contents
        let before = tokio::fs::read(wrapper_path(&paths)).await.unwrap();
        // Second call must not error and must not change contents
        let _ = ensure_extracted(&paths).await.unwrap();
        let after = tokio::fs::read(wrapper_path(&paths)).await.unwrap();
        assert_eq!(before.len(), after.len());
    }
}
