//! Fabric meta API HTTP client.
//!
//! Endpoints (verified 2026-04-26 — see 06-RESEARCH.md §API Reference):
//!   GET /v2/versions/loader
//!   GET /v2/versions/loader/{game_version}/{loader_version}/profile/json
//!
//! Override the base URL for tests via `MINELTUI_FABRIC_META_BASE_URL`.

use std::time::Duration;

use serde::Deserialize;

use crate::loader::error::LoaderError;
use crate::loader::types::{LoaderLibrary, LoaderVersionEntry};

pub const DEFAULT_FABRIC_META_BASE: &str = "https://meta.fabricmc.net";
pub const FABRIC_META_BASE_URL_ENV: &str = "MINELTUI_FABRIC_META_BASE_URL";

// -----------------------------------------------------------------------
// Wire types — Fabric meta response shapes (private; map to LoaderVersionEntry)
// -----------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FabricLoaderItem {
    pub version: String,
    pub stable: bool,
    #[serde(default)]
    pub build: u32,
}

// LoaderLibrary is the canonical shared type defined in src/loader/types.rs
// by 06-01. Fabric and Quilt clients both import this single nominal type.

/// Parsed loader profile returned by `fetch_profile`.
/// `raw_bytes` is the verbatim API response body (written to disk by 06-05
/// to preserve any future fields not yet known to this struct).
#[derive(Debug, Clone)]
pub struct FabricProfile {
    pub id: String,
    pub raw_bytes: Vec<u8>,
    pub libraries: Vec<LoaderLibrary>,
}

#[derive(Debug, Deserialize)]
struct FabricProfileJson {
    pub id: String,
    #[serde(default)]
    pub libraries: Vec<LoaderLibrary>,
}

// -----------------------------------------------------------------------
// Client
// -----------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FabricMetaClient {
    http: reqwest::Client,
    base_url: String,
}

impl FabricMetaClient {
    pub fn new() -> Result<Self, LoaderError> {
        let base_url = std::env::var(FABRIC_META_BASE_URL_ENV)
            .unwrap_or_else(|_| DEFAULT_FABRIC_META_BASE.to_owned());
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
                loader: "fabric",
                reason: format!("reqwest build: {e}"),
            })?;
        Ok(Self { http, base_url: base_url.into() })
    }

    #[tracing::instrument(skip_all)]
    pub async fn list_loader_versions(&self) -> Result<Vec<LoaderVersionEntry>, LoaderError> {
        let url = format!("{}/v2/versions/loader", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| LoaderError::MetaFetch {
                loader: "fabric",
                reason: format!("GET {url}: {e}"),
            })?
            .error_for_status()
            .map_err(|e| LoaderError::MetaFetch {
                loader: "fabric",
                reason: format!("status {url}: {e}"),
            })?;

        let bytes = resp.bytes().await.map_err(|e| LoaderError::MetaFetch {
            loader: "fabric",
            reason: format!("body {url}: {e}"),
        })?;

        let items: Vec<FabricLoaderItem> =
            serde_json::from_slice(&bytes).map_err(|e| LoaderError::MetaParse {
                loader: "fabric",
                reason: format!("loader list: {e}"),
            })?;

        Ok(items
            .into_iter()
            .map(|i| LoaderVersionEntry {
                version: i.version,
                stable: i.stable,
                build: Some(i.build),
            })
            .collect())
    }

    /// Borrow the underlying reqwest client for library downloads.
    /// Reusing this client preserves connection pooling and respects the
    /// same timeouts as meta API calls.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    #[tracing::instrument(skip_all, fields(game_version, loader_version))]
    pub async fn fetch_profile(
        &self,
        game_version: &str,
        loader_version: &str,
    ) -> Result<FabricProfile, LoaderError> {
        let url = format!(
            "{}/v2/versions/loader/{game_version}/{loader_version}/profile/json",
            self.base_url
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| LoaderError::MetaFetch {
                loader: "fabric",
                reason: format!("GET {url}: {e}"),
            })?
            .error_for_status()
            .map_err(|e| LoaderError::MetaFetch {
                loader: "fabric",
                reason: format!("status {url}: {e}"),
            })?;

        let raw_bytes = resp
            .bytes()
            .await
            .map_err(|e| LoaderError::MetaFetch {
                loader: "fabric",
                reason: format!("body {url}: {e}"),
            })?
            .to_vec();

        let parsed: FabricProfileJson =
            serde_json::from_slice(&raw_bytes).map_err(|e| LoaderError::MetaParse {
                loader: "fabric",
                reason: format!("profile json: {e}"),
            })?;

        Ok(FabricProfile {
            id: parsed.id,
            raw_bytes,
            libraries: parsed.libraries,
        })
    }
}

