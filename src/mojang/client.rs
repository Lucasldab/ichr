//! Mojang HTTP client.
//!
//! One `reqwest::Client` per `MojangClient`. Always uses a ichr User-Agent.
//! TLS via rustls (Cargo.toml feature = "rustls"; not "rustls-tls").

use std::path::Path;
use std::time::Duration;

use sha1::{Digest, Sha1};

use crate::error::AppError;
use crate::mojang::cache::{atomic_write, cache_is_fresh, verify_sha1, MANIFEST_CACHE_TTL};
use crate::mojang::types::{AssetIndexFile, VersionJson, VersionManifest};

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

pub const ASSET_CDN_BASE: &str = "https://resources.download.minecraft.net";

pub const USER_AGENT: &str = "ichr/0.1 (+https://github.com/placeholder/ichr)";

/// Thin HTTP facade over Mojang's CDN. All methods are async, cancel-safe,
/// and do not hold blocking I/O across await points.
#[derive(Debug, Clone)]
pub struct MojangClient {
    http: reqwest::Client,
}

impl MojangClient {
    /// Construct with the launcher's User-Agent and a 30s request timeout.
    pub fn new() -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AppError::Http(format!("reqwest build: {e}")))?;
        Ok(Self { http })
    }

    /// The static User-Agent string; returned for tests and diagnostics.
    pub fn user_agent(&self) -> &'static str {
        USER_AGENT
    }

    /// Fetch the version manifest, using `cache_path` as a 1h TTL cache.
    /// If cache is fresh, parses and returns without hitting the network.
    pub async fn fetch_manifest(&self, cache_path: &Path) -> Result<VersionManifest, AppError> {
        if cache_is_fresh(cache_path, MANIFEST_CACHE_TTL).await? {
            let bytes = tokio::fs::read(cache_path).await?;
            return Ok(serde_json::from_slice(&bytes)?);
        }
        let resp = self
            .http
            .get(MANIFEST_URL)
            .send()
            .await
            .map_err(|e| AppError::Http(format!("GET manifest: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Http(format!("manifest status: {e}")))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Http(format!("manifest body: {e}")))?;
        atomic_write(cache_path, &bytes).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Fetch a per-version JSON (e.g., 1.21.4.json) from `url`, verify the
    /// declared `expected_sha1` from the manifest entry, and cache to
    /// `cache_path`. If `cache_path` exists AND its SHA1 matches, skip network.
    pub async fn fetch_version_json(
        &self,
        url: &str,
        expected_sha1: &str,
        cache_path: &Path,
    ) -> Result<VersionJson, AppError> {
        if verify_sha1(cache_path, expected_sha1).await? {
            let bytes = tokio::fs::read(cache_path).await?;
            return Ok(serde_json::from_slice(&bytes)?);
        }
        let bytes = self.get_bytes_verified(url, expected_sha1).await?;
        atomic_write(cache_path, &bytes).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Fetch an asset index JSON and cache to `cache_path`.
    /// Caller passes the expected SHA1 from `VersionJson.asset_index.sha1`.
    pub async fn fetch_asset_index(
        &self,
        url: &str,
        expected_sha1: &str,
        cache_path: &Path,
    ) -> Result<AssetIndexFile, AppError> {
        if verify_sha1(cache_path, expected_sha1).await? {
            let bytes = tokio::fs::read(cache_path).await?;
            return Ok(serde_json::from_slice(&bytes)?);
        }
        let bytes = self.get_bytes_verified(url, expected_sha1).await?;
        atomic_write(cache_path, &bytes).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Download `url` to `dest`, streaming bytes. Verify SHA1 after download.
    /// On mismatch: retry once, then return `AppError::Sha1Mismatch`.
    /// If `dest` already exists AND its SHA1 matches `expected_sha1`, skip.
    pub async fn download_verified(
        &self,
        url: &str,
        dest: &Path,
        expected_sha1: &str,
    ) -> Result<(), AppError> {
        // Skip if already correct.
        if verify_sha1(dest, expected_sha1).await? {
            return Ok(());
        }

        for attempt in 0..2u8 {
            let bytes = match self.download_stream(url).await {
                Ok(b) => b,
                Err(e) if attempt == 0 => {
                    tracing::warn!(%url, attempt, error = %e, "download failed, retrying");
                    continue;
                }
                Err(e) => return Err(e),
            };
            let got = format!("{:x}", Sha1::digest(&bytes));
            if got.eq_ignore_ascii_case(expected_sha1) {
                atomic_write(dest, &bytes).await?;
                return Ok(());
            }
            if attempt == 0 {
                tracing::warn!(%url, expected = %expected_sha1, %got, "SHA1 mismatch; retrying");
                continue;
            }
            return Err(AppError::Sha1Mismatch {
                target: url.to_string(),
                expected: expected_sha1.to_string(),
                got,
            });
        }
        unreachable!("loop exits via return after 2 attempts")
    }

    /// Download `url` to `dest` WITHOUT SHA-1 verification.
    ///
    /// Use ONLY when the upstream API does not provide a sha1 (Quilt loader
    /// libraries -- see Phase 8.4 GAP-LIBRARY-SHAPE-08). All Mojang-protocol
    /// callers MUST use `download_verified` instead. The caller is responsible
    /// for logging the trade-off (e.g. tracing::info!) at the call site.
    pub async fn download_unverified(&self, url: &str, dest: &Path) -> Result<(), AppError> {
        let bytes = self.download_stream(url).await?;
        atomic_write(dest, &bytes).await?;
        Ok(())
    }

    /// Private: fetch `url`, return SHA1-verified bytes or AppError. Used by
    /// fetch_version_json / fetch_asset_index.
    async fn get_bytes_verified(
        &self,
        url: &str,
        expected_sha1: &str,
    ) -> Result<Vec<u8>, AppError> {
        let bytes = self.download_stream(url).await?;
        let got = format!("{:x}", Sha1::digest(&bytes));
        if !got.eq_ignore_ascii_case(expected_sha1) {
            return Err(AppError::Sha1Mismatch {
                target: url.to_string(),
                expected: expected_sha1.to_string(),
                got,
            });
        }
        Ok(bytes)
    }

    /// Private: stream `url` into memory. Returns error wrapped as
    /// `AppError::Http`. No content-length required.
    async fn download_stream(&self, url: &str) -> Result<Vec<u8>, AppError> {
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
            .map_err(|e| AppError::Http(format!("body {url}: {e}")))?;
        Ok(bytes.to_vec())
    }
}
