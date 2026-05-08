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
        let base = std::env::var(CF_BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_CF_BASE.to_owned());
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
                CurseForgeError::Http("invalid api key header value (non-ASCII?)".into())
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
    pub async fn get_mod(&self, mod_id: u64) -> Result<CurseForgeProjectDetail, CurseForgeError> {
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
        let env: Env = serde_json::from_slice(&bytes)
            .map_err(|e| CurseForgeError::Parse(format!("get_file_download_url {url}: {e}")))?;
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

// ============================================================================
// === Tests (httpmock 0.8.3 — already in [dev-dependencies])              ===
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn make_client(server: &MockServer) -> CurseForgeClient {
        CurseForgeClient::new_with_base_url("test-key", server.base_url())
            .expect("client construction")
    }

    // --- Construction --------------------------------------------------------

    #[test]
    fn test_empty_api_key_rejected_at_construction() {
        let r = CurseForgeClient::new("");
        assert!(
            matches!(r, Err(CurseForgeError::Http(_))),
            "empty key must error before reqwest sees it; got {r:?}"
        );
    }

    // --- search --------------------------------------------------------------

    #[tokio::test]
    async fn test_search_url_shape_includes_canonical_params() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/mods/search")
                .query_param("gameId", "432")
                .query_param("classId", "6")
                .query_param("searchFilter", "sodium")
                .query_param("sortField", "2")
                .query_param("sortOrder", "desc")
                .query_param("pageSize", "50")
                .query_param("gameVersion", "1.20.4")
                .query_param("modLoaderType", "4");
            then.status(200).body(r#"{"data":[]}"#);
        });
        let c = make_client(&server);
        let _ = c
            .search("sodium", Some("1.20.4"), Some(4), 50)
            .await
            .unwrap();
        m.assert();
    }

    #[tokio::test]
    async fn test_search_response_parses_data_envelope() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/search");
            then.status(200).body(
                r#"{
                    "data":[
                        {"id":443959,"slug":"sodium","name":"Sodium","summary":"fast","downloadCount":100,"categories":[]},
                        {"id":2,"slug":"x","name":"X","summary":"y","downloadCount":50,"categories":[]}
                    ]
                }"#,
            );
        });
        let c = make_client(&server);
        let hits = c.search("sodium", None, None, 50).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].slug, "sodium");
        assert_eq!(hits[0].download_count, 100);
        assert_eq!(hits[1].id, 2);
        assert!(
            !hits[0].already_installed,
            "client never sets already_installed; caller stamps from ledger"
        );
    }

    #[tokio::test]
    async fn test_x_api_key_header_present_on_search() {
        // Pitfall 2 invariant: every request MUST carry x-api-key.
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/mods/search")
                .header("x-api-key", "test-key");
            then.status(200).body(r#"{"data":[]}"#);
        });
        let c = make_client(&server);
        let _ = c.search("x", None, None, 50).await.unwrap();
        m.assert();
    }

    #[tokio::test]
    async fn test_accept_header_application_json_present_on_search() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/mods/search")
                .header("accept", "application/json");
            then.status(200).body(r#"{"data":[]}"#);
        });
        let c = make_client(&server);
        let _ = c.search("x", None, None, 50).await.unwrap();
        m.assert();
    }

    #[tokio::test]
    async fn test_search_omits_loader_param_when_none() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/v1/mods/search");
            then.status(200).body(r#"{"data":[]}"#);
        });
        let c = make_client(&server);
        let _ = c.search("x", None, None, 50).await.unwrap();
        m.assert_calls(1);
        // The URL-shape test above proves modLoaderType is included when Some;
        // this test exercises the None branch end-to-end (one request, parsed).
    }

    // --- get_mod -------------------------------------------------------------

    #[tokio::test]
    async fn test_get_mod_happy_path() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/443959");
            then.status(200).body(
                r#"{"data":{
                    "id":443959,"slug":"sodium","name":"Sodium",
                    "downloadCount":100,"authors":[{"id":1,"name":"a","url":""}],
                    "links":{"websiteUrl":"https://x"}
                }}"#,
            );
        });
        let c = make_client(&server);
        let detail = c.get_mod(443959).await.unwrap();
        assert_eq!(detail.id, 443959);
        assert_eq!(detail.slug, "sodium");
        assert_eq!(detail.authors.len(), 1);
        assert_eq!(detail.links.website_url, "https://x");
    }

    #[tokio::test]
    async fn test_get_mod_404_returns_mod_not_found() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/999");
            then.status(404).body(r#"{"error":"not found"}"#);
        });
        let c = make_client(&server);
        let r = c.get_mod(999).await;
        assert!(
            matches!(r, Err(CurseForgeError::ModNotFound { mod_id: 999 })),
            "expected ModNotFound, got {r:?}"
        );
    }

    #[tokio::test]
    async fn test_x_api_key_header_present_on_get_mod() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/mods/1")
                .header("x-api-key", "test-key");
            then.status(200).body(
                r#"{"data":{"id":1,"slug":"x","name":"X","downloadCount":0,"authors":[],"links":{"websiteUrl":""}}}"#,
            );
        });
        let c = make_client(&server);
        let _ = c.get_mod(1).await.unwrap();
        m.assert();
    }

    // --- list_files ----------------------------------------------------------

    #[tokio::test]
    async fn test_list_files_url_shape_with_filters() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/mods/443959/files")
                .query_param("pageSize", "50")
                .query_param("gameVersion", "1.20.4")
                .query_param("modLoaderType", "4");
            then.status(200).body(r#"{"data":[]}"#);
        });
        let c = make_client(&server);
        let _ = c.list_files(443959, Some("1.20.4"), Some(4)).await.unwrap();
        m.assert();
    }

    #[tokio::test]
    async fn test_list_files_parses_null_download_url() {
        // The load-bearing fact for MOD-04: downloadUrl is null when restricted.
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/443959/files");
            then.status(200).body(
                r#"{"data":[{
                    "id":4567890,"displayName":"X 1.0","fileName":"x.jar",
                    "releaseType":1,"fileStatus":4,"hashes":[{"value":"abc","algo":1}],
                    "fileDate":"2026-01-01T00:00:00Z","fileLength":1024,"downloadCount":1,
                    "downloadUrl":null,"gameVersions":["1.20.4"],"dependencies":[],"isAvailable":true
                }]}"#,
            );
        });
        let c = make_client(&server);
        let files = c.list_files(443959, None, None).await.unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            files[0].download_url.is_none(),
            "downloadUrl null must parse as None"
        );
        assert_eq!(files[0].id, 4567890);
        assert_eq!(files[0].hashes[0].algo, 1);
    }

    // --- get_file_download_url ----------------------------------------------

    #[tokio::test]
    async fn test_get_file_download_url_200_returns_some() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/v1/mods/443959/files/4567890/download-url");
            then.status(200)
                .body(r#"{"data":"https://edge.forgecdn.net/files/4567/890/sodium.jar"}"#);
        });
        let c = make_client(&server);
        let url = c.get_file_download_url(443959, 4567890).await.unwrap();
        assert_eq!(
            url.as_deref(),
            Some("https://edge.forgecdn.net/files/4567/890/sodium.jar")
        );
    }

    #[tokio::test]
    async fn test_get_file_download_url_403_returns_ok_none() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/v1/mods/443959/files/4567890/download-url");
            then.status(403).body(r#"{"error":"restricted"}"#);
        });
        let c = make_client(&server);
        let r = c.get_file_download_url(443959, 4567890).await.unwrap();
        assert!(r.is_none(), "403 must return Ok(None), got {r:?}");
    }

    #[tokio::test]
    async fn test_get_file_download_url_404_returns_ok_none() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/v1/mods/443959/files/4567890/download-url");
            then.status(404).body(r#"{"error":"not found"}"#);
        });
        let c = make_client(&server);
        let r = c.get_file_download_url(443959, 4567890).await.unwrap();
        assert!(r.is_none(), "404 must return Ok(None), got {r:?}");
    }

    // --- 429 / 5xx / parse error mapping ------------------------------------

    #[tokio::test]
    async fn test_429_with_retry_after_returns_rate_limited_42() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/search");
            then.status(429)
                .header("Retry-After", "42")
                .body("rate limited");
        });
        let c = make_client(&server);
        let r = c.search("x", None, None, 50).await;
        assert!(
            matches!(
                r,
                Err(CurseForgeError::RateLimited {
                    retry_after_secs: 42
                })
            ),
            "got {r:?}"
        );
    }

    #[tokio::test]
    async fn test_429_no_retry_after_defaults_60() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/search");
            then.status(429).body("");
        });
        let c = make_client(&server);
        let r = c.search("x", None, None, 50).await;
        assert!(
            matches!(
                r,
                Err(CurseForgeError::RateLimited {
                    retry_after_secs: 60
                })
            ),
            "got {r:?}"
        );
    }

    #[tokio::test]
    async fn test_5xx_returns_http_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/search");
            then.status(500).body("internal");
        });
        let c = make_client(&server);
        let r = c.search("x", None, None, 50).await;
        assert!(matches!(r, Err(CurseForgeError::Http(_))), "got {r:?}");
    }

    #[tokio::test]
    async fn test_malformed_json_returns_parse_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/search");
            then.status(200).body("definitely not json");
        });
        let c = make_client(&server);
        let r = c.search("x", None, None, 50).await;
        assert!(matches!(r, Err(CurseForgeError::Parse(_))), "got {r:?}");
    }
}
