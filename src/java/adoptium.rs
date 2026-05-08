//! Adoptium fallback JRE download pipeline.
//!
//! Fetches the Adoptium v3 latest-asset endpoint for (major, arch, os),
//! downloads the referenced archive, verifies SHA-256, and extracts it
//! inside `spawn_blocking` (stripping the archive's top-level prefix
//! directory so `bin/java` lands at `{jre_dir}/bin/java`).
//!
//! # Endpoint
//!
//! ```text
//! GET https://api.adoptium.net/v3/assets/latest/{major}/hotspot
//!     ?architecture={arch}&heap_size=normal&image_type=jre&os={os}&vendor=eclipse
//! ```
//!
//! Override the base URL for tests via `ICHR_ADOPTIUM_BASE_URL`.

use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::domain::platform::{Arch, OsName};
use crate::error::AppError;
use crate::java::mapping;
use crate::persistence::paths::AppPaths;

/// Default Adoptium API base URL.
pub const DEFAULT_ADOPTIUM_BASE: &str = "https://api.adoptium.net";

/// Environment variable that overrides `DEFAULT_ADOPTIUM_BASE` for testing.
pub const ADOPTIUM_BASE_URL_ENV: &str = "ICHR_ADOPTIUM_BASE_URL";

// ---------------------------------------------------------------------------
// Serde types -- Adoptium API response
// ---------------------------------------------------------------------------

/// Top-level Adoptium asset entry (one element of the response array).
///
/// Endpoint returns a JSON array; we always use `[0]`.
/// Verified against <https://api.adoptium.net/v3/assets/latest/21/hotspot> (2026-04-20).
#[derive(Debug, Deserialize)]
pub struct AdoptiumRelease {
    pub binary: AdoptiumBinary,
    pub version: AdoptiumVersion,
    pub release_name: String,
}

/// Binary section of an Adoptium release.
#[derive(Debug, Deserialize)]
pub struct AdoptiumBinary {
    pub package: AdoptiumPackage,
}

/// Package metadata -- download URL + SHA-256 checksum.
#[derive(Debug, Deserialize)]
pub struct AdoptiumPackage {
    /// Direct download URL for the `.tar.gz` (Linux) or `.zip` (Windows).
    pub link: String,
    /// SHA-256 hex digest of the archive (lowercase or uppercase -- compared case-insensitively).
    pub checksum: String,
    pub size: u64,
}

/// Version metadata within an Adoptium release.
#[derive(Debug, Deserialize)]
pub struct AdoptiumVersion {
    pub major: u32,
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

/// HTTP façade for Adoptium API requests.
///
/// Mirrors `MojangJreClient` -- same User-Agent, gzip, 30s timeout.
/// The base URL can be overridden at construction time via the
/// `ICHR_ADOPTIUM_BASE_URL` environment variable (for httpmock in tests).
#[derive(Debug, Clone)]
pub struct AdoptiumClient {
    http: reqwest::Client,
    base_url: String,
}

impl AdoptiumClient {
    /// Construct with the launcher's User-Agent and a 30s request timeout.
    ///
    /// The base URL defaults to `DEFAULT_ADOPTIUM_BASE` but is overridden
    /// by `ICHR_ADOPTIUM_BASE_URL` if set.
    pub fn new() -> Result<Self, AppError> {
        let base_url = std::env::var(ADOPTIUM_BASE_URL_ENV)
            .unwrap_or_else(|_| DEFAULT_ADOPTIUM_BASE.to_owned());
        Self::new_with_base_url(base_url)
    }

    /// Construct with an explicit base URL.
    ///
    /// Used in tests to avoid global env-var mutation -- pass the httpmock
    /// server's base URL directly.
    pub fn new_with_base_url(base_url: impl Into<String>) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .user_agent(crate::mojang::client::USER_AGENT)
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AppError::Http(format!("reqwest build (adoptium): {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.into(),
        })
    }

