//! Modrinth REST API v2 client — hand-rolled per STACK.md "hand-roll" decision.
//!
//! Six endpoints over `https://api.modrinth.com`. Mirrors `FabricMetaClient`
//! (src/loader/fabric.rs) for constructor + tracing + error-mapping + httpmock-test
//! conventions.
//!
//! ASSUMPTION A1 from 08-RESEARCH.md — `MOD_DOWNLOAD_CONCURRENCY = 6` is safe
//! below the 300 req/min cap. Verify in human checkpoint.
//!
//! ASSUMPTION A2 from 08-RESEARCH.md — `mineltui/0.1 (+https://github.com/.../mineltui)`
//! UA satisfies Modrinth's "uniquely identifying" requirement. Verify in human checkpoint.
//!
//! PITFALL 1: Modrinth returns 403 against the default reqwest UA — every request
//! MUST carry `crate::mojang::client::USER_AGENT`. Set at client-build time, NOT
//! per-request.

use std::time::Duration;

use crate::mods::error::ModrinthError;
use crate::mods::filter::{is_safe_modrinth_slug, search_facets};
use crate::mods::types::{
    ModrinthProjectDetail, ModrinthSearchHit, ModrinthVersion, ProjectIdTitle,
};

pub const DEFAULT_MODRINTH_BASE: &str = "https://api.modrinth.com";
pub const MODRINTH_BASE_URL_ENV: &str = "MINELTUI_MODRINTH_BASE_URL";
pub const SEARCH_DEFAULT_LIMIT: u32 = 20;

/// Maximum allowed file size for a single mod download — defense in depth
/// against a tampered API response advertising a multi-GB file (08-RESEARCH.md
/// §Security Domain row "Denial-of-service via huge size field"). Largest
/// known legitimate mod is ~10MB; cap at 256MB (25× headroom). The cap is
/// enforced by the 08-06 installer, NOT this client — declared here as a
/// re-exportable constant so installer + tests share one definition.
pub const MAX_MOD_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ModrinthClient {
    http: reqwest::Client,
    base_url: String,
}

impl ModrinthClient {
    pub fn new() -> Result<Self, ModrinthError> {
        let base_url = std::env::var(MODRINTH_BASE_URL_ENV)
            .unwrap_or_else(|_| DEFAULT_MODRINTH_BASE.to_owned());
        Self::new_with_base_url(base_url)
    }

    pub fn new_with_base_url(base_url: impl Into<String>) -> Result<Self, ModrinthError> {
        let http = reqwest::Client::builder()
            .user_agent(crate::mojang::client::USER_AGENT)
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ModrinthError::Http(format!("reqwest build: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.into(),
        })
    }

    /// Borrow the underlying reqwest client for streaming mod-file downloads
    /// in 08-06 installer (preserves connection pool). Allowed-dead-code
    /// because the consumer (08-06 installer) lands in a later plan; this
    /// accessor is intentionally exposed as part of the public-within-crate
    /// API contract documented in 08-03 plan must_haves.
    #[allow(dead_code)]
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    // --- Endpoints -----------------------------------------------------------

