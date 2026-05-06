//! Hand-rolled CurseForge for Studios REST API v1 client over the four
//! endpoints needed by Phase 9. Mirrors `mods::modrinth::client::ModrinthClient`
//! shape with these substitutions:
//!
//!   - `x-api-key` header on default_headers (NEVER per-request, NEVER in URL)
//!   - integer `modLoaderType` enum filter (NOT a string list)
//!   - nullable `downloadUrl` — `get_file_download_url` returns Ok(None) on 403/404
//!   - SHA-1 (CurseForge default) instead of SHA-512
//!
//! The api key value is **never** logged. Tracing instruments use
//! `skip_all` and an explicit field allowlist that excludes the key.
//!
//! Per 09-RESEARCH.md §Endpoint Reference (lines 96-148) and
//! §"x-api-key header and Accept header" (lines 149-164).

use std::time::Duration;

use crate::mods::curseforge::error::CurseForgeError;
use crate::mods::curseforge::types::{
    CurseForgeFileEntry, CurseForgeProjectDetail, CurseForgeSearchHit,
};

pub const DEFAULT_CF_BASE: &str = "https://api.curseforge.com";
pub const CF_BASE_URL_ENV: &str = "MINELTUI_CURSEFORGE_BASE_URL";
pub const MINECRAFT_GAME_ID: u32 = 432;
pub const MOD_CLASS_ID: u32 = 6;
pub const SEARCH_DEFAULT_PAGE_SIZE: u32 = 50;

/// Hand-rolled CurseForge for Studios v1 REST client.
///
/// Holds a single `reqwest::Client` with `x-api-key` and `Accept: application/json`
/// configured on `default_headers` at build time. The key value is never
/// reproduced in URLs, query strings, error messages, or tracing fields.
#[derive(Debug, Clone)]
pub struct CurseForgeClient {
    http: reqwest::Client,
    base_url: String,
}

impl CurseForgeClient {
    /// Construct from an API key (read by the caller via the api_key resolver).
    /// Reads `MINELTUI_CURSEFORGE_BASE_URL` env var for httpmock injection;
    /// falls back to `DEFAULT_CF_BASE`.
    pub fn new(api_key: &str) -> Result<Self, CurseForgeError> {
        let base = std::env::var(CF_BASE_URL_ENV)
            .unwrap_or_else(|_| DEFAULT_CF_BASE.to_owned());
        Self::new_with_base_url(api_key, base)
    }

    /// Construct with an explicit base URL (test ctor for httpmock).
    pub fn new_with_base_url(
        api_key: &str,
        base_url: impl Into<String>,
    ) -> Result<Self, CurseForgeError> {
        // Defense: empty key is a real precedence-bug failure mode (Pitfall 1
        // surface). Reject before reqwest's header parser sees it.
        if api_key.is_empty() {
            return Err(CurseForgeError::Http(
                "invalid api key header value (empty)".into(),
            ));
        }

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-api-key",
            api_key.parse().map_err(|_| {
                CurseForgeError::Http(
                    "invalid api key header value (non-ASCII?)".into(),
                )
            })?,
        );
        headers.insert(
            reqwest::header::ACCEPT,
            "application/json".parse().expect("constant header value"),
        );

