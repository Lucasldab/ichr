//! Post-install harvest: walk staging dir after installer succeeds, extract
//! the produced version JSON + library tree, and merge into ichr's
//! shared `libraries/` Maven layout + `versions/` tree.
//!
//! Atomicity: the merged version JSON is written via `atomic_write`
//! (Phase 2 tmp+rename pattern). The instance manifest's `loader` field
//! is written LAST by the caller (`LoaderService::install_loader`) -- never here.
//!
//! **07-RESEARCH.md Open Question 4 (harvest variation):** We use the
//! "any non-vanilla dir" rule (NOT a string-substring forge-name heuristic)
//! and validate the parsed JSON `id` against the discovered directory name.
//! Ambiguous multiple non-vanilla dirs → immediate HarvestFailed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::install::version_installer::LIB_CONCURRENCY;
use crate::loader::error::LoaderError;
use crate::loader::maven::maven_coord_to_path;
use crate::loader::types::LoaderLibrary;
use crate::mojang::cache::{atomic_write, verify_sha1};
use crate::persistence::paths::AppPaths;

/// One library entry harvested from the staging tree, ready to be
/// copied into the shared libraries Maven layout.
#[derive(Debug, Clone)]
pub struct HarvestedLibrary {
    /// Relative path under `libraries/` (e.g. `"net/minecraftforge/forge/1.20.1-47.4.20/forge-1.20.1-47.4.20.jar"`).
    pub maven_path: String,
    /// Absolute path inside the staging `libraries/` tree.
    pub source_path: PathBuf,
    /// Expected SHA-1 hex from the version JSON, if present.
    pub sha1: Option<String>,
}

/// Everything produced by `harvest_install`.
#[derive(Debug)]
pub struct HarvestedInstall {
    /// Version ID as recorded in the produced version JSON `id` field.
    pub version_id: String,
    /// Raw bytes of the produced version JSON.
    pub version_json_bytes: Vec<u8>,
    /// Libraries found in both the version JSON and the staging `libraries/` tree.
    pub libraries: Vec<HarvestedLibrary>,
}

/// Partial parse of the version JSON produced by the installer.
/// Only the fields needed for harvest validation are extracted.
#[derive(Deserialize)]
struct ProducedVersion {
    /// Used for JSON-vs-dir-name validation (Open Question 4).
    id: String,
    #[serde(default)]
    libraries: Vec<LoaderLibrary>,
}

