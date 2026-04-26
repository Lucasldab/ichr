//! Quilt meta API HTTP client.
//!
//! Endpoints (verified 2026-04-26 — see 06-RESEARCH.md §API Reference):
//!   GET /v3/versions/loader
//!   GET /v3/versions/loader/{game_version}/{loader_version}/profile/json
//!
//! Differences from Fabric:
//! - Path prefix is `/v3/` not `/v2/`.
//! - Loader list has NO `stable` boolean — derive from version string
//!   (presence of `"beta"`, `"rc"`, or `"pre"` => pre-release).
//! - Profile JSON libraries are `{name, url}` only — no hash fields.
//!
//! Override the base URL for tests via `MINELTUI_QUILT_META_BASE_URL`.

use std::time::Duration;

use serde::Deserialize;

use crate::loader::error::LoaderError;
use crate::loader::types::{LoaderLibrary, LoaderVersionEntry};

pub const DEFAULT_QUILT_META_BASE: &str = "https://meta.quiltmc.org";
pub const QUILT_META_BASE_URL_ENV: &str = "MINELTUI_QUILT_META_BASE_URL";

// -----------------------------------------------------------------------
// Wire types
// -----------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct QuiltLoaderItem {
    pub version: String,
    #[serde(default)]
    pub build: u32,
}

// LoaderLibrary is the canonical shared type defined in src/loader/types.rs
// by 06-01. Quilt parses every hash field (sha1/sha256/sha512/md5) as None
// — asserted by test_fetch_profile_no_hashes_on_libraries (06-04-01).

/// Parsed Quilt loader profile. `raw_bytes` is the verbatim API response,
/// written verbatim to disk by 06-05 to preserve any future fields.
#[derive(Debug, Clone)]
pub struct QuiltProfile {
    pub id: String,
    pub raw_bytes: Vec<u8>,
    pub libraries: Vec<LoaderLibrary>,
}

#[derive(Debug, Deserialize)]
struct QuiltProfileJson {
    pub id: String,
    #[serde(default)]
    pub libraries: Vec<LoaderLibrary>,
}

/// Quilt has no `stable` boolean on its loader list. Pre-release versions
/// contain `beta`, `rc`, or `pre` (case-insensitive). Anything else is stable.
pub fn is_quilt_stable(version: &str) -> bool {
    let lower = version.to_ascii_lowercase();
    !lower.contains("beta") && !lower.contains("rc") && !lower.contains("pre")
}

// -----------------------------------------------------------------------
// Client
// -----------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct QuiltMetaClient {
    http: reqwest::Client,
    base_url: String,
}

impl QuiltMetaClient {
    pub fn new() -> Result<Self, LoaderError> {
        let base_url = std::env::var(QUILT_META_BASE_URL_ENV)
            .unwrap_or_else(|_| DEFAULT_QUILT_META_BASE.to_owned());
        Self::new_with_base_url(base_url)
    }