    /// `GET /v2/search` — project search with facets.
    ///
    /// 08-RESEARCH.md §Endpoint #1.
    ///
    /// Wraps `search_with_project_type` with `project_type = "mod"` for
    /// backward compatibility. The existing gate (`if mc.is_some() ||
    /// !loaders.is_empty()`) is preserved here so callers that intentionally
    /// want unfaceted "any-project_type" searches continue to work. Pack search
    /// MUST use `search_with_project_type` directly to guarantee the
    /// `project_type` facet is always emitted (HIGH-1 invariant).
    #[tracing::instrument(skip_all, fields(query, mc, loaders_count = loaders.len(), limit))]
    pub async fn search(
        &self,
        query: &str,
        mc: Option<&str>,
        loaders: &[&str],
        limit: u32,
    ) -> Result<Vec<ModrinthSearchHit>, ModrinthError> {
        let mut url = format!(
            "{}/v2/search?query={}&limit={}&index=relevance",
            self.base_url,
            urlencoding::encode(query),
            limit
        );
        if mc.is_some() || !loaders.is_empty() {
            let mc_versions: Vec<String> = mc.map(|v| vec![v.to_string()]).unwrap_or_default();
            let facets = search_facets(loaders, &mc_versions, "mod");
            url.push_str("&facets=");
            url.push_str(&urlencoding::encode(&facets));
        }
        let bytes = self.send_get_bytes(&url).await?;
        // Modrinth returns { hits: [...], offset, limit, total_hits }.
        // Parse a minimal envelope and extract the hits we need.
        #[derive(serde::Deserialize)]
        struct SearchEnv {
            hits: Vec<SearchHitWire>,
        }
        #[derive(serde::Deserialize)]
        struct SearchHitWire {
            project_id: String,
            slug: String,
            title: String,
            description: String,
            #[serde(default)]
            downloads: u64,
        }
        let env: SearchEnv = serde_json::from_slice(&bytes)
            .map_err(|e| ModrinthError::Parse(format!("search {url}: {e}")))?;
        Ok(env
            .hits
            .into_iter()
            .map(|h| ModrinthSearchHit {
                project_id: h.project_id,
                slug: h.slug,
                title: h.title,
                description: h.description,
                downloads: h.downloads,
                already_installed: false, // caller stamps from ledger
            })
            .collect())
    }