/// Walk `<staging>/versions/` to find the produced loader version dir;
/// parse its JSON; identify the library tree under `<staging>/libraries/`.
/// Does NOT touch shared paths -- caller copies via `copy_libraries_into_shared`.
///
/// **Algorithm (07-RESEARCH.md Q4 fix -- "any non-MC dir" rule; no string-heuristic on loader names):**
/// 1. Collect all subdirectory names in `<staging>/versions/` whose name is NOT `vanilla_mc_id`.
/// 2. If `expected_loader_id` is Some and present in candidates → pick it (anchored fast path).
/// 3. Else if exactly one candidate → pick it (common case).
/// 4. Else if no candidates → HarvestFailed ("no non-vanilla version dir ...").
/// 5. Else (multiple and not anchored) → HarvestFailed ("ambiguous version dirs ...").
/// 6. Validate parsed JSON `id` against directory name; tolerate mismatch only if
///    JSON `id` matches the caller's `expected_loader_id`.
/// 7. Collect libraries present in `<staging>/libraries/`; warn-and-skip missing ones.
#[tracing::instrument(skip_all, fields(staging = %staging.display(), vanilla_mc_id = %vanilla_mc_id))]
pub async fn harvest_install(
    staging: &Path,
    expected_loader_id: Option<&str>,
    vanilla_mc_id: &str,
) -> Result<HarvestedInstall, LoaderError> {
    let versions_dir = staging.join("versions");
    let mut entries =
        tokio::fs::read_dir(&versions_dir)
            .await
            .map_err(|e| LoaderError::HarvestFailed {
                reason: format!("read {}: {e}", versions_dir.display()),
            })?;

    // Step 1: collect ALL subdirectories EXCEPT the vanilla MC dir.
    let mut candidates: Vec<String> = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| LoaderError::HarvestFailed {
            reason: format!("iterate {}: {e}", versions_dir.display()),
        })?
    {
        if !entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name_str = entry.file_name().to_string_lossy().to_string();
        if name_str == vanilla_mc_id {
            continue;
        }
        candidates.push(name_str);
    }

    // Step 2: pick the loader version_id by anchored / single / ambiguous rule.
    let version_id = match (expected_loader_id, candidates.as_slice()) {
        (Some(expected), c) if c.iter().any(|n| n == expected) => expected.to_string(),
        (_, [only]) => only.clone(),
        (_, []) => {
            return Err(LoaderError::HarvestFailed {
                reason: "no non-vanilla version dir produced by installer".into(),
            });
        }
        (_, many) => {
            return Err(LoaderError::HarvestFailed {
                reason: format!("ambiguous version dirs in staging: {many:?}"),
            });
        }
    };

    let version_json_path = versions_dir
        .join(&version_id)
        .join(format!("{version_id}.json"));
    let raw_bytes =
        tokio::fs::read(&version_json_path)
            .await
            .map_err(|e| LoaderError::HarvestFailed {
                reason: format!("read {}: {e}", version_json_path.display()),
            })?;
    let parsed: ProducedVersion =
        serde_json::from_slice(&raw_bytes).map_err(|e| LoaderError::HarvestFailed {
            reason: format!("parse {}: {e}", version_json_path.display()),
        })?;

    // Step 3 (Open Question 4 fix): validate JSON `id` field against directory name.
    if parsed.id != version_id {
        // Tolerate the mismatch only when the JSON `id` matches the caller's
        // expected_loader_id (installer is authoritative on the produced ID).
        let acceptable = matches!(expected_loader_id, Some(e) if parsed.id == e);
        if !acceptable {
            return Err(LoaderError::HarvestFailed {
                reason: format!(
                    "version JSON id {} does not match directory name {}",
                    parsed.id, version_id
                ),
            });
        }
        tracing::warn!(
            json_id = %parsed.id,
            dir_name = %version_id,
            "version JSON id differs from dir name; trusting JSON id per expected_loader_id"
        );
    }

    let staging_libs = staging.join("libraries");
    let mut libraries = Vec::with_capacity(parsed.libraries.len());
    for lib in &parsed.libraries {
        let maven_path = maven_coord_to_path(&lib.name)?;
        let source_path = staging_libs.join(&maven_path);
        if !tokio::fs::try_exists(&source_path).await.unwrap_or(false) {
            tracing::warn!(
                src = %source_path.display(),
                coord = %lib.name,
                "library declared in version JSON but not present in staging -- skipping"
            );
            continue;
        }
        libraries.push(HarvestedLibrary {
            maven_path,
            source_path,
            sha1: lib.sha1.clone(),
        });
    }

    Ok(HarvestedInstall {
        version_id,
        version_json_bytes: raw_bytes,
        libraries,
    })
}

/// Copy harvested libraries into `paths.libraries_dir()`. Idempotent --
/// files already present with matching SHA1 are skipped; otherwise written
/// via `atomic_write`. Bounded by `LIB_CONCURRENCY` (8) semaphore.
/// Cancellation-aware: returns `LoaderError::Cancelled` on token fire.
#[tracing::instrument(skip_all, fields(count = libraries.len()))]
pub async fn copy_libraries_into_shared(
    paths: &AppPaths,
    libraries: &[HarvestedLibrary],
    token: &CancellationToken,
) -> Result<(), LoaderError> {
    let sem = Arc::new(Semaphore::new(LIB_CONCURRENCY));
    let mut set = tokio::task::JoinSet::new();
    for lib in libraries {
        let lib = lib.clone();
        let sem = Arc::clone(&sem);
        let paths = paths.clone();
        let token = token.clone();
        set.spawn(async move {
            let _permit = sem
                .acquire_owned()
                .await
                .map_err(|e| LoaderError::HarvestFailed {
                    reason: format!("semaphore: {e}"),
                })?;
            if token.is_cancelled() {
                return Err(LoaderError::Cancelled);
            }
            copy_one_library(&paths, &lib).await
        });
    }
    let mut first_err: Option<LoaderError> = None;
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
                set.abort_all();
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(LoaderError::HarvestFailed {
                        reason: format!("join: {e}"),
                    });
                }
                set.abort_all();
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