    pub fn new_with_base_url(base_url: impl Into<String>) -> Result<Self, LoaderError> {
        let http = reqwest::Client::builder()
            .user_agent(crate::mojang::client::USER_AGENT)
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| LoaderError::MetaFetch {
                loader: "quilt",
                reason: format!("reqwest build: {e}"),
            })?;
        Ok(Self { http, base_url: base_url.into() })
    }

    #[tracing::instrument(skip_all)]
    pub async fn list_loader_versions(&self) -> Result<Vec<LoaderVersionEntry>, LoaderError> {
        let url = format!("{}/v3/versions/loader", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| LoaderError::MetaFetch {
                loader: "quilt",
                reason: format!("GET {url}: {e}"),
            })?
            .error_for_status()
            .map_err(|e| LoaderError::MetaFetch {
                loader: "quilt",
                reason: format!("status {url}: {e}"),
            })?;

        let bytes = resp.bytes().await.map_err(|e| LoaderError::MetaFetch {
            loader: "quilt",
            reason: format!("body {url}: {e}"),
        })?;

        let items: Vec<QuiltLoaderItem> =
            serde_json::from_slice(&bytes).map_err(|e| LoaderError::MetaParse {
                loader: "quilt",
                reason: format!("loader list: {e}"),
            })?;

        Ok(items
            .into_iter()
            .map(|i| LoaderVersionEntry {
                stable: is_quilt_stable(&i.version),
                version: i.version,
                build: Some(i.build),
            })
            .collect())
    }

    #[tracing::instrument(skip_all, fields(game_version, loader_version))]
    pub async fn fetch_profile(
        &self,
        game_version: &str,
        loader_version: &str,
    ) -> Result<QuiltProfile, LoaderError> {
        let url = format!(
            "{}/v3/versions/loader/{game_version}/{loader_version}/profile/json",
            self.base_url
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| LoaderError::MetaFetch {
                loader: "quilt",
                reason: format!("GET {url}: {e}"),
            })?
            .error_for_status()
            .map_err(|e| LoaderError::MetaFetch {
                loader: "quilt",
                reason: format!("status {url}: {e}"),
            })?;

        let raw_bytes = resp
            .bytes()
            .await
            .map_err(|e| LoaderError::MetaFetch {
                loader: "quilt",
                reason: format!("body {url}: {e}"),
            })?
            .to_vec();

        let parsed: QuiltProfileJson =
            serde_json::from_slice(&raw_bytes).map_err(|e| LoaderError::MetaParse {
                loader: "quilt",
                reason: format!("profile json: {e}"),
            })?;

        Ok(QuiltProfile {
            id: parsed.id,
            raw_bytes,
            libraries: parsed.libraries,
        })
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::GET;
    use httpmock::MockServer;

    fn make_client(server: &MockServer) -> QuiltMetaClient {
        QuiltMetaClient::new_with_base_url(server.base_url()).expect("client build")
    }

    // ---- is_quilt_stable ----

    #[test]
    fn test_is_quilt_stable_pure_classifier() {
        assert!(is_quilt_stable("0.27.2"));
        assert!(is_quilt_stable("1.0.0"));
        assert!(!is_quilt_stable("0.30.0-beta.7"));
        assert!(!is_quilt_stable("0.30.0-rc.1"));
        assert!(!is_quilt_stable("0.30.0-pre.4"));
        assert!(!is_quilt_stable("0.30.0-BETA.7")); // case-insensitive
    }

    // ---- list_loader_versions ----

    #[tokio::test]
    async fn test_list_loader_versions_derives_stable_from_version_string() {
        let server = MockServer::start();
        // Quilt API has NO stable field — only version + build.
        let body = r#"[
            {"version":"0.30.0-beta.7","maven":"org.quiltmc:quilt-loader:0.30.0-beta.7","build":120,"separator":"-"},
            {"version":"0.27.2","maven":"org.quiltmc:quilt-loader:0.27.2","build":50,"separator":"."}
        ]"#;
        server.mock(|when, then| {
            when.method(GET).path("/v3/versions/loader");
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });

        let client = make_client(&server);
        let v = client.list_loader_versions().await.expect("ok");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].version, "0.30.0-beta.7");
        assert!(!v[0].stable, "beta should be unstable");
        assert_eq!(v[1].version, "0.27.2");
        assert!(v[1].stable, "no beta/rc/pre => stable");
    }

    #[tokio::test]
    async fn test_list_loader_versions_5xx_maps_to_metafetch() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v3/versions/loader");
            then.status(503).body("upstream");
        });

        let client = make_client(&server);
        let r = client.list_loader_versions().await;
        assert!(matches!(r, Err(LoaderError::MetaFetch { loader: "quilt", .. })));
    }

    #[tokio::test]
    async fn test_list_loader_versions_bad_json_maps_to_metaparse() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v3/versions/loader");
            then.status(200).body("nonsense");
        });

        let client = make_client(&server);
        let r = client.list_loader_versions().await;
        assert!(matches!(r, Err(LoaderError::MetaParse { loader: "quilt", .. })));
    }

    // ---- fetch_profile ----

    #[tokio::test]
    async fn test_fetch_profile_no_hashes_on_libraries() {
        let server = MockServer::start();
        // Verified shape: Quilt libraries are `{name, url}` only.
        let body = r#"{
            "id": "quilt-loader-0.30.0-beta.7-1.21.4",
            "inheritsFrom": "1.21.4",
            "mainClass": "org.quiltmc.loader.impl.launch.knot.KnotClient",
            "arguments": { "game": [] },
            "libraries": [
                {"name": "org.quiltmc:quilt-loader:0.30.0-beta.7", "url": "https://maven.quiltmc.org/"},
                {"name": "net.fabricmc:intermediary:1.21.4", "url": "https://maven.fabricmc.net/"}
            ]
        }"#;
        server.mock(|when, then| {
            when.method(GET)
                .path("/v3/versions/loader/1.21.4/0.30.0-beta.7/profile/json");
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });

        let client = make_client(&server);
        let p = client
            .fetch_profile("1.21.4", "0.30.0-beta.7")
            .await
            .expect("fetch_profile ok");

        assert_eq!(p.id, "quilt-loader-0.30.0-beta.7-1.21.4");
        assert_eq!(p.libraries.len(), 2);
        // Critical assertion per Pattern 6: Quilt has NO hashes (parse-time invariant).
        assert!(p.libraries[0].sha1.is_none(), "Quilt libs must NOT have sha1");
        assert!(p.libraries[0].sha256.is_none(), "Quilt libs must NOT have sha256");
        assert!(p.libraries[0].sha512.is_none(), "Quilt libs must NOT have sha512");
        assert!(p.libraries[0].md5.is_none(), "Quilt libs must NOT have md5");
        assert!(p.libraries[0].size.is_none(), "Quilt libs must NOT have size");
        // Same for the second library
        assert!(p.libraries[1].sha1.is_none());
        assert!(p.libraries[1].sha256.is_none());
        assert!(p.libraries[1].sha512.is_none());
        assert!(p.libraries[1].md5.is_none());
        // Pitfall 3: intermediary library is included as a regular library
        assert_eq!(p.libraries[1].name, "net.fabricmc:intermediary:1.21.4");
        // Raw bytes are preserved for verbatim disk write
        assert!(p.raw_bytes.len() > 200);
    }

    #[tokio::test]
    async fn test_fetch_profile_404_maps_to_metafetch() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/v3/versions/loader/1.21.4/9.9.9/profile/json");
            then.status(404).body("not found");
        });

        let client = make_client(&server);
        let r = client.fetch_profile("1.21.4", "9.9.9").await;
        assert!(matches!(r, Err(LoaderError::MetaFetch { loader: "quilt", .. })));
    }

    // ---- env override ----

    #[tokio::test]
    async fn test_env_override_base_url() {
        let server = MockServer::start();
        let api_mock = server.mock(|when, then| {
            when.method(GET).path("/v3/versions/loader");
            then.status(200).body("[]");
        });

        let prior = std::env::var(QUILT_META_BASE_URL_ENV).ok();
        std::env::set_var(QUILT_META_BASE_URL_ENV, server.base_url());

        let client = QuiltMetaClient::new().expect("client from env");
        let result = client.list_loader_versions().await;

        match prior {
            Some(v) => std::env::set_var(QUILT_META_BASE_URL_ENV, v),
            None => std::env::remove_var(QUILT_META_BASE_URL_ENV),
        }

        result.expect("env-override list_loader_versions");
        api_mock.assert_calls(1);
    }
}