    /// `GET /v2/search` with an explicit `project_type` — used by `PackService`
    /// to search for `"resourcepack"` or `"shader"` projects.
    ///
    /// HIGH-1 invariant: the `project_type` facet is emitted UNCONDITIONALLY,
    /// even when `mc == None` and `loaders == []`. This prevents Modrinth from
    /// returning mods, modpacks, and packs mixed together when neither mc nor
    /// loader filters narrow the query. `search()` preserves the old gate for
    /// backward compat; this method does NOT inherit it.
    #[tracing::instrument(
        skip_all,
        fields(query, mc, loaders_count = loaders.len(), project_type, limit)
    )]
    pub async fn search_with_project_type(
        &self,
        query: &str,
        mc: Option<&str>,
        loaders: &[&str],
        project_type: &str,
        limit: u32,
    ) -> Result<Vec<ModrinthSearchHit>, ModrinthError> {
        let url = {
            let mc_versions: Vec<String> = mc.map(|v| vec![v.to_string()]).unwrap_or_default();
            // Unconditionally build facets including project_type (HIGH-1).
            let facets = search_facets(loaders, &mc_versions, project_type);
            format!(
                "{}/v2/search?query={}&limit={}&index=relevance&facets={}",
                self.base_url,
                urlencoding::encode(query),
                limit,
                urlencoding::encode(&facets),
            )
        };
        let bytes = self.send_get_bytes(&url).await?;
        #[derive(serde::Deserialize)]
        struct SearchEnv {
            hits: Vec<SearchHitWire>,
        }
        #[derive(serde::Deserialize)]
        struct SearchHitWire {
            project_id: String,
            slug: String,
            title: String,
            description: String,
            #[serde(default)]
            downloads: u64,
        }
        let env: SearchEnv = serde_json::from_slice(&bytes)
            .map_err(|e| ModrinthError::Parse(format!("search_with_project_type {url}: {e}")))?;
        Ok(env
            .hits
            .into_iter()
            .map(|h| ModrinthSearchHit {
                project_id: h.project_id,
                slug: h.slug,
                title: h.title,
                description: h.description,
                downloads: h.downloads,
                already_installed: false,
            })
            .collect())
    }

    /// `GET /v2/project/{id|slug}` — single project detail.
    ///
    /// 08-RESEARCH.md §Endpoint #2.
    #[tracing::instrument(skip_all, fields(id_or_slug))]
    pub async fn get_project(
        &self,
        id_or_slug: &str,
    ) -> Result<ModrinthProjectDetail, ModrinthError> {
        // V5 input validation BEFORE building URL.
        if !is_safe_modrinth_slug(id_or_slug) {
            return Err(ModrinthError::Http(format!(
                "invalid project id/slug: {id_or_slug}"
            )));
        }
        let url = format!("{}/v2/project/{}", self.base_url, id_or_slug);
        let bytes = self.send_get_bytes(&url).await?;
        // Wire shape — see 08-RESEARCH.md §Endpoint #2 line 98.
        #[derive(serde::Deserialize)]
        struct ProjectWire {
            id: String,
            title: String,
            #[serde(default)]
            description: String,
            #[serde(default)]
            body: String,
            #[serde(default)]
            downloads: u64,
            #[serde(default)]
            license: Option<LicenseWire>,
            #[serde(default)]
            categories: Vec<String>,
            #[serde(default)]
            additional_categories: Vec<String>,
            // Latest version label is NOT directly on the project response;
            // we leave it blank here and let the caller fill from list_versions
            // when it needs the value (UI-SPEC detail pane currently shows
            // categories + downloads + license; the "Latest:" line is fed by
            // the version-picker fetch).
        }
        #[derive(serde::Deserialize)]
        struct LicenseWire {
            #[serde(default)]
            id: String,
        }
        let p: ProjectWire = serde_json::from_slice(&bytes)
            .map_err(|e| ModrinthError::Parse(format!("project {url}: {e}")))?;
        // Author: Modrinth project response does NOT include the author name
        // directly — only a `team` ID. UI-SPEC §Mod Browser Detail Pane shows
        // `by {author}`; for v1 we put an empty string — the live smoke in 08-09
        // verifies this is acceptable; if not, follow-up fetches /v2/team/{id}/members.
        // Documented as a known minor gap.
        let cats = if p.additional_categories.is_empty() {
            p.categories.clone()
        } else {
            let mut all = p.categories.clone();
            all.extend(p.additional_categories.iter().cloned());
            all
        };
        Ok(ModrinthProjectDetail {
            project_id: p.id,
            title: p.title,
            author: String::new(), // see comment above
            body: if !p.body.is_empty() {
                p.body
            } else {
                p.description
            },
            downloads: p.downloads,
            latest_version_label: String::new(),
            latest_version_channel: String::new(),
            license_id: p.license.map(|l| l.id).unwrap_or_default(),
            categories: cats,
        })
    }

    /// `GET /v2/projects?ids=[...]` — batch-fetch (id, title) pairs.
    ///
    /// Used by the dep-resolver title-hydration pass (closes GAP-8-D —
    /// without this, `ResolvedDep.project_title` is empty and the dep-confirm
    /// modal + Installed Mods List surface opaque project_ids).
    ///
    /// Returns `Ok(vec![])` for an empty input slice (no HTTP call). Modrinth
    /// allows up to 200 ids per call; callers in this codebase expect at most
    /// ~10 deps per resolve so we do not chunk. Issues a single round-trip
    /// regardless of the BFS diamond shape (dedupe before call).
    ///
    /// The response projection is intentionally minimal — only `id` + `title`
    /// — because the BFS resolver only needs the title for display. Other
    /// project fields (description, license, categories) flow through
    /// `get_project` on the detail-pane code path.
    #[tracing::instrument(skip_all, fields(n = ids.len()))]
    pub async fn get_projects_batch(
        &self,
        ids: &[&str],
    ) -> Result<Vec<ProjectIdTitle>, ModrinthError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        // Modrinth expects ids as a JSON-encoded array literal in the query
        // string: `ids=["P7dR8mSH","AANobbMI"]`. urlencoding handles the
        // percent-encoding of brackets and quotes.
        let json_array = serde_json::to_string(ids)
            .map_err(|e| ModrinthError::Parse(format!("get_projects_batch ids encode: {e}")))?;
        let url = format!(
            "{}/v2/projects?ids={}",
            self.base_url,
            urlencoding::encode(&json_array)
        );
        let bytes = self.send_get_bytes(&url).await?;
        serde_json::from_slice::<Vec<ProjectIdTitle>>(&bytes)
            .map_err(|e| ModrinthError::Parse(format!("get_projects_batch {url}: {e}")))
    }

    /// `GET /v2/project/{id}/version` — filtered version list.
    ///
    /// 08-RESEARCH.md §Endpoint #3. Always sets `include_changelog=false`.
    #[tracing::instrument(skip_all, fields(project_id, mc, loaders_count = loaders.len()))]
    pub async fn list_versions(
        &self,
        project_id: &str,
        mc: Option<&str>,
        loaders: &[&str],
    ) -> Result<Vec<ModrinthVersion>, ModrinthError> {
        if !is_safe_modrinth_slug(project_id) {
            return Err(ModrinthError::Http(format!(
                "invalid project id: {project_id}"
            )));
        }
        let mut url = format!(
            "{}/v2/project/{}/version?include_changelog=false",
            self.base_url, project_id
        );
        if !loaders.is_empty() {
            let loaders_json = serde_json::to_string(loaders)
                .map_err(|e| ModrinthError::Parse(format!("loaders json: {e}")))?;
            url.push_str(&format!("&loaders={}", urlencoding::encode(&loaders_json)));
        }
        if let Some(m) = mc {
            let mc_json = serde_json::to_string(&[m])
                .map_err(|e| ModrinthError::Parse(format!("game_versions json: {e}")))?;
            url.push_str(&format!("&game_versions={}", urlencoding::encode(&mc_json)));
        }
        let bytes = self.send_get_bytes(&url).await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| ModrinthError::Parse(format!("list_versions {url}: {e}")))
    }

    /// `GET /v2/version/{id}` — single version.
    ///
    /// 08-RESEARCH.md §Endpoint #4. Used by 08-04 dep resolver when a dep pins
    /// a specific version_id (Q2 from 08-RESEARCH.md).
    #[tracing::instrument(skip_all, fields(version_id))]
    pub async fn get_version(&self, version_id: &str) -> Result<ModrinthVersion, ModrinthError> {
        if !is_safe_modrinth_slug(version_id) {
            return Err(ModrinthError::Http(format!(
                "invalid version_id: {version_id}"
            )));
        }
        let url = format!("{}/v2/version/{}", self.base_url, version_id);
        let bytes = self.send_get_bytes(&url).await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| ModrinthError::Parse(format!("get_version {url}: {e}")))
    }

    /// `GET /v2/version_file/{hash}?algorithm=sha512` — hash → version.
    ///
    /// 08-RESEARCH.md §Endpoint #5. Returns Ok(None) on 404. Caller uses this
    /// to resolve "is this manually-dropped JAR a known Modrinth version?"
    /// (Q3 from 08-RESEARCH.md, deferred to v2 — the method exists for
    /// forward-compat).
    #[tracing::instrument(
        skip_all,
        fields(sha512_prefix = &sha512_hex[..sha512_hex.len().min(8)])
    )]
    pub async fn version_from_hash(
        &self,
        sha512_hex: &str,
    ) -> Result<Option<ModrinthVersion>, ModrinthError> {
        // Hash is hex; reuse slug allowlist (matches `[A-Za-z0-9_-]`).
        // 128 hex chars expected for sha512.
        if !is_safe_modrinth_slug(sha512_hex) {
            return Err(ModrinthError::Http(format!(
                "invalid sha512 hex: {sha512_hex}"
            )));
        }
        let url = format!(
            "{}/v2/version_file/{}?algorithm=sha512",
            self.base_url, sha512_hex
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ModrinthError::Http(format!("GET {url}: {e}")))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if resp.status().as_u16() == 429 {
            return Err(rate_limit_from(&resp));
        }
        let resp = resp
            .error_for_status()
            .map_err(|e| ModrinthError::Http(format!("status {url}: {e}")))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ModrinthError::Http(format!("body {url}: {e}")))?;
        let v: ModrinthVersion = serde_json::from_slice(&bytes)
            .map_err(|e| ModrinthError::Parse(format!("version_from_hash {url}: {e}")))?;
        Ok(Some(v))
    }

    // --- Internal helpers ----------------------------------------------------

    /// Issue a GET, return body bytes. Maps 429 to `RateLimited`, other non-2xx
    /// to `Http`, network/body errors to `Http`.
    async fn send_get_bytes(&self, url: &str) -> Result<Vec<u8>, ModrinthError> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| ModrinthError::Http(format!("GET {url}: {e}")))?;
        if resp.status().as_u16() == 429 {
            return Err(rate_limit_from(&resp));
        }
        let resp = resp
            .error_for_status()
            .map_err(|e| ModrinthError::Http(format!("status {url}: {e}")))?;
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| ModrinthError::Http(format!("body {url}: {e}")))
    }
}