async fn copy_one_library(paths: &AppPaths, lib: &HarvestedLibrary) -> Result<(), LoaderError> {
    let dst = paths.library_path(&lib.maven_path);

    // Skip if already present and SHA matches (or no SHA known and file exists).
    if tokio::fs::try_exists(&dst).await.unwrap_or(false) {
        if let Some(expected) = &lib.sha1 {
            let ok = verify_sha1(&dst, expected)
                .await
                .map_err(|e| LoaderError::HarvestFailed {
                    reason: format!("verify_sha1 {}: {e}", dst.display()),
                })?;
            if ok {
                return Ok(());
            }
            // Mismatch -- fall through to overwrite.
        } else {
            return Ok(());
        }
    }

    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| LoaderError::HarvestFailed {
                reason: format!("mkdir {}: {e}", parent.display()),
            })?;
    }
    let bytes =
        tokio::fs::read(&lib.source_path)
            .await
            .map_err(|e| LoaderError::HarvestFailed {
                reason: format!("read {}: {e}", lib.source_path.display()),
            })?;
    atomic_write(&dst, &bytes)
        .await
        .map_err(|e| LoaderError::HarvestFailed {
            reason: format!("write {}: {e}", dst.display()),
        })?;
    if let Some(expected) = &lib.sha1 {
        let ok = verify_sha1(&dst, expected)
            .await
            .map_err(|e| LoaderError::HarvestFailed {
                reason: format!("verify_sha1 {}: {e}", dst.display()),
            })?;
        if !ok {
            return Err(LoaderError::Sha1Mismatch {
                path: dst.display().to_string(),
                expected: expected.clone(),
                got: "<computed mismatch>".into(),
            });
        }
    }
    Ok(())
}

