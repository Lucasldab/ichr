//! ForgeWrapper.jar — bundled MS-PL binary asset extracted on first install.
//!
//! Vendored from https://github.com/ZekerZhayard/ForgeWrapper at tag `1.6.0`
//! (see `assets/forge_wrapper/README.md` for SHA-256 + license attribution).
//!
//! At launch time (Phase 12 — deferred), the wrapper's `Main` class is the
//! JVM entry point. Install time (Phase 7) does NOT use ForgeWrapper as of
//! 07.3-01 (GAP-7-A-v3): ForgeWrapper `Main` is a LAUNCH-time entry point
//! that reads Mojang launch argv (`--fml.mcVersion`, `--launchTarget`, etc.)
//! at Main.java:28 and cannot be invoked at install time (empty argv →
//! IndexOutOfBoundsException). The install path instead invokes the official
//! Forge/NeoForge installer JAR directly:
//!
//!     java -Djava.awt.headless=true
//!          -jar <installer.jar>
//!          --installClient <staging>      # Forge
//!          # OR
//!          --install-client <staging>     # NeoForge (canonical; both accepted)
//!
//! Launch-time wiring (Phase 12) supplies `FORGE_WRAPPER_MAIN_CLASS` as the
//! version JSON `mainClass` override.

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

/// Fully-qualified ForgeWrapper entry-point class — used ONLY at LAUNCH
/// time (Phase 12). Install-time invocation does NOT route through
/// ForgeWrapper as of 07.3-01 (see GAP-7-A-v3 in 07-UAT.md and
/// `.planning/debug/forge-installer-deep-bytecode-diagnosis.md`).
///
/// Main is the LAUNCH-time entry point: it parses Mojang launch argv
/// (`--fml.mcVersion`, `--launchTarget`, etc.) at `Main.java:28`
/// (verbatim from upstream pinned commit 3c6712d6:
/// `String mcVersion = argsList.get(argsList.indexOf("--fml.mcVersion") + 1);`),
/// reflectively calls `Installer.install()` as an install-on-first-launch
/// side effect, then transfers control to the modded game via
/// `mainClass.main(args)`. It has NO install-only mode and CANNOT be
/// invoked at install time (empty argv → IndexOutOfBoundsException).
///
/// At launch time (Phase 12 — deferred): used as the `mainClass` field
/// in the produced version JSON when launching modded MC. `Main` then
/// resolves modlauncher via `setupBootstrapLauncher` and reflectively
/// invokes the modded game.
///
/// At install time: NOT used. The install path invokes the official
/// Forge/NeoForge installer JAR directly via
/// `java -Djava.awt.headless=true -jar <installer> <install_flag> <staging>`
/// where install_flag is `--installClient` for Forge and `--install-client`
/// for NeoForge (NeoForge accepts both spellings; the canonical hyphen-form
/// matches upstream NeoForge documentation). See
/// `src/loader/service.rs::install_subprocess_loader` Step 4.
///
/// `#[allow(dead_code)]` is re-applied because no current code consumes
/// this constant (Phase 12 will be the first consumer). Round-2 (07.1-02)
/// removed the attribute when service.rs briefly consumed the constant;
/// 07.3-01 removes that consumer and the attribute returns.
#[allow(dead_code)]
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