// -----------------------------------------------------------------------
// Tests — httpmock-driven, no env-var mutation
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::GET;
    use httpmock::MockServer;

    fn make_client(server: &MockServer) -> FabricMetaClient {
        FabricMetaClient::new_with_base_url(server.base_url()).expect("client build")
    }

    // ---- list_loader_versions ----

    #[tokio::test]
    async fn test_list_loader_versions_parses_stable_and_unstable() {
        let server = MockServer::start();
        let body = r#"[
            {"version":"0.16.9","stable":true,"maven":"net.fabricmc:fabric-loader:0.16.9","build":509,"separator":"."},
            {"version":"0.17.0-beta.1","stable":false,"maven":"net.fabricmc:fabric-loader:0.17.0-beta.1","build":600,"separator":"-"}
        ]"#;
        server.mock(|when, then| {
            when.method(GET).path("/v2/versions/loader");
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });

        let client = make_client(&server);
        let v = client.list_loader_versions().await.expect("ok");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].version, "0.16.9");
        assert!(v[0].stable);
        assert_eq!(v[0].build, Some(509));
        assert_eq!(v[1].version, "0.17.0-beta.1");
        assert!(!v[1].stable);
    }

    #[tokio::test]
    async fn test_list_loader_versions_http_error_maps_to_metafetch() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/versions/loader");
            then.status(500).body("upstream error");
        });

        let client = make_client(&server);
        let r = client.list_loader_versions().await;
        assert!(matches!(r, Err(LoaderError::MetaFetch { loader: "fabric", .. })));
    }

    #[tokio::test]
    async fn test_list_loader_versions_bad_json_maps_to_metaparse() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/versions/loader");
            then.status(200).body("not json at all");
        });

        let client = make_client(&server);
        let r = client.list_loader_versions().await;
        assert!(matches!(r, Err(LoaderError::MetaParse { loader: "fabric", .. })));
    }

    // ---- fetch_profile ----

    #[tokio::test]
    async fn test_fetch_profile_parses_id_and_libraries_with_sha1() {
        let server = MockServer::start();
        let body = r#"{
            "id": "fabric-loader-0.16.9-1.21.4",
            "inheritsFrom": "1.21.4",
            "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
            "arguments": { "game": [], "jvm": ["-DFabricMcEmu= net.minecraft.client.main.Main"] },
            "libraries": [
                {
                    "name": "org.ow2.asm:asm:9.7.1",
                    "url": "https://maven.fabricmc.net/",
                    "sha1": "f0ed132a49244b042cd0e15702ab9f2ce3cc8436",
                    "sha256": "aa",
                    "size": 65000
                },
                {
                    "name": "net.fabricmc:fabric-loader:0.16.9",
                    "url": "https://maven.fabricmc.net/",
                    "sha1": "abc"
                }
            ]
        }"#;
        server.mock(|when, then| {
            when.method(GET)
                .path("/v2/versions/loader/1.21.4/0.16.9/profile/json");
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });

        let client = make_client(&server);
        let p = client
            .fetch_profile("1.21.4", "0.16.9")
            .await
            .expect("fetch_profile ok");

        assert_eq!(p.id, "fabric-loader-0.16.9-1.21.4");
        assert_eq!(p.libraries.len(), 2);
        assert_eq!(p.libraries[0].name, "org.ow2.asm:asm:9.7.1");
        assert_eq!(p.libraries[0].sha1.as_deref(), Some("f0ed132a49244b042cd0e15702ab9f2ce3cc8436"));
        assert_eq!(p.libraries[0].size, Some(65000));
        // raw_bytes is preserved verbatim for atomic_write later
        assert!(p.raw_bytes.len() > 200);
        assert!(std::str::from_utf8(&p.raw_bytes).unwrap().contains("inheritsFrom"));
    }

    #[tokio::test]
    async fn test_fetch_profile_404_maps_to_metafetch() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/v2/versions/loader/1.21.4/0.99.99/profile/json");
            then.status(404).body("not found");
        });

        let client = make_client(&server);
        let r = client.fetch_profile("1.21.4", "0.99.99").await;
        assert!(matches!(r, Err(LoaderError::MetaFetch { loader: "fabric", .. })));
    }

    // ---- env override ----

    #[tokio::test]
    async fn test_env_override_base_url() {
        let server = MockServer::start();
        let api_mock = server.mock(|when, then| {
            when.method(GET).path("/v2/versions/loader");
            then.status(200).body("[]");
        });

        // Save and restore env to keep parallel tests deterministic.
        let prior = std::env::var(FABRIC_META_BASE_URL_ENV).ok();
        std::env::set_var(FABRIC_META_BASE_URL_ENV, server.base_url());

        let client = FabricMetaClient::new().expect("client from env");
        let result = client.list_loader_versions().await;

        match prior {
            Some(v) => std::env::set_var(FABRIC_META_BASE_URL_ENV, v),
            None => std::env::remove_var(FABRIC_META_BASE_URL_ENV),
        }

        result.expect("list_loader_versions via env should succeed");
        api_mock.assert_calls(1);
    }
}