/// Atomic write of the merged version JSON bytes into the shared versions tree.
/// Uses `atomic_write` (tmp + rename -- Phase 2 pattern, Pitfall 7 prevention).
#[tracing::instrument(skip_all, fields(version_id = %version_id))]
pub async fn write_version_json(
    paths: &AppPaths,
    version_id: &str,
    bytes: &[u8],
) -> Result<(), LoaderError> {
    let dest = paths.version_json(version_id);
    atomic_write(&dest, bytes)
        .await
        .map_err(|e| LoaderError::ProfileWrite {
            path: dest.display().to_string(),
            reason: e.to_string(),
        })
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

    /// Build a minimal staging tree:
    ///   versions/<vanilla>/   (empty dir -- represents vanilla)
    ///   versions/<loader_id>/<loader_id>.json
    ///   libraries/              (empty dir)
    async fn make_staging_tree(base: &Path, vanilla_mc_id: &str, loader_id: &str, json: &str) {
        tokio::fs::create_dir_all(base.join("versions").join(vanilla_mc_id))
            .await
            .unwrap();
        tokio::fs::create_dir_all(base.join("versions").join(loader_id))
            .await
            .unwrap();
        tokio::fs::create_dir_all(base.join("libraries"))
            .await
            .unwrap();
        tokio::fs::write(
            base.join("versions")
                .join(loader_id)
                .join(format!("{loader_id}.json")),
            json.as_bytes(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_harvest_install_finds_forge_version_dir() {
        let td = TempDir::new().unwrap();
        let staging = td.path();
        let loader_id = "1.20.1-forge-47.4.20";
        let json = format!(r#"{{"id":"{loader_id}","libraries":[]}}"#);
        make_staging_tree(staging, "1.20.1", loader_id, &json).await;

        let h = harvest_install(staging, None, "1.20.1").await.unwrap();
        assert_eq!(h.version_id, loader_id);
        assert!(h.libraries.is_empty());
    }

    #[tokio::test]
    async fn test_harvest_install_finds_neoforge_version_dir() {
        let td = TempDir::new().unwrap();
        let staging = td.path();
        let loader_id = "neoforge-21.4.114";
        let json = format!(r#"{{"id":"{loader_id}","libraries":[]}}"#);
        make_staging_tree(staging, "1.21.4", loader_id, &json).await;

        let h = harvest_install(staging, None, "1.21.4").await.unwrap();
        assert_eq!(h.version_id, loader_id);
        assert!(h.libraries.is_empty());
    }

    #[tokio::test]
    async fn test_harvest_install_no_loader_dir_returns_harvest_failed() {
        let td = TempDir::new().unwrap();
        let staging = td.path();
        // Only the vanilla dir -- no loader dir.
        tokio::fs::create_dir_all(staging.join("versions").join("1.20.1"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(staging.join("libraries"))
            .await
            .unwrap();

        let err = harvest_install(staging, None, "1.20.1").await.unwrap_err();
        match err {
            LoaderError::HarvestFailed { reason } => {
                assert!(
                    reason.contains("no non-vanilla"),
                    "expected 'no non-vanilla' in reason: {reason}"
                );
            }
            other => panic!("expected HarvestFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_harvest_install_rejects_multiple_non_mc_dirs() {
        let td = TempDir::new().unwrap();
        let staging = td.path();
        // Two loader dirs -- ambiguous (no expected_loader_id to anchor).
        let id1 = "1.20.1-forge-47.4.20";
        let id2 = "1.20.1-forge-47.4.10";
        let j1 = format!(r#"{{"id":"{id1}","libraries":[]}}"#);
        let j2 = format!(r#"{{"id":"{id2}","libraries":[]}}"#);
        make_staging_tree(staging, "1.20.1", id1, &j1).await;
        tokio::fs::create_dir_all(staging.join("versions").join(id2))
            .await
            .unwrap();
        tokio::fs::write(
            staging
                .join("versions")
                .join(id2)
                .join(format!("{id2}.json")),
            j2.as_bytes(),
        )
        .await
        .unwrap();

        let err = harvest_install(staging, None, "1.20.1").await.unwrap_err();
        match err {
            LoaderError::HarvestFailed { reason } => {
                assert!(
                    reason.contains("ambiguous"),
                    "expected 'ambiguous' in reason: {reason}"
                );
            }
            other => panic!("expected HarvestFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_copy_libraries_into_shared_copies_and_skips_existing() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);

        // One library in staging.
        let coord = "org.ow2.asm:asm:9.7.1";
        let maven_path = crate::loader::maven::maven_coord_to_path(coord).unwrap();
        let source_path = td.path().join("staging-libs").join(&maven_path);
        tokio::fs::create_dir_all(source_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&source_path, b"FAKE_ASM_JAR")
            .await
            .unwrap();

        let lib = HarvestedLibrary {
            maven_path: maven_path.clone(),
            source_path: source_path.clone(),
            sha1: None,
        };
        let token = CancellationToken::new();

        // First call -- should copy.
        copy_libraries_into_shared(&paths, std::slice::from_ref(&lib), &token)
            .await
            .unwrap();
        let dst = paths.library_path(&maven_path);
        assert!(dst.exists(), "library must be copied");
        assert_eq!(tokio::fs::read(&dst).await.unwrap(), b"FAKE_ASM_JAR");

        // Second call -- should skip (file already exists, no SHA).
        copy_libraries_into_shared(&paths, &[lib], &token)
            .await
            .unwrap();
        // dst still exists with correct contents.
        assert_eq!(tokio::fs::read(&dst).await.unwrap(), b"FAKE_ASM_JAR");
    }

    #[tokio::test]
    async fn test_write_version_json_atomic_write_round_trip() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let version_id = "1.20.1-forge-47.4.20";
        let json_bytes = br#"{"id":"1.20.1-forge-47.4.20","inheritsFrom":"1.20.1"}"#;

        write_version_json(&paths, version_id, json_bytes)
            .await
            .unwrap();

        let dest = paths.version_json(version_id);
        let read_back = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(read_back, json_bytes);
    }
}
