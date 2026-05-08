//! Staging directory population + cleanup for the Forge/NeoForge installer
//! subprocess.
//!
//! The installer expects an MMC-style instance directory: `launcher_profiles.json`
//! at the root, plus `versions/<mc>/<mc>.{json,jar}` pre-populated from the
//! mineltui Phase 2 cache. After the subprocess completes, `harvest.rs` walks
//! the produced layout. On cancel, `cleanup_staging` is best-effort.
//!
//! See PITFALL 2 (launcher_profiles.json missing) and PITFALL 6 (cleanup on
//! cancel) in 07-RESEARCH.md.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::loader::error::LoaderError;
use crate::persistence::paths::AppPaths;

/// Required skeleton — installers refuse to install client-side without
/// this file present (Pitfall 2).
pub const LAUNCHER_PROFILES_SKELETON: &[u8] =
    br#"{"profiles":{},"selectedProfile":"default","clientToken":""}"#;

static STAGING_COUNTER: AtomicU32 = AtomicU32::new(0);

#[derive(Debug)]
pub struct StagingDir {
    root: PathBuf,
    slug: String,
}

impl StagingDir {
    /// Create a uniquely-named staging directory under `{paths.data_dir}/staging/`.
    ///
    /// Uniqueness is guaranteed by combining the Unix timestamp with a
    /// per-process monotonic counter — two calls within the same second still
    /// produce distinct paths.
    #[tracing::instrument(skip_all, fields(slug = %slug))]
    pub async fn create(paths: &AppPaths, slug: &str) -> Result<Self, LoaderError> {
        let unix_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let seq = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = paths
            .data_dir
            .join("staging")
            .join(format!("{slug}-{unix_ts}-{seq:04x}"));
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|e| LoaderError::StagingPopulate {
                reason: format!("mkdir staging root {}: {e}", root.display()),
            })?;
        Ok(Self {
            root,
            slug: slug.to_string(),
        })
    }

    /// Root of this staging directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Slug used when creating this staging directory.
    pub fn slug(&self) -> &str {
        &self.slug
    }

    /// `<root>/libraries` — where the installer drops library JARs.
    pub fn libraries_dir(&self) -> PathBuf {
        self.root.join("libraries")
    }

    /// `<root>/versions` — where the installer drops version JSON + client jar.
    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }

    /// Pitfall 2 fix — write the skeleton `launcher_profiles.json` the
    /// installer requires to choose the client install branch.
    pub async fn write_launcher_profiles(&self) -> Result<(), LoaderError> {
        let path = self.root.join("launcher_profiles.json");
        tokio::fs::write(&path, LAUNCHER_PROFILES_SKELETON)
            .await
            .map_err(|e| LoaderError::StagingPopulate {
                reason: format!("write {}: {e}", path.display()),
            })?;
        Ok(())
    }

    /// Copy `versions/<mc>/<mc>.{json,jar}` from the shared Phase 2 cache
    /// into the staging tree so the installer can read the vanilla profile.
    #[tracing::instrument(skip_all, fields(mc = %mc_version))]
    pub async fn populate_vanilla(
        &self,
        paths: &AppPaths,
        mc_version: &str,
    ) -> Result<(), LoaderError> {
        let staging_versions = self.versions_dir().join(mc_version);
        tokio::fs::create_dir_all(&staging_versions)
            .await
            .map_err(|e| LoaderError::StagingPopulate {
                reason: format!("mkdir {}: {e}", staging_versions.display()),
            })?;
        tokio::fs::create_dir_all(&self.libraries_dir())
            .await
            .map_err(|e| LoaderError::StagingPopulate {
                reason: format!("mkdir {}: {e}", self.libraries_dir().display()),
            })?;

        let src_json = paths.version_json(mc_version);
        let dst_json = staging_versions.join(format!("{mc_version}.json"));
        tokio::fs::copy(&src_json, &dst_json)
            .await
            .map_err(|e| LoaderError::StagingPopulate {
                reason: format!(
                    "copy version JSON {} -> {}: {e}",
                    src_json.display(),
                    dst_json.display()
                ),
            })?;

        let src_jar = paths.version_jar(mc_version);
        let dst_jar = staging_versions.join(format!("{mc_version}.jar"));
        tokio::fs::copy(&src_jar, &dst_jar)
            .await
            .map_err(|e| LoaderError::StagingPopulate {
                reason: format!(
                    "copy client JAR {} -> {}: {e}",
                    src_jar.display(),
                    dst_jar.display()
                ),
            })?;

        Ok(())
    }
}

/// Best-effort staging dir cleanup. `NotFound` is silently ignored; other I/O
/// errors are logged via `tracing::warn!` but do NOT propagate (Pitfall 6).
#[tracing::instrument(skip_all, fields(staging = %staging.display()))]
pub async fn cleanup_staging(staging: &Path) {
    match tokio::fs::remove_dir_all(staging).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            tracing::warn!(path = %staging.display(), %e, "staging cleanup failed (best-effort)")
        }
    }
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

    #[tokio::test]
    async fn test_create_creates_unique_dir() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let a = StagingDir::create(&paths, "myslug").await.unwrap();
        let b = StagingDir::create(&paths, "myslug").await.unwrap();
        assert_ne!(
            a.root(),
            b.root(),
            "two creates must produce distinct paths"
        );
        assert!(a.root().is_dir());
        assert!(b.root().is_dir());
    }

    #[tokio::test]
    async fn test_write_launcher_profiles_skeleton_byte_exact() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let s = StagingDir::create(&paths, "test").await.unwrap();
        s.write_launcher_profiles().await.unwrap();
        let body = tokio::fs::read(s.root().join("launcher_profiles.json"))
            .await
            .unwrap();
        assert_eq!(body, LAUNCHER_PROFILES_SKELETON.to_vec());
    }

    #[tokio::test]
    async fn test_populate_vanilla_copies_both_files() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        // Pre-create fake vanilla cache
        let mc = "1.21.4";
        let json_src = paths.version_json(mc);
        let jar_src = paths.version_jar(mc);
        tokio::fs::create_dir_all(json_src.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&json_src, b"{\"id\":\"1.21.4\"}")
            .await
            .unwrap();
        tokio::fs::write(&jar_src, b"FAKE_JAR_BYTES").await.unwrap();

        let s = StagingDir::create(&paths, "test").await.unwrap();
        s.populate_vanilla(&paths, mc).await.unwrap();

        let dst_json = s.versions_dir().join(mc).join("1.21.4.json");
        let dst_jar = s.versions_dir().join(mc).join("1.21.4.jar");
        assert_eq!(
            tokio::fs::read(&dst_json).await.unwrap(),
            b"{\"id\":\"1.21.4\"}"
        );
        assert_eq!(tokio::fs::read(&dst_jar).await.unwrap(), b"FAKE_JAR_BYTES");
    }

    #[tokio::test]
    async fn test_cleanup_staging_is_idempotent_on_missing_path() {
        let p = std::path::PathBuf::from("/tmp/mineltui-staging-does-not-exist-xyz123");
        cleanup_staging(&p).await; // must not panic
    }

    #[tokio::test]
    async fn test_cleanup_staging_removes_existing_dir() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let s = StagingDir::create(&paths, "test").await.unwrap();
        let root = s.root().to_path_buf();
        assert!(root.is_dir());
        cleanup_staging(&root).await;
        assert!(!root.exists());
    }
}