        let http = reqwest::Client::builder()
            .user_agent(crate::mojang::client::USER_AGENT)
            .default_headers(headers)
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| CurseForgeError::Http(format!("reqwest build: {e}")))?;

        Ok(Self {
            http,
            base_url: base_url.into(),
        })
    }

    /// Borrow the underlying reqwest client for streaming mod-file downloads
    /// in 09-04/09-05 installer (preserves connection pool). The header
    /// `x-api-key` is set on default_headers so any borrowed request inherits
    /// it automatically.
    #[allow(dead_code)]
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    // --- Endpoints -----------------------------------------------------------

    /// `GET /v1/mods/search` — project search.
    ///
    /// 09-RESEARCH.md §Endpoint #1.
    #[tracing::instrument(skip_all, fields(query, mc, loader_type, page_size))]
    pub async fn search(
        &self,
        query: &str,
        mc: Option<&str>,
        loader_type: Option<i32>,
        page_size: u32,
    ) -> Result<Vec<CurseForgeSearchHit>, CurseForgeError> {
        let mut url = format!(
            "{}/v1/mods/search?gameId={}&classId={}&searchFilter={}&sortField=2&sortOrder=desc&index=0&pageSize={}",
            self.base_url,
            MINECRAFT_GAME_ID,
            MOD_CLASS_ID,
            urlencoding::encode(query),
            page_size,
        );
        if let Some(mc) = mc {
            url.push_str(&format!("&gameVersion={}", urlencoding::encode(mc)));
        }
        if let Some(lt) = loader_type {
            url.push_str(&format!("&modLoaderType={lt}"));
        }
        let bytes = self.send_get_bytes(&url).await?;
        #[derive(serde::Deserialize)]
        struct Env {
            data: Vec<CurseForgeSearchHit>,
        }
        let env: Env = serde_json::from_slice(&bytes)
            .map_err(|e| CurseForgeError::Parse(format!("search {url}: {e}")))?;
        Ok(env.data)
    }

    /// `GET /v1/mods/{modId}` — single mod detail.
    ///
    /// 09-RESEARCH.md §Endpoint #2. 404 → `CurseForgeError::ModNotFound`.
    #[tracing::instrument(skip_all, fields(mod_id))]
    pub async fn get_mod(
        &self,
        mod_id: u64,
    ) -> Result<CurseForgeProjectDetail, CurseForgeError> {
        let url = format!("{}/v1/mods/{}", self.base_url, mod_id);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| CurseForgeError::Http(format!("GET {url}: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CurseForgeError::ModNotFound { mod_id });
        }
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(rate_limit_from(&resp));
        }
        let resp = resp
            .error_for_status()
            .map_err(|e| CurseForgeError::Http(format!("status {url}: {e}")))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| CurseForgeError::Http(format!("body {url}: {e}")))?;
        #[derive(serde::Deserialize)]
        struct Env {
            data: CurseForgeProjectDetail,
        }
        let env: Env = serde_json::from_slice(&bytes)
            .map_err(|e| CurseForgeError::Parse(format!("get_mod {url}: {e}")))?;
        Ok(env.data)
    }

    /// `GET /v1/mods/{modId}/files` — file list (versions).
    ///
    /// 09-RESEARCH.md §Endpoint #3.
    #[tracing::instrument(skip_all, fields(mod_id, mc, loader_type))]
    pub async fn list_files(
        &self,
        mod_id: u64,
        mc: Option<&str>,
        loader_type: Option<i32>,
    ) -> Result<Vec<CurseForgeFileEntry>, CurseForgeError> {
        let mut url = format!(
            "{}/v1/mods/{}/files?pageSize=50&index=0",
            self.base_url, mod_id,
        );
        if let Some(mc) = mc {
            url.push_str(&format!("&gameVersion={}", urlencoding::encode(mc)));
        }
        if let Some(lt) = loader_type {
            url.push_str(&format!("&modLoaderType={lt}"));
        }
        let bytes = self.send_get_bytes(&url).await?;
        #[derive(serde::Deserialize)]
        struct Env {
            data: Vec<CurseForgeFileEntry>,
        }
        let env: Env = serde_json::from_slice(&bytes)
            .map_err(|e| CurseForgeError::Parse(format!("list_files {url}: {e}")))?;
        Ok(env.data)
    }

    /// `GET /v1/mods/{modId}/files/{fileId}/download-url` — fetch download URL.
    ///
    /// 09-RESEARCH.md §Endpoint #4. 403/404 → `Ok(None)` (file restricted; the
    /// installer composes the FileNotDownloadable error with the web URL).
    /// 200 → `Ok(Some(url))`.
    #[tracing::instrument(skip_all, fields(mod_id, file_id))]
    pub async fn get_file_download_url(
        &self,
        mod_id: u64,
        file_id: u64,
    ) -> Result<Option<String>, CurseForgeError> {
        let url = format!(
            "{}/v1/mods/{}/files/{}/download-url",
            self.base_url, mod_id, file_id,
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| CurseForgeError::Http(format!("GET {url}: {e}")))?;

        // 403/404 means restricted — let the installer compose the
        // FileNotDownloadable error with the web URL. We DO NOT raise here.
        // Per 09-RESEARCH.md §"Endpoint #4" line 141.
        if resp.status() == reqwest::StatusCode::NOT_FOUND
            || resp.status() == reqwest::StatusCode::FORBIDDEN
        {
            return Ok(None);
        }
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(rate_limit_from(&resp));
        }
        let resp = resp
            .error_for_status()
            .map_err(|e| CurseForgeError::Http(format!("status {url}: {e}")))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| CurseForgeError::Http(format!("body {url}: {e}")))?;
        #[derive(serde::Deserialize)]
        struct Env {
            data: String,
        }
        let env: Env = serde_json::from_slice(&bytes).map_err(|e| {
            CurseForgeError::Parse(format!("get_file_download_url {url}: {e}"))
        })?;
        Ok(Some(env.data))
    }

    // --- Internal helpers ----------------------------------------------------

    /// Issue a GET, return body bytes. Maps 429 to `RateLimited`, other non-2xx
    /// to `Http`, network/body errors to `Http`. The `x-api-key` and `Accept`
    /// headers are inherited from the client's default_headers; this helper
    /// adds nothing per-request.
    async fn send_get_bytes(&self, url: &str) -> Result<Vec<u8>, CurseForgeError> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| CurseForgeError::Http(format!("GET {url}: {e}")))?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(rate_limit_from(&resp));
        }
        let resp = resp
            .error_for_status()
            .map_err(|e| CurseForgeError::Http(format!("status {url}: {e}")))?;
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| CurseForgeError::Http(format!("body {url}: {e}")))
    }
}

/// Parse 429 response into `CurseForgeError::RateLimited`.
/// Falls back to 60s if Retry-After is absent or unparseable. Per Pitfall 7
/// (09-RESEARCH.md line 972) v1 surfaces only — no auto-retry.
fn rate_limit_from(resp: &reqwest::Response) -> CurseForgeError {
    let secs = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(60);
    CurseForgeError::RateLimited {
        retry_after_secs: secs,
    }
}