    /// Fetch the latest Adoptium JRE release for `(major, arch, os)`.
    ///
    /// Returns the first element of the JSON array. An empty array is treated
    /// as "no release available" and returns `AppError::JavaDownloadFailed`.
    #[tracing::instrument(skip_all, fields(major))]
    pub async fn fetch_latest_release(
        &self,
        major: u32,
        arch: Arch,
        os: OsName,
    ) -> Result<AdoptiumRelease, AppError> {
        let arch_str = mapping::adoptium_arch_str(arch);
        let os_str = mapping::adoptium_os_str(os);
        let url = format!(
            "{}/v3/assets/latest/{major}/hotspot\
             ?architecture={arch_str}&heap_size=normal&image_type=jre&os={os_str}&vendor=eclipse",
            self.base_url
        );

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::Http(format!("GET adoptium latest: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Http(format!("adoptium status: {e}")))?;

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Http(format!("adoptium body: {e}")))?;

        let releases: Vec<AdoptiumRelease> = serde_json::from_slice(&bytes)?;

        releases
            .into_iter()
            .next()
            .ok_or_else(|| AppError::JavaDownloadFailed {
                variant: format!("adoptium-{major}"),
                reason: "no release found for this platform".into(),
            })
    }

    /// Download `url` and verify its SHA-256 against `expected_sha256`.
    ///
    /// Returns the raw archive bytes on success.
    /// Returns `AppError::JavaDownloadFailed` if the digest does not match.
    #[tracing::instrument(skip_all)]
    pub async fn download_verified(
        &self,
        url: &str,
        expected_sha256: &str,
        variant_id: &str,
    ) -> Result<Vec<u8>, AppError> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| AppError::Http(format!("GET {url}: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Http(format!("status {url}: {e}")))?;

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Http(format!("body {url}: {e}")))?
            .to_vec();

        // SHA-256 verify -- Adoptium uses SHA-256 (not SHA-1)
        let got = sha256_hex(&bytes);
        if !got.eq_ignore_ascii_case(expected_sha256) {
            return Err(AppError::JavaDownloadFailed {
                variant: variant_id.to_string(),
                reason: format!("sha256 mismatch: expected {expected_sha256} got {got}"),
            });
        }

        Ok(bytes)
    }

    /// Install an Adoptium JRE for Java `major` into `paths.jre_dir("adoptium-{major}")`.
    ///
    /// - If `paths.jre_executable("adoptium-{major}")` already exists: skip (idempotent).
    /// - Fetches the latest release via the Adoptium API.
    /// - Downloads and SHA-256-verifies the archive.
    /// - Extracts into `{jre_dir}.tmp` (inside `spawn_blocking`), then renames atomically.
    /// - On any failure: removes `{jre_dir}.tmp` and propagates the error.
    ///
    /// Returns the path to the java executable on success.
    #[tracing::instrument(skip_all, fields(major))]
    pub async fn install_adoptium(
        &self,
        paths: &AppPaths,
        major: u32,
        arch: Arch,
        os: OsName,
    ) -> Result<PathBuf, AppError> {
        let variant_id = format!("adoptium-{major}");
        let exe_path = paths.jre_executable(&variant_id);

        // Idempotency guard
        if tokio::fs::try_exists(&exe_path).await.unwrap_or(false) {
            tracing::debug!(variant_id, "Adoptium JRE already installed, skipping");
            return Ok(exe_path);
        }

        let release = self.fetch_latest_release(major, arch, os).await?;
        let bytes = self
            .download_verified(
                &release.binary.package.link,
                &release.binary.package.checksum,
                &variant_id,
            )
            .await?;

        let jre_dir = paths.jre_dir(&variant_id);
        let tmp_dir = jre_dir.with_extension("tmp");

        // Clear any prior partial extraction
        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
        tokio::fs::create_dir_all(&tmp_dir).await?;

        let tmp_for_task = tmp_dir.clone();
        let extract_result = tokio::task::spawn_blocking(move || -> Result<(), AppError> {
            #[cfg(unix)]
            {
                extract_tar_gz_blocking(bytes, &tmp_for_task)
            }
            #[cfg(windows)]
            {
                extract_zip_blocking(bytes, &tmp_for_task)
            }
            #[cfg(not(any(unix, windows)))]
            {
                Err(AppError::JavaExtractFailed {
                    dest: tmp_for_task,
                    reason: "unsupported OS for JRE extraction".into(),
                })
            }
        })
        .await
        .map_err(|e| AppError::JavaExtractFailed {
            dest: tmp_dir.clone(),
            reason: format!("spawn_blocking join error: {e}"),
        })?;

        if let Err(e) = extract_result {
            let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
            return Err(e);
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
// Archive extraction helpers
// ---------------------------------------------------------------------------

/// Strip the first path component from `p`.
///
/// Returns `None` when `p` has zero or one component (i.e. it IS the top-level
/// prefix dir and has no meaningful sub-path beneath it).
fn strip_top_prefix(p: &Path) -> Option<PathBuf> {
    let mut comps = p.components();
    let _first = comps.next()?; // skip the top-level dir (e.g. `jdk-21.0.10+7-jre/`)
    let rest: PathBuf = comps.collect();
    if rest.as_os_str().is_empty() {
        None
    } else {
        Some(rest)
    }
}

/// Extract a `.tar.gz` byte slice into `dest_dir`, stripping the top-level
/// prefix directory so that `jdk-21.0.10+7-jre/bin/java` extracts to
/// `{dest_dir}/bin/java`.
///
/// MUST be called from inside `tokio::task::spawn_blocking` -- the `tar` and
/// `flate2` crates perform synchronous I/O.
#[cfg(unix)]
fn extract_tar_gz_blocking(bytes: Vec<u8>, dest_dir: &Path) -> Result<(), AppError> {
    use flate2::read::GzDecoder;

    let gz = GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(gz);
    // Preserve permissions from the archive (executable bits etc.)
    archive.set_preserve_permissions(true);

    for entry in archive.entries().map_err(|e| AppError::JavaExtractFailed {
        dest: dest_dir.to_path_buf(),
        reason: format!("tar entries iterator: {e}"),
    })? {
        let mut entry = entry.map_err(|e| AppError::JavaExtractFailed {
            dest: dest_dir.to_path_buf(),
            reason: format!("tar entry: {e}"),
        })?;

        let raw_path = entry
            .path()
            .map_err(|e| AppError::JavaExtractFailed {
                dest: dest_dir.to_path_buf(),
                reason: format!("tar entry path: {e}"),
            })?
            .to_path_buf();

        // Path-traversal guard: every component must be Normal (no `..`, no absolute)
        if !raw_path
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        {
            tracing::warn!(path = ?raw_path, "tar: skipping path-traversal entry");
            continue;
        }

        // Strip the top-level prefix directory (e.g. `jdk-21.0.10+7-jre/`)
        let Some(rel) = strip_top_prefix(&raw_path) else {
            continue; // this IS the top-level dir entry itself
        };

        let dest = dest_dir.join(&rel);

        // Create parent dirs as needed
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(AppError::Io)?;
        }

        // Unpack preserves mode bits from the archive when set_preserve_permissions(true)
        entry
            .unpack(&dest)
            .map_err(|e| AppError::JavaExtractFailed {
                dest: dest.clone(),
                reason: format!("tar unpack: {e}"),
            })?;
    }

    Ok(())
}

/// Extract a `.zip` byte slice into `dest_dir`, stripping the top-level
/// prefix directory so that `jdk-21-jre/bin/java.exe` extracts to
/// `{dest_dir}/bin/java.exe`.
///
/// MUST be called from inside `tokio::task::spawn_blocking`.
#[cfg(windows)]
fn extract_zip_blocking(bytes: Vec<u8>, dest_dir: &Path) -> Result<(), AppError> {
    let reader = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| AppError::JavaExtractFailed {
        dest: dest_dir.to_path_buf(),
        reason: format!("zip open: {e}"),
    })?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| AppError::JavaExtractFailed {
                dest: dest_dir.to_path_buf(),
                reason: format!("zip entry {i}: {e}"),
            })?;

        let name = match file.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };

