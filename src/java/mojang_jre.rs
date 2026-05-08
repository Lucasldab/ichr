//! Mojang JRE download pipeline.
//!
//! Fetches the Mojang JRE all.json index, selects the platform+variant entry,
//! downloads the per-variant manifest, and extracts every file with SHA1
//! verification. Atomic install via `.tmp` directory rename.
//!
//! # Hash rotation
//!
//! `DEFAULT_MOJANG_JRE_ALL_URL` is content-addressed by Mojang. If Mojang
//! rotates the hash (rare -- stable since at least 2023), update the constant
//! below and ship a patch. Set `ICHR_JRE_ALL_URL` env var at runtime
//! to override without recompiling.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha1::{Digest, Sha1};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::error::AppError;
use crate::mojang::cache::atomic_write;
use crate::persistence::paths::AppPaths;

/// Mojang JRE all.json -- stable content-addressed URL.
///
/// To update: replace the 40-char SHA1 hex segment with the new hash
/// from the Mojang launcher manifest. Override at runtime via
/// `ICHR_JRE_ALL_URL` env var (no recompile needed).
pub const DEFAULT_MOJANG_JRE_ALL_URL: &str =
    "https://piston-meta.mojang.com/v1/products/java-runtime/\
     2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Environment variable that overrides `DEFAULT_MOJANG_JRE_ALL_URL`.
pub const MOJANG_JRE_URL_ENV: &str = "ICHR_JRE_ALL_URL";

// ---------------------------------------------------------------------------
// Serde types -- Mojang JRE all.json
// ---------------------------------------------------------------------------

/// Top-level all.json: `{ "linux": { "java-runtime-delta": [MojangJreVariant] } }`
#[derive(Debug, Deserialize)]
pub struct MojangJreIndex(pub HashMap<String, HashMap<String, Vec<MojangJreVariant>>>);

/// Single variant entry within the all.json platform map.
#[derive(Debug, Deserialize)]
pub struct MojangJreVariant {
    pub manifest: MojangManifestRef,
    pub version: MojangJreVersionInfo,
}

/// Manifest reference (URL + SHA1) embedded in `MojangJreVariant`.
#[derive(Debug, Deserialize)]
pub struct MojangManifestRef {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

/// Version metadata embedded in `MojangJreVariant`.
#[derive(Debug, Deserialize)]
pub struct MojangJreVersionInfo {
    pub name: String,
    pub released: String,
}

// ---------------------------------------------------------------------------
// Serde types -- per-variant manifest
// ---------------------------------------------------------------------------

/// Per-variant manifest: `{ "files": { "bin/java": MojangFileEntry } }`
#[derive(Debug, Deserialize)]
pub struct MojangVariantManifest {
    pub files: HashMap<String, MojangFileEntry>,
}

/// A single entry in the per-variant manifest files map.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MojangFileEntry {
    File {
        downloads: MojangFileDownloads,
        #[serde(default)]
        executable: bool,
    },
    Directory {},
    Link {
        target: String,
    },
}

/// Download variants for a file entry. We always use `raw`.
#[derive(Debug, Deserialize)]
pub struct MojangFileDownloads {
    pub raw: MojangDownloadInfo,
    /// lzma is optional; we always prefer `raw` to avoid an lzma dependency.
    #[serde(default)]
    pub lzma: Option<MojangDownloadInfo>,
}