/// Parse 429 response into ModrinthError::RateLimited.
/// Falls back to 60s if Retry-After is absent or unparseable.
fn rate_limit_from(resp: &reqwest::Response) -> ModrinthError {
    let secs = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(60);
    ModrinthError::RateLimited {
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
    use serde_json::json;

    fn make_client(server: &MockServer) -> ModrinthClient {
        ModrinthClient::new_with_base_url(server.base_url()).expect("client::new_with_base_url")
    }

    // --- search --------------------------------------------------------------

    #[tokio::test]
    async fn test_search_url_shape_facets_and_query() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/search")
                .query_param("query", "sodium")
                .query_param("limit", "20")
                .query_param("index", "relevance")
                .query_param_exists("facets");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"hits":[],"offset":0,"limit":20,"total_hits":0}"#);
        });
        let c = make_client(&server);
        let _ = c
            .search("sodium", Some("1.20.4"), &["fabric"], 20)
            .await
            .unwrap();
        m.assert();
    }

    #[tokio::test]
    async fn test_search_user_agent_is_project_ua_not_default_reqwest() {
        // PITFALL 1 — Modrinth blocks default reqwest UA with 403.
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/search")
                .header_exists("User-Agent")
                .is_true(|req| {
                    let ua = req
                        .headers_vec()
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
                        .map(|(_, v)| v.as_str())
                        .unwrap_or("");
                    ua.starts_with("mineltui/")
                });
            then.status(200).body(r#"{"hits":[]}"#);
        });
        let c = make_client(&server);
        let _ = c.search("x", None, &[], 20).await.unwrap();
        m.assert();
    }

    #[tokio::test]
    async fn test_search_parses_hits() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/search");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    json!({
                        "hits":[
                            {"project_id":"AANobbMI","slug":"sodium","title":"Sodium",
                             "description":"Modern rendering","downloads":12345}
                        ],
                        "offset":0,"limit":20,"total_hits":1
                    })
                    .to_string(),
                );
        });
        let c = make_client(&server);
        let hits = c.search("sodium", None, &[], 20).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "sodium");
        assert_eq!(hits[0].project_id, "AANobbMI");
        assert!(
            !hits[0].already_installed,
            "client never sets this true — caller stamps"
        );
    }

    #[tokio::test]
    async fn test_search_omits_facets_when_no_filter() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/search")
                .query_param_missing("facets");
            then.status(200).body(r#"{"hits":[]}"#);
        });
        let c = make_client(&server);
        let _ = c.search("x", None, &[], 20).await.unwrap();
        m.assert();
    }

    #[tokio::test]
    async fn test_search_429_returns_rate_limited() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/search");
            then.status(429).header("Retry-After", "42").body("{}");
        });
        let c = make_client(&server);
        let r = c.search("x", None, &[], 20).await;
        match r {
            Err(ModrinthError::RateLimited { retry_after_secs }) => {
                assert_eq!(retry_after_secs, 42)
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_search_429_default_60_when_header_missing() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/search");
            then.status(429).body("{}");
        });
        let c = make_client(&server);
        let r = c.search("x", None, &[], 20).await;
        assert!(matches!(
            r,
            Err(ModrinthError::RateLimited {
                retry_after_secs: 60
            })
        ));
    }

    #[tokio::test]
    async fn test_search_500_returns_http() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/search");
            then.status(500).body("upstream blew up");
        });
        let c = make_client(&server);
        let r = c.search("x", None, &[], 20).await;
        assert!(matches!(r, Err(ModrinthError::Http(_))), "got {r:?}");
    }

    #[tokio::test]
    async fn test_search_malformed_json_returns_parse() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/search");
            then.status(200).body("{not-json");
        });
        let c = make_client(&server);
        let r = c.search("x", None, &[], 20).await;
        assert!(matches!(r, Err(ModrinthError::Parse(_))), "got {r:?}");
    }

    // --- get_project ---------------------------------------------------------

    #[tokio::test]
    async fn test_get_project_url_and_parse() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/v2/project/sodium");
            then.status(200).body(
                json!({
                    "id":"AANobbMI","slug":"sodium","title":"Sodium",
                    "description":"Modern rendering","body":"# Sodium\n...",
                    "downloads":12345,
                    "license":{"id":"LGPL-3.0-only"},
                    "categories":["library","optimization"]
                })
                .to_string(),
            );
        });
        let c = make_client(&server);
        let p = c.get_project("sodium").await.unwrap();
        m.assert();
        assert_eq!(p.title, "Sodium");
        assert_eq!(p.license_id, "LGPL-3.0-only");
        assert!(p.categories.contains(&"library".to_string()));
    }

    #[tokio::test]
    async fn test_get_project_rejects_unsafe_slug() {
        let server = MockServer::start();
        let c = make_client(&server);
        let r = c.get_project("../etc/passwd").await;
        assert!(matches!(r, Err(ModrinthError::Http(_))), "got {r:?}");
    }

    // --- list_versions -------------------------------------------------------

    #[tokio::test]
    async fn test_list_versions_url_shape() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/project/AANobbMI/version")
                .query_param("include_changelog", "false")
                .query_param_exists("loaders")
                .query_param_exists("game_versions");
            then.status(200).body("[]");
        });
        let c = make_client(&server);
        let _ = c
            .list_versions("AANobbMI", Some("1.20.4"), &["fabric"])
            .await
            .unwrap();
        m.assert();
    }

    #[tokio::test]
    async fn test_list_versions_parses_dependencies() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/project/AANobbMI/version");
            then.status(200).body(
                json!([{
                    "id":"Yp8wLY1P","project_id":"AANobbMI","name":"Sodium 0.5.8",
                    "version_number":"0.5.8","version_type":"release",
                    "game_versions":["1.20.4"],"loaders":["fabric"],
                    "downloads":1000,"date_published":"2026-01-01T00:00:00Z",
                    "dependencies":[{"project_id":"P7dR8mSH","dependency_type":"required"}],
                    "files":[{
                        "url":"https://cdn.modrinth.com/x.jar","filename":"x.jar","primary":true,
                        "size":1024,"hashes":{"sha1":"a","sha512":"b"}
                    }]
                }])
                .to_string(),
            );
        });
        let c = make_client(&server);
        let vs = c
            .list_versions("AANobbMI", Some("1.20.4"), &["fabric"])
            .await
            .unwrap();
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].dependencies.len(), 1);
        assert_eq!(
            vs[0].dependencies[0].project_id.as_deref(),
            Some("P7dR8mSH")
        );
        assert_eq!(vs[0].files[0].hashes.sha512, "b");
    }

    #[tokio::test]
    async fn test_list_versions_rejects_unsafe_project_id() {
        let server = MockServer::start();
        let c = make_client(&server);
        let r = c.list_versions("../passwd", None, &[]).await;
        assert!(matches!(r, Err(ModrinthError::Http(_))), "got {r:?}");
    }

    // --- get_version ---------------------------------------------------------

    #[tokio::test]
    async fn test_get_version_happy_path() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/version/Yp8wLY1P");
            then.status(200).body(
                json!({
                    "id":"Yp8wLY1P","project_id":"AANobbMI","name":"Sodium 0.5.8",
                    "version_number":"0.5.8","version_type":"release",
                    "game_versions":["1.20.4"],"loaders":["fabric"],
                    "downloads":1000,"date_published":"2026-01-01T00:00:00Z",
                    "files":[{
                        "url":"https://cdn.modrinth.com/x.jar","filename":"x.jar","primary":true,
                        "size":1024,"hashes":{"sha1":"a","sha512":"b"}
                    }]
                })
                .to_string(),
            );
        });
        let c = make_client(&server);
        let v = c.get_version("Yp8wLY1P").await.unwrap();
        assert_eq!(v.id, "Yp8wLY1P");
    }

    // --- version_from_hash ---------------------------------------------------

    #[tokio::test]
    async fn test_version_from_hash_some() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/v2/version_file/deadbeef")
                .query_param("algorithm", "sha512");
            then.status(200).body(
                json!({
                    "id":"Yp8wLY1P","project_id":"AANobbMI","name":"x","version_number":"0.0",
                    "version_type":"release","game_versions":["1.20.4"],"loaders":["fabric"],
                    "date_published":"2026-01-01T00:00:00Z",
                    "files":[{
                        "url":"https://cdn.modrinth.com/x.jar","filename":"x.jar","primary":true,
                        "size":1,"hashes":{"sha1":"a","sha512":"deadbeef"}
                    }]
                })
                .to_string(),
            );
        });
        let c = make_client(&server);
        let v = c.version_from_hash("deadbeef").await.unwrap();
        assert!(v.is_some());
    }

    #[tokio::test]
    async fn test_version_from_hash_404_returns_none() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/version_file/deadbeef");
            then.status(404).body("");
        });
        let c = make_client(&server);
        let v = c.version_from_hash("deadbeef").await.unwrap();
        assert!(v.is_none());
    }

    // --- get_projects_batch --------------------------------------------------

    #[tokio::test]
    async fn test_get_projects_batch_returns_id_title_pairs() {
        // Closes GAP-8-D — this endpoint feeds the dep-resolver title-hydration
        // pass that populates ResolvedDep.project_title.
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/projects")
                .query_param("ids", r#"["P7dR8mSH","AANobbMI"]"#);
            then.status(200).body(
                json!([
                    { "id": "P7dR8mSH", "title": "Fabric API" },
                    { "id": "AANobbMI", "title": "Sodium" }
                ])
                .to_string(),
            );
        });
        let c = make_client(&server);
        let pairs = c
            .get_projects_batch(&["P7dR8mSH", "AANobbMI"])
            .await
            .unwrap();
        m.assert();
        assert_eq!(pairs.len(), 2);
        assert!(
            pairs
                .iter()
                .any(|p| p.id == "P7dR8mSH" && p.title == "Fabric API"),
            "missing Fabric API entry: {pairs:?}"
        );
        assert!(
            pairs
                .iter()
                .any(|p| p.id == "AANobbMI" && p.title == "Sodium"),
            "missing Sodium entry: {pairs:?}"
        );
    }

    #[tokio::test]
    async fn test_get_projects_batch_empty_no_http() {
        // Empty input must return Ok(vec![]) WITHOUT issuing an HTTP call.
        // We point the client at an unreachable address; if the impl tried to
        // send a request, this would error or hang.
        let c = ModrinthClient::new_with_base_url("http://127.0.0.1:1".to_string()).unwrap();
        let pairs = c.get_projects_batch(&[]).await.unwrap();
        assert!(pairs.is_empty());
    }

    #[tokio::test]
    async fn test_get_projects_batch_429_returns_rate_limited() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/projects");
            then.status(429).header("Retry-After", "30").body("{}");
        });
        let c = make_client(&server);
        let r = c.get_projects_batch(&["P7dR8mSH"]).await;
        match r {
            Err(ModrinthError::RateLimited { retry_after_secs }) => {
                assert_eq!(retry_after_secs, 30)
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    // --- env override --------------------------------------------------------

    #[tokio::test]
    async fn test_env_override_base_url() {
        // Mirror the parallel-test-safe env restore pattern from
        // src/loader/fabric.rs::tests::test_env_override_base_url.
        // Use a tokio::sync::Mutex (file-local) to serialize this single test
        // since std::env mutation races with other tests.
        static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _guard = ENV_LOCK.lock().await;
        let prev = std::env::var(MODRINTH_BASE_URL_ENV).ok();
        std::env::set_var(MODRINTH_BASE_URL_ENV, "https://example.invalid");
        let c = ModrinthClient::new().expect("new with env");
        assert_eq!(c.base_url(), "https://example.invalid");
        match prev {
            Some(v) => std::env::set_var(MODRINTH_BASE_URL_ENV, v),
            None => std::env::remove_var(MODRINTH_BASE_URL_ENV),
        }
    }
}