        // Path-traversal guard
        if !name.components().all(|c| matches!(c, Component::Normal(_))) {
            tracing::warn!(path = ?name, "zip: skipping path-traversal entry");
            continue;
        }

        // Strip top-level prefix
        let Some(rel) = strip_top_prefix(&name) else {
            continue;
        };

        let dest = dest_dir.join(&rel);

        if file.is_dir() {
            std::fs::create_dir_all(&dest).map_err(AppError::Io)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(AppError::Io)?;
            }
            let mut out = std::fs::File::create(&dest).map_err(AppError::Io)?;
            std::io::copy(&mut file, &mut out).map_err(AppError::Io)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Compute SHA-256 hex of `bytes` (lowercase, 64 chars).
///
/// Adoptium uses SHA-256 (not SHA-1). See Pitfall 5 in 05-RESEARCH.md.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write;
        write!(s, "{b:02x}").unwrap();
        s
    })
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

    /// Build a client pointed at the given mock server base URL.
    /// Does NOT touch env vars -- avoids races between parallel tests.
    fn make_client(server: &MockServer) -> AdoptiumClient {
        AdoptiumClient::new_with_base_url(server.base_url()).expect("client build")
    }

    fn make_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    /// Build a synthetic `.tar.gz` archive with layout:
    /// ```text
    /// fixture-jdk-21-jre/
    ///   bin/
    ///     java    (content "fake-java\n", mode 0755)
    /// ```
    #[cfg(unix)]
    fn make_tar_gz_bytes() -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let buf = Vec::new();
        let gz = GzEncoder::new(buf, Compression::default());
        let mut builder = tar::Builder::new(gz);

        // top-level dir
        let mut h = tar::Header::new_gnu();
        h.set_path("fixture-jdk-21-jre/").unwrap();
        h.set_entry_type(tar::EntryType::Directory);
        h.set_mode(0o755);
        h.set_size(0);
        h.set_cksum();
        builder.append(&h, &[] as &[u8]).unwrap();

        // bin/ dir
        let mut h = tar::Header::new_gnu();
        h.set_path("fixture-jdk-21-jre/bin/").unwrap();
        h.set_entry_type(tar::EntryType::Directory);
        h.set_mode(0o755);
        h.set_size(0);
        h.set_cksum();
        builder.append(&h, &[] as &[u8]).unwrap();

        // bin/java file
        let content = b"fake-java\n";
        let mut h = tar::Header::new_gnu();
        h.set_path("fixture-jdk-21-jre/bin/java").unwrap();
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mode(0o755);
        h.set_size(content.len() as u64);
        h.set_cksum();
        builder.append(&h, content.as_ref()).unwrap();

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap()
    }

    /// Build a raw `.tar.gz` that contains a path-traversal entry `../evil/x`.
    ///
    /// The safe `tar::Header::set_path` API rejects `..` components, so we
    /// construct the 512-byte tar header block manually.
    #[cfg(unix)]
    fn make_traversal_tar_gz_bytes() -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let content = b"evil\n";

        // Build a POSIX/ustar 512-byte header block manually.
        let mut block = [0u8; 512];
        // name field (bytes 0-99): the traversal path
        let path = b"../evil/x";
        block[..path.len()].copy_from_slice(path);
        // mode (bytes 100-107)
        block[100..107].copy_from_slice(b"0000644");
        // uid (bytes 108-115)
        block[108..115].copy_from_slice(b"0000000");
        // gid (bytes 116-123)
        block[116..123].copy_from_slice(b"0000000");
        // size (bytes 124-135): octal representation of content.len()
        let size_str = format!("{:011o}\0", content.len());
        block[124..136].copy_from_slice(size_str.as_bytes());
        // mtime (bytes 136-147)
        block[136..147].copy_from_slice(b"00000000000");
        // typeflag (byte 156): '0' = regular file
        block[156] = b'0';
        // magic (bytes 257-262): "ustar "
        block[257..263].copy_from_slice(b"ustar ");
        // version (bytes 263-264)
        block[263..265].copy_from_slice(b" \0");

        // Compute checksum: sum of all bytes with checksum field treated as spaces
        block[148..156].copy_from_slice(b"        "); // treat as spaces for checksum
        let checksum: u32 = block.iter().map(|&b| b as u32).sum();
        let cksum_str = format!("{checksum:06o}\0 ");
        block[148..156].copy_from_slice(cksum_str.as_bytes());

        // Content block (padded to 512)
        let mut content_block = [0u8; 512];
        content_block[..content.len()].copy_from_slice(content);

        // End-of-archive: two 512-byte zero blocks
        let end_blocks = [0u8; 1024];

        // Wrap in gzip
        let buf = Vec::new();
        let mut gz = GzEncoder::new(buf, Compression::default());
        gz.write_all(&block).unwrap();
        gz.write_all(&content_block).unwrap();
        gz.write_all(&end_blocks).unwrap();
        gz.finish().unwrap()
    }

    /// Build a synthetic `.zip` archive with layout:
    /// ```text
    /// fixture-jdk-21-jre/
    ///   bin/
    ///     java.exe    (content "fake-java\n")
    /// ```
    #[cfg(windows)]
    #[allow(dead_code)] // Windows zip-extract test fixture; preserved for future extract-blocking test
    fn make_zip_bytes() -> Vec<u8> {
        use std::io::Write;

        let buf = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        zip.add_directory("fixture-jdk-21-jre/", opts).unwrap();
        zip.add_directory("fixture-jdk-21-jre/bin/", opts).unwrap();

        zip.start_file("fixture-jdk-21-jre/bin/java.exe", opts)
            .unwrap();
        zip.write_all(b"fake-java\n").unwrap();

        zip.finish().unwrap().into_inner()
    }

    // -----------------------------------------------------------------------
    // Task 1: AdoptiumClient + fetch_latest_release + download_verified
    // -----------------------------------------------------------------------

    /// Fixture: one-element JSON array matching the Adoptium API shape.
    fn adoptium_response_json(server_base: &str, archive_sha256: &str) -> String {
        format!(
            r#"[{{
  "binary": {{
    "package": {{
      "link": "{server_base}/OpenJDK21U-jre_x64_linux_hotspot_21.0.10_7.tar.gz",
      "checksum": "{archive_sha256}",
      "size": 1024
    }}
  }},
  "version": {{ "major": 21 }},
  "release_name": "jdk-21.0.10+7"
}}]"#
        )
    }

    #[tokio::test]
    async fn test_fetch_latest_release_parses_array_first_element() {
        let server = MockServer::start();
        let body = adoptium_response_json(&server.base_url(), "aabb");

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/v3/assets/latest/21/hotspot");
            then.status(200)
                .header("content-type", "application/json")
                .body(body.as_str());
        });

        let client = make_client(&server);
        let release = client
            .fetch_latest_release(21, Arch::X86_64, OsName::Linux)
            .await
            .expect("fetch_latest_release");

        assert_eq!(release.version.major, 21);
        assert_eq!(release.release_name, "jdk-21.0.10+7");
        assert!(release.binary.package.link.contains("OpenJDK21U"));
    }

    #[tokio::test]
    async fn test_fetch_latest_release_query_string() {
        let server = MockServer::start();
        let body = adoptium_response_json(&server.base_url(), "aabb");

        let api_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v3/assets/latest/21/hotspot")
                .query_param("architecture", "x64")
                .query_param("heap_size", "normal")
                .query_param("image_type", "jre")
                .query_param("os", "linux")
                .query_param("vendor", "eclipse");
            then.status(200)
                .header("content-type", "application/json")
                .body(body.as_str());
        });

        let client = make_client(&server);
        let _ = client
            .fetch_latest_release(21, Arch::X86_64, OsName::Linux)
            .await
            .expect("fetch");

        api_mock.assert_calls(1);
    }

    #[tokio::test]
    async fn test_download_verified_ok() {
        let server = MockServer::start();
        let content = b"a".repeat(64);
        let expected_sha256 = sha256_hex(&content);

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/archive.tar.gz");
            then.status(200).body(content.as_slice());
        });

        let client = make_client(&server);
        let url = format!("{}/archive.tar.gz", server.base_url());
        let bytes = client
            .download_verified(&url, &expected_sha256, "adoptium-21")
            .await
            .expect("download_verified");

        assert_eq!(bytes, content.as_slice());
    }

    #[tokio::test]
    async fn test_download_verified_sha256_mismatch() {
        let server = MockServer::start();
        let content = b"wrong bytes here";
        let wrong_sha256 = "0".repeat(64); // definitely not matching

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/archive.tar.gz");
            then.status(200).body(content.as_ref());
        });

        let client = make_client(&server);
        let url = format!("{}/archive.tar.gz", server.base_url());
        let result = client
            .download_verified(&url, &wrong_sha256, "adoptium-21")
            .await;

        assert!(result.is_err(), "should fail on sha256 mismatch");
        match result.unwrap_err() {
            AppError::JavaDownloadFailed { reason, .. } => {
                assert!(
                    reason.contains("sha256 mismatch"),
                    "reason should mention sha256: {reason}"
                );
            }
            other => panic!("expected JavaDownloadFailed, got: {other:?}"),
        }
    }

    /// Verify that `ICHR_ADOPTIUM_BASE_URL` is read by `AdoptiumClient::new()`.
    #[tokio::test]
    async fn test_env_override_base_url() {
        let server = MockServer::start();
        let body = adoptium_response_json(&server.base_url(), "aabb");

        let mock = server.mock(|when, then| {
            when.method(GET).path("/v3/assets/latest/17/hotspot");
            then.status(200)
                .header("content-type", "application/json")
                .body(body.as_str());
        });

        // Temporarily set the env var so `new()` picks up the mock server
        let prior = std::env::var(ADOPTIUM_BASE_URL_ENV).ok();
        std::env::set_var(ADOPTIUM_BASE_URL_ENV, server.base_url());

        // Build the client AFTER setting the env var
        let client = AdoptiumClient::new().expect("client from env override");

        let result = client
            .fetch_latest_release(17, Arch::X86_64, OsName::Linux)
            .await;

        // Restore env var before any assert (so we don't leave it set on failure)
        match prior {
            Some(v) => std::env::set_var(ADOPTIUM_BASE_URL_ENV, v),
            None => std::env::remove_var(ADOPTIUM_BASE_URL_ENV),
        }

        result.expect("env override fetch should succeed");
        // The mock was hit -- proves the env var was honoured
        mock.assert_calls(1);
    }

    // -----------------------------------------------------------------------
    // Task 2: install_adoptium -- extraction + prefix strip + idempotency
    // -----------------------------------------------------------------------

    /// Full happy-path install test (Linux tar.gz).
    ///
    /// Verifies that `{jre_dir}/bin/java` exists with content `"fake-java\n"`
    /// (the archive's top-level prefix `fixture-jdk-21-jre/` is stripped).
    #[cfg(unix)]
    #[tokio::test]
    async fn test_install_adoptium_linux_strips_prefix() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let archive_bytes = make_tar_gz_bytes();
        let archive_sha256 = sha256_hex(&archive_bytes);

        let api_body = adoptium_response_json(&server.base_url(), &archive_sha256);

        let _mock_api = server.mock(|when, then| {
            when.method(GET).path("/v3/assets/latest/21/hotspot");
            then.status(200)
                .header("content-type", "application/json")
                .body(api_body.as_str());
        });

        let _mock_archive = server.mock(|when, then| {
            when.method(GET)
                .path("/OpenJDK21U-jre_x64_linux_hotspot_21.0.10_7.tar.gz");
            then.status(200).body(archive_bytes.as_slice());
        });

        let client = make_client(&server);
        let exe = client
            .install_adoptium(&paths, 21, Arch::X86_64, OsName::Linux)
            .await
            .expect("install_adoptium");

        // Executable path must exist
        assert!(exe.exists(), "jre_executable should exist at {exe:?}");

        // Must be at {jre_dir}/bin/java, NOT {jre_dir}/fixture-jdk-21-jre/bin/java
        let jre_dir = paths.jre_dir("adoptium-21");
        let java_path = jre_dir.join("bin/java");
        assert!(
            java_path.exists(),
            "bin/java should exist (prefix stripped)"
        );

        let content = std::fs::read(&java_path).unwrap();
        assert_eq!(content, b"fake-java\n", "content must match fixture");

        // .tmp must be gone
        let tmp_dir = jre_dir.with_extension("tmp");
        assert!(!tmp_dir.exists(), ".tmp dir must be removed after success");

        // Executable bit must be set
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&java_path).unwrap().permissions().mode();
        assert_ne!(mode & 0o111, 0, "bin/java must have executable bit set");
    }

    /// Idempotent re-install: second call is a no-op (no network calls).
    #[tokio::test]
    async fn test_install_adoptium_skips_if_installed() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        // Pre-create the executable so the idempotency guard fires
        let exe = paths.jre_executable("adoptium-21");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"pre-existing").unwrap();

        // This mock must never be hit
        let api_mock = server.mock(|when, then| {
            when.method(GET).path("/v3/assets/latest/21/hotspot");
            then.status(200).body("[]");
        });

        let client = make_client(&server);
        let result = client
            .install_adoptium(&paths, 21, Arch::X86_64, OsName::Linux)
            .await;

        assert!(result.is_ok(), "idempotent install should return Ok");
        api_mock.assert_calls(0);
    }

    /// SHA-256 mismatch: error returned, no `.tmp` dir left behind.
    #[tokio::test]
    async fn test_install_adoptium_sha256_mismatch_no_tmp_left() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let wrong_bytes = b"garbage-content";
        // Use a sha256 that definitely does NOT match wrong_bytes
        let bad_sha256 = "0".repeat(64);

        let api_body = adoptium_response_json(&server.base_url(), &bad_sha256);

        let _mock_api = server.mock(|when, then| {
            when.method(GET).path("/v3/assets/latest/21/hotspot");
            then.status(200)
                .header("content-type", "application/json")
                .body(api_body.as_str());
        });

        let _mock_archive = server.mock(|when, then| {
            when.method(GET)
                .path("/OpenJDK21U-jre_x64_linux_hotspot_21.0.10_7.tar.gz");
            then.status(200).body(wrong_bytes.as_ref());
        });

        let client = make_client(&server);
        let result = client
            .install_adoptium(&paths, 21, Arch::X86_64, OsName::Linux)
            .await;

        assert!(result.is_err(), "sha256 mismatch should return Err");
        match result.unwrap_err() {
            AppError::JavaDownloadFailed { reason, .. } => {
                assert!(reason.contains("sha256"), "reason: {reason}");
            }
            other => panic!("expected JavaDownloadFailed, got: {other:?}"),
        }

        // No .tmp dir should be left (download failed before extraction started)
        let jre_dir = paths.jre_dir("adoptium-21");
        let tmp_dir = jre_dir.with_extension("tmp");
        assert!(
            !tmp_dir.exists(),
            ".tmp must not exist after sha256 mismatch"
        );
    }

    /// Path-traversal in tar archive must be rejected silently (entry skipped).
    ///
    /// A synthetic archive containing `../evil/x` must NOT create that file
    /// outside `dest_dir`. We build the tarball with raw bytes because the
    /// safe `tar::Header::set_path` API rejects `..` components at build time.
    #[cfg(unix)]
    #[test]
    fn test_extract_tar_gz_path_traversal_rejected() {
        let archive_bytes = make_traversal_tar_gz_bytes();

        let td = TempDir::new().unwrap();
        let dest = td.path().join("jre");
        std::fs::create_dir_all(&dest).unwrap();

        // extract_tar_gz_blocking runs synchronously (normally in spawn_blocking)
        let result = extract_tar_gz_blocking(archive_bytes, &dest);

        // Should succeed -- the bad entry is silently skipped
        assert!(
            result.is_ok(),
            "traversal entry should be skipped, not errored: {result:?}"
        );

        // The evil file must NOT exist outside dest
        let evil_path = td.path().join("evil/x");
        assert!(
            !evil_path.exists(),
            "traversal file must not be created: {evil_path:?}"
        );
    }
}