/// URL + SHA1 + size for a single download.
#[derive(Debug, Deserialize)]
pub struct MojangDownloadInfo {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

/// HTTP façade for Mojang JRE manifest and file downloads.
///
/// Mirrors `MojangClient` from `src/mojang/client.rs` -- same User-Agent,
/// gzip, 30s timeout.
#[derive(Debug, Clone)]
pub struct MojangJreClient {
    http: reqwest::Client,
}

impl MojangJreClient {
    /// Construct with the launcher's User-Agent and a 30s request timeout.
    pub fn new() -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .user_agent(crate::mojang::client::USER_AGENT)
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AppError::Http(format!("reqwest build (jre): {e}")))?;
        Ok(Self { http })
    }

    /// Fetch and parse the Mojang JRE all.json.
    ///
    /// URL priority:
    /// 1. `url_override` argument (for tests)
    /// 2. `ICHR_JRE_ALL_URL` environment variable
    /// 3. `DEFAULT_MOJANG_JRE_ALL_URL` constant
    #[tracing::instrument(skip_all)]
    pub async fn fetch_all_json(
        &self,
        url_override: Option<&str>,
    ) -> Result<MojangJreIndex, AppError> {
        let url = match url_override {
            Some(u) => u.to_owned(),
            None => std::env::var(MOJANG_JRE_URL_ENV)
                .unwrap_or_else(|_| DEFAULT_MOJANG_JRE_ALL_URL.to_owned()),
        };
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::Http(format!("GET jre all.json: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Http(format!("jre all.json status: {e}")))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Http(format!("jre all.json body: {e}")))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Select the first variant entry for `(platform_key, component)` from an index.
    ///
    /// Returns `None` if the platform or component is absent.
    pub fn select_variant<'a>(
        index: &'a MojangJreIndex,
        platform_key: &str,
        component: &str,
    ) -> Option<&'a MojangJreVariant> {
        index.0.get(platform_key)?.get(component)?.first()
    }

    /// Fetch and parse a per-variant manifest from `url`.
    ///
    /// Verifies the manifest bytes against `expected_sha1` before parsing.
    #[tracing::instrument(skip_all, fields(url))]
    pub async fn fetch_variant_manifest(
        &self,
        url: &str,
        expected_sha1: &str,
    ) -> Result<(MojangVariantManifest, Vec<u8>), AppError> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| AppError::Http(format!("GET variant manifest: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Http(format!("variant manifest status: {e}")))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Http(format!("variant manifest body: {e}")))?
            .to_vec();

        // Verify the manifest integrity before parsing
        let got = sha1_hex(&bytes);
        if !got.eq_ignore_ascii_case(expected_sha1) {
            return Err(AppError::Sha1Mismatch {
                target: url.to_string(),
                expected: expected_sha1.to_string(),
                got,
            });
        }

        let manifest: MojangVariantManifest = serde_json::from_slice(&bytes)?;
        Ok((manifest, bytes))
    }

    /// Install a Mojang JRE variant atomically into `paths.jre_dir(variant_id)`.
    ///
    /// - If `paths.jre_executable(variant_id)` already exists: skip (idempotent).
    /// - Extracts into `{jre_dir}.tmp`, then renames to `{jre_dir}` on success.
    /// - On any failure: removes `{jre_dir}.tmp` and propagates the error.
    /// - Downloads are parallelised with a `Semaphore(8)`.
    ///
    /// Returns the path to the java executable on success.
    #[tracing::instrument(skip_all, fields(variant_id))]
    pub async fn install_mojang_variant(
        &self,
        paths: &AppPaths,
        variant: &MojangJreVariant,
        variant_id: &str,
    ) -> Result<PathBuf, AppError> {
        let jre_dir = paths.jre_dir(variant_id);
        let exe_path = paths.jre_executable(variant_id);

        // Idempotency guard
        if tokio::fs::try_exists(&exe_path).await? {
            tracing::debug!(variant_id, "JRE already installed, skipping");
            return Ok(exe_path);
        }

        // Fetch + verify manifest
        let manifest_cache = paths.jre_manifest_cache(variant_id);
        let (manifest, manifest_bytes) = self
            .fetch_variant_manifest(&variant.manifest.url, &variant.manifest.sha1)
            .await?;

        // Cache the manifest bytes for debugging / future incremental resume
        // (parse from in-memory bytes above -- do NOT re-read from disk)
        let _ = atomic_write(&manifest_cache, &manifest_bytes).await;

        let tmp_dir = jre_dir.with_extension("tmp");
        // Clear any partial extraction from a previous aborted attempt
        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
        tokio::fs::create_dir_all(&tmp_dir).await?;

        // Separate entries by type so we create all dirs first
        let mut directories: Vec<String> = Vec::new();
        let mut files: Vec<(String, MojangDownloadInfo, bool)> = Vec::new();
        let mut links: Vec<(String, String)> = Vec::new();

        for (rel_path, entry) in manifest.files {
            // Path-traversal guard: reject any component that is `..` or absolute
            if !is_safe_rel_path(&rel_path) {
                let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                return Err(AppError::JavaExtractFailed {
                    dest: tmp_dir.clone(),
                    reason: format!("path traversal rejected: {rel_path}"),
                });
            }
            match entry {
                MojangFileEntry::Directory {} => directories.push(rel_path),
                MojangFileEntry::File {
                    downloads,
                    executable,
                } => {
                    files.push((rel_path, downloads.raw, executable));
                }
                MojangFileEntry::Link { target } => links.push((rel_path, target)),
            }
        }

        // Create directories first
        for rel in &directories {
            let dest = tmp_dir.join(rel);
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| AppError::JavaExtractFailed {
                    dest: dest.clone(),
                    reason: format!("create_dir_all: {e}"),
                })?;
        }

        // Download files in parallel, bounded by Semaphore(8)
        let sem = std::sync::Arc::new(Semaphore::new(8));
        let mut join_set: JoinSet<Result<(), AppError>> = JoinSet::new();

        for (rel_path, raw_info, executable) in files {
            let permit = sem.clone().acquire_owned().await.unwrap();
            let http = self.http.clone();
            let dest = tmp_dir.join(&rel_path);
            let url = raw_info.url.clone();
            let expected_sha1 = raw_info.sha1.clone();

            join_set.spawn(async move {
                let _permit = permit;

                let resp = http
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| AppError::Http(format!("GET {url}: {e}")))?
                    .error_for_status()
                    .map_err(|e| AppError::Http(format!("status {url}: {e}")))?;
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| AppError::Http(format!("body {url}: {e}")))?;

                // SHA1 verify before writing
                let got = sha1_hex(&bytes);
                if !got.eq_ignore_ascii_case(&expected_sha1) {
                    return Err(AppError::Sha1Mismatch {
                        target: url.clone(),
                        expected: expected_sha1,
                        got,
                    });
                }

                atomic_write(&dest, &bytes).await?;

                // Set executable bit on Unix
                if executable {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mut perms = tokio::fs::metadata(&dest).await?.permissions();
                        perms.set_mode(0o755);
                        tokio::fs::set_permissions(&dest, perms).await?;
                    }
                }

                Ok(())
            });
        }

        // Collect results; clean up on first error
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    join_set.abort_all();
                    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                    return Err(e);
                }
                Err(join_err) => {
                    join_set.abort_all();
                    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                    return Err(AppError::JavaExtractFailed {
                        dest: tmp_dir.clone(),
                        reason: format!("task panicked: {join_err}"),
                    });
                }
            }
        }

        // Create symlinks (Unix only; skip on Windows with a warning)
        for (rel_path, target) in links {
            let dest = tmp_dir.join(&rel_path);
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    AppError::JavaExtractFailed {
                        dest: parent.to_path_buf(),
                        reason: format!("create_dir_all for link parent: {e}"),
                    }
                })?;
            }

            #[cfg(unix)]
            {
                let dest_c = dest.clone();
                let target_c = target.clone();
                tokio::task::spawn_blocking(move || std::os::unix::fs::symlink(&target_c, &dest_c))
                    .await
                    .map_err(|e| AppError::JavaExtractFailed {
                        dest: dest.clone(),
                        reason: format!("spawn_blocking for symlink: {e}"),
                    })?
                    .map_err(|e| AppError::JavaExtractFailed {
                        dest: dest.clone(),
                        reason: format!("symlink({target}, {dest:?}): {e}"),
                    })?;
            }

            #[cfg(windows)]
            {
                tracing::warn!(
                    path = %rel_path,
                    target = %target,
                    "skipping Mojang link entry on Windows -- no manifests known to use links on Windows"
                );
            }
        }

        // Atomic rename: tmp -> final
        if let Err(e) = tokio::fs::rename(&tmp_dir, &jre_dir).await {
            let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
            return Err(AppError::JavaExtractFailed {
                dest: jre_dir,
                reason: format!("rename from .tmp: {e}"),
            });
        }

        Ok(exe_path)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute SHA1 hex of `bytes` (lowercase, 40 chars).
fn sha1_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

/// Return `true` if every component of `rel` is a `Normal` segment
/// (i.e. no `..`, no absolute root, no prefix on Windows).
fn is_safe_rel_path(rel: &str) -> bool {
    Path::new(rel)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::GET;
    use httpmock::MockServer;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_client() -> MojangJreClient {
        MojangJreClient::new().expect("client build")
    }

    fn make_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    // -----------------------------------------------------------------------
    // Task 1: all.json fetch + variant selection
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_all_json_snippet() {
        let raw = include_str!("../../tests/fixtures/java/mojang_all_snippet.json");
        let index: MojangJreIndex = serde_json::from_str(raw).expect("parse fixture");
        let linux = index.0.get("linux").expect("linux key");
        assert!(linux.contains_key("java-runtime-delta"), "delta missing");
        assert!(
            linux.contains_key("java-runtime-epsilon"),
            "epsilon missing"
        );
        let delta = linux.get("java-runtime-delta").unwrap();
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].version.name, "21.0.7");
    }

    #[tokio::test]
    async fn test_fetch_all_json_with_mock() {
        let server = MockServer::start();
        let body = include_str!("../../tests/fixtures/java/mojang_all_snippet.json");
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/all.json");
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });
        let client = make_client();
        let url = format!("{}/all.json", server.base_url());
        let index = client.fetch_all_json(Some(&url)).await.expect("fetch");
        assert!(index.0.contains_key("linux"), "linux key present");
    }

    #[test]
    fn test_select_variant_found() {
        let raw = include_str!("../../tests/fixtures/java/mojang_all_snippet.json");
        let index: MojangJreIndex = serde_json::from_str(raw).unwrap();
        let v = MojangJreClient::select_variant(&index, "linux", "java-runtime-delta");
        assert!(v.is_some());
        assert_eq!(v.unwrap().version.name, "21.0.7");
    }

    #[test]
    fn test_select_variant_missing_component() {
        let raw = include_str!("../../tests/fixtures/java/mojang_all_snippet.json");
        let index: MojangJreIndex = serde_json::from_str(raw).unwrap();
        let v = MojangJreClient::select_variant(&index, "linux", "java-runtime-nonexistent");
        assert!(v.is_none());
    }

    #[test]
    fn test_select_variant_missing_platform() {
        let raw = include_str!("../../tests/fixtures/java/mojang_all_snippet.json");
        let index: MojangJreIndex = serde_json::from_str(raw).unwrap();
        let v = MojangJreClient::select_variant(&index, "mac-os", "java-runtime-delta");
        assert!(v.is_none());
    }

    #[tokio::test]
    async fn test_env_override_read() {
        let server = MockServer::start();
        let body = include_str!("../../tests/fixtures/java/mojang_all_snippet.json");
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/override-all.json");
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });

        let override_url = format!("{}/override-all.json", server.base_url());
        // Save + set + restore env var
        let prior = std::env::var(MOJANG_JRE_URL_ENV).ok();
        std::env::set_var(MOJANG_JRE_URL_ENV, &override_url);

        let client = make_client();
        let result = client.fetch_all_json(None).await;

        // Restore
        match prior {
            Some(v) => std::env::set_var(MOJANG_JRE_URL_ENV, v),
            None => std::env::remove_var(MOJANG_JRE_URL_ENV),
        }

        let index = result.expect("env override fetch");
        assert!(index.0.contains_key("linux"));
    }

    // -----------------------------------------------------------------------
    // Task 2: manifest fetch + extraction
    // -----------------------------------------------------------------------

    fn fixture_bytes() -> &'static [u8] {
        b"fixture-java-bin\n"
    }

    // Used only by the linux-only + unix-only tests below
    // (test_install_extracts_file_with_sha1_verify, test_install_symlink_on_linux).
    // On Windows both callers are gated off → helper is dead. Match scope.
    #[cfg(unix)]
    fn make_manifest_body(server_base: &str) -> String {
        let sha1 = crate::mojang::cache::sha1_hex_of_bytes(fixture_bytes());
        let template = include_str!("../../tests/fixtures/java/mojang_variant_manifest.json");
        template
            .replace("__SHA1_OF_FIXTURE_BYTES__", &sha1)
            .replace("PLACEHOLDER", server_base)
    }

    fn make_all_json_body(server_base: &str, manifest_sha1: &str) -> String {
        include_str!("../../tests/fixtures/java/mojang_all_snippet.json")
            .replace("aaaa1234aaaa1234aaaa1234aaaa1234aaaa1234", manifest_sha1)
            .replace("PLACEHOLDER", server_base)
    }

    // Linux-only: select_variant("linux", ...) + asserts `bin/java` (no .exe)
    // is extracted. install_mojang_variant returns a host-OS-conditional path
    // (`bin/java.exe` on Windows), so on Windows host the existence assert
    // looks for `.exe` while the Linux fixture wrote `bin/java`. A Windows
    // counterpart would need a separate fixture with `bin/java.exe` entry.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_install_extracts_file_with_sha1_verify() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let manifest_body = make_manifest_body(&server.base_url());
        let manifest_sha1 = crate::mojang::cache::sha1_hex_of_bytes(manifest_body.as_bytes());
        let all_json_body = make_all_json_body(&server.base_url(), &manifest_sha1);

        let _m_all = server.mock(|when, then| {
            when.method(GET).path("/all.json");
            then.status(200).body(all_json_body.clone());
        });
        let _m_manifest = server.mock(|when, then| {
            when.method(GET).path("/manifest-delta.json");
            then.status(200).body(manifest_body.clone());
        });
        let _m_file = server.mock(|when, then| {
            when.method(GET).path("/bin/java");
            then.status(200).body(fixture_bytes());
        });

        let all_url = format!("{}/all.json", server.base_url());
        let client = make_client();
        let index = client.fetch_all_json(Some(&all_url)).await.unwrap();
        let variant = MojangJreClient::select_variant(&index, "linux", "java-runtime-delta")
            .expect("variant");

        let exe = client
            .install_mojang_variant(&paths, variant, "java-runtime-delta")
            .await
            .expect("install");

        assert!(exe.exists(), "executable should exist at {exe:?}");

        let jre_dir = paths.jre_dir("java-runtime-delta");
        let file_path = jre_dir.join("bin/java");
        assert!(file_path.exists(), "bin/java should exist");
        let content = std::fs::read(&file_path).unwrap();
        assert_eq!(content, fixture_bytes());

        // .tmp should be gone
        let tmp_dir = jre_dir.with_extension("tmp");
        assert!(!tmp_dir.exists(), ".tmp dir should be cleaned up");

        // Executable bit on Linux
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file_path).unwrap().permissions().mode();
            assert_ne!(mode & 0o111, 0, "executable bit should be set");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_install_symlink_on_linux() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let manifest_body = make_manifest_body(&server.base_url());
        let manifest_sha1 = crate::mojang::cache::sha1_hex_of_bytes(manifest_body.as_bytes());
        let all_json_body = make_all_json_body(&server.base_url(), &manifest_sha1);

        let _m_all = server.mock(|when, then| {
            when.method(GET).path("/all.json");
            then.status(200).body(all_json_body);
        });
        let _m_manifest = server.mock(|when, then| {
            when.method(GET).path("/manifest-delta.json");
            then.status(200).body(manifest_body);
        });
        let _m_file = server.mock(|when, then| {
            when.method(GET).path("/bin/java");
            then.status(200).body(fixture_bytes());
        });

        let all_url = format!("{}/all.json", server.base_url());
        let client = make_client();
        let index = client.fetch_all_json(Some(&all_url)).await.unwrap();
        let variant = MojangJreClient::select_variant(&index, "linux", "java-runtime-delta")
            .expect("variant");

        client
            .install_mojang_variant(&paths, variant, "java-runtime-delta")
            .await
            .expect("install");

        let link_path = paths.jre_dir("java-runtime-delta").join("legal/LICENSE");
        let meta = std::fs::symlink_metadata(&link_path).expect("link metadata");
        assert!(
            meta.file_type().is_symlink(),
            "legal/LICENSE should be a symlink"
        );
    }

    #[tokio::test]
    async fn test_install_skips_if_executable_exists() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        // Pre-create the executable so the idempotency check fires
        let exe = paths.jre_executable("java-runtime-delta");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"fake").unwrap();

        // This mock should NEVER be hit
        let network_mock = server.mock(|when, then| {
            when.method(GET).path("/all.json");
            then.status(200).body("{}");
        });

        // Build a fake variant pointing at the mock (should not be reached)
        let all_url = format!("{}/all.json", server.base_url());
        let raw = include_str!("../../tests/fixtures/java/mojang_all_snippet.json")
            .replace("PLACEHOLDER", &server.base_url());
        let index: MojangJreIndex = serde_json::from_str(&raw).unwrap();
        let variant = MojangJreClient::select_variant(&index, "linux", "java-runtime-delta")
            .expect("variant");

        let client = make_client();
        let _ = all_url; // suppress unused warning
        let result = client
            .install_mojang_variant(&paths, variant, "java-runtime-delta")
            .await;
        assert!(result.is_ok(), "idempotent install should return Ok");

        network_mock.assert_calls(0);
    }

    #[tokio::test]
    async fn test_install_sha1_mismatch_cleans_tmp() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        // Build manifest with a valid SHA1, but serve wrong bytes
        let wrong_bytes = b"WRONG_BYTES_XXXX";
        let correct_sha1 = crate::mojang::cache::sha1_hex_of_bytes(fixture_bytes());
        // manifest references correct sha1, but server serves wrong bytes
        let manifest_body = {
            let template = include_str!("../../tests/fixtures/java/mojang_variant_manifest.json");
            template
                .replace("__SHA1_OF_FIXTURE_BYTES__", &correct_sha1)
                .replace("PLACEHOLDER", &server.base_url())
        };
        let manifest_sha1 = crate::mojang::cache::sha1_hex_of_bytes(manifest_body.as_bytes());
        let all_json_body = make_all_json_body(&server.base_url(), &manifest_sha1);

        let _m_all = server.mock(|when, then| {
            when.method(GET).path("/all.json");
            then.status(200).body(all_json_body);
        });
        let _m_manifest = server.mock(|when, then| {
            when.method(GET).path("/manifest-delta.json");
            then.status(200).body(manifest_body);
        });
        // Serve WRONG bytes -- SHA1 mismatch
        let _m_file = server.mock(|when, then| {
            when.method(GET).path("/bin/java");
            then.status(200).body(wrong_bytes.as_ref());
        });

        let all_url = format!("{}/all.json", server.base_url());
        let client = make_client();
        let index = client.fetch_all_json(Some(&all_url)).await.unwrap();
        let variant = MojangJreClient::select_variant(&index, "linux", "java-runtime-delta")
            .expect("variant");

        let result = client
            .install_mojang_variant(&paths, variant, "java-runtime-delta")
            .await;

        assert!(result.is_err(), "should fail on SHA1 mismatch");
        match result.unwrap_err() {
            AppError::Sha1Mismatch { .. } => {}
            other => panic!("expected Sha1Mismatch, got: {other:?}"),
        }

        // .tmp dir must be cleaned up
        let tmp_dir = paths.jre_dir("java-runtime-delta").with_extension("tmp");
        assert!(!tmp_dir.exists(), ".tmp must be removed on failure");
    }

    #[tokio::test]
    async fn test_install_path_traversal_rejected() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        // Build a manifest with a path-traversal entry
        let traversal_manifest = r#"{"files": {"../evil/x": {"type": "directory"}}}"#;
        let manifest_sha1 = crate::mojang::cache::sha1_hex_of_bytes(traversal_manifest.as_bytes());
        let all_json_body = make_all_json_body(&server.base_url(), &manifest_sha1);

        let _m_all = server.mock(|when, then| {
            when.method(GET).path("/all.json");
            then.status(200).body(all_json_body);
        });
        let _m_manifest = server.mock(|when, then| {
            when.method(GET).path("/manifest-delta.json");
            then.status(200).body(traversal_manifest);
        });

        let all_url = format!("{}/all.json", server.base_url());
        let client = make_client();
        let index = client.fetch_all_json(Some(&all_url)).await.unwrap();
        let variant = MojangJreClient::select_variant(&index, "linux", "java-runtime-delta")
            .expect("variant");

        let result = client
            .install_mojang_variant(&paths, variant, "java-runtime-delta")
            .await;

        assert!(result.is_err(), "path traversal must be rejected");
    }
}
