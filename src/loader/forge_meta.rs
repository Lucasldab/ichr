//! Forge meta API HTTP client.
//!
//! Endpoints (verified 2026-05-06 — see 07-RESEARCH.md §Forge Endpoint Reference):
//!   GET https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml
//!   GET https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json
//!   GET https://maven.minecraftforge.net/net/minecraftforge/forge/{mc}-{forge}/forge-{mc}-{forge}-installer.jar
//!
//! Override base URLs for tests via `MINELTUI_FORGE_MAVEN_BASE_URL` and
//! `MINELTUI_FORGE_PROMOTIONS_URL`.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::loader::error::LoaderError;
use crate::loader::maven_metadata;
use crate::loader::types::LoaderVersionEntry;

pub const DEFAULT_FORGE_MAVEN_BASE: &str =
    "https://maven.minecraftforge.net/net/minecraftforge/forge";
pub const DEFAULT_FORGE_PROMOTIONS_URL: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
pub const FORGE_MAVEN_BASE_URL_ENV: &str = "MINELTUI_FORGE_MAVEN_BASE_URL";
pub const FORGE_PROMOTIONS_URL_ENV: &str = "MINELTUI_FORGE_PROMOTIONS_URL";

// -----------------------------------------------------------------------
// Wire types
// -----------------------------------------------------------------------

/// Parsed `promotions_slim.json` response.
///
/// Shape:
/// ```json
/// {
///   "homepage": "...",
///   "promos": {
///     "1.20.1-recommended": "47.4.20",
///     "1.20.1-latest": "47.4.20",
///     "1.21.8-latest": "58.0.3"
///   }
/// }
/// ```
/// Note: `promos` values are BARE Forge versions (e.g., `"47.4.20"`), NOT
/// prefixed with the MC version.
#[derive(Debug, Default, Clone, Deserialize)]
struct PromotionsSlim {
    #[serde(default)]
    promos: HashMap<String, String>,
}

impl PromotionsSlim {
    /// Returns true if `bare_forge_version` matches either the recommended
    /// OR latest pin for `mc_version`. `promos` values are bare versions
    /// (e.g., `"47.4.20"`) — NOT prefixed with the MC version.
    fn is_recommended_or_latest(&self, mc_version: &str, bare_forge_version: &str) -> bool {
        let recommended_key = format!("{mc_version}-recommended");
        let latest_key = format!("{mc_version}-latest");
        let r = self
            .promos
            .get(&recommended_key)
            .map(|v| v == bare_forge_version)
            .unwrap_or(false);
        let l = self
            .promos
            .get(&latest_key)
            .map(|v| v == bare_forge_version)
            .unwrap_or(false);
        r || l
    }
}

// -----------------------------------------------------------------------
// Client
// -----------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ForgeMetaClient {
    http: reqwest::Client,
    maven_base: String,
    promotions_url: String,
}

impl ForgeMetaClient {
    pub fn new() -> Result<Self, LoaderError> {
        let maven_base = std::env::var(FORGE_MAVEN_BASE_URL_ENV)
            .unwrap_or_else(|_| DEFAULT_FORGE_MAVEN_BASE.to_owned());
        let promotions_url = std::env::var(FORGE_PROMOTIONS_URL_ENV)
            .unwrap_or_else(|_| DEFAULT_FORGE_PROMOTIONS_URL.to_owned());
        Self::new_with_base_urls(maven_base, promotions_url)
    }

    pub fn new_with_base_urls(
        maven_base: impl Into<String>,
        promotions_url: impl Into<String>,
    ) -> Result<Self, LoaderError> {
        let http = reqwest::Client::builder()
            .user_agent(crate::mojang::client::USER_AGENT)
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| LoaderError::MavenMetadataFetch {
                reason: format!("reqwest build (forge): {e}"),
            })?;
        Ok(Self {
            http,
            maven_base: maven_base.into(),
            promotions_url: promotions_url.into(),
        })
    }

    /// List Forge loader versions compatible with `mc_version`.
    ///
    /// Fetches `maven-metadata.xml`, filters entries by `"{mc_version}-"` prefix,
    /// strips the prefix to get bare Forge versions, and augments each with a
    /// `stable` boolean from `promotions_slim.json` (best-effort — promotions
    /// failure degrades gracefully to `stable: false` for all entries).
    #[tracing::instrument(skip_all, fields(mc_version = %mc_version))]
    pub async fn list_loader_versions(
        &self,
        mc_version: &str,
    ) -> Result<Vec<LoaderVersionEntry>, LoaderError> {
        // 1. Fetch maven-metadata.xml
        let url = format!("{}/maven-metadata.xml", self.maven_base.trim_end_matches('/'));
        let xml = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| LoaderError::MavenMetadataFetch {
                reason: format!("GET {url}: {e}"),
            })?
            .error_for_status()
            .map_err(|e| LoaderError::MavenMetadataFetch {
                reason: format!("status {url}: {e}"),
            })?
            .text()
            .await
            .map_err(|e| LoaderError::MavenMetadataFetch {
                reason: format!("body {url}: {e}"),
            })?;

        // 2. Extract all versions (returns empty Vec on malformed/empty XML — not an error)
        let all = maven_metadata::extract_versions(&xml);

        // 3. Fetch promotions (best-effort — empty on failure, D-05 graceful degradation)
        let promotions = self.fetch_promotions().await.unwrap_or_default();

        // 4. Filter by MC prefix, strip prefix to get bare Forge version, apply stable
        let prefix = format!("{mc_version}-");
        let entries: Vec<LoaderVersionEntry> = all
            .into_iter()
            .filter(|v| v.starts_with(&prefix))
            .map(|v| {
                let bare = v.strip_prefix(&prefix).unwrap_or(&v).to_string();
                let stable = promotions.is_recommended_or_latest(mc_version, &bare);
                LoaderVersionEntry { version: bare, stable, build: None }
            })
            .collect();

        Ok(entries)
    }

    /// Fetch and parse `promotions_slim.json`. On any failure, returns an
    /// error (callers use `unwrap_or_default()` for graceful degradation).
    async fn fetch_promotions(&self) -> Result<PromotionsSlim, LoaderError> {
        let resp = self
            .http
            .get(&self.promotions_url)
            .send()
            .await
            .map_err(|e| LoaderError::MetaFetch {
                loader: "forge",
                reason: format!("GET {}: {e}", self.promotions_url),
            })?
            .error_for_status()
            .map_err(|e| LoaderError::MetaFetch {
                loader: "forge",
                reason: format!("status {}: {e}", self.promotions_url),
            })?;

        let bytes = resp.bytes().await.map_err(|e| LoaderError::MetaFetch {
            loader: "forge",
            reason: format!("body {}: {e}", self.promotions_url),
        })?;

        let parsed: PromotionsSlim = serde_json::from_slice(&bytes).map_err(|e| {
            LoaderError::MetaParse {
                loader: "forge",
                reason: format!("promotions JSON: {e}"),
            }
        })?;

        Ok(parsed)
    }

    /// Return the installer JAR URL for a given `mc_version` + `forge_version`.
    ///
    /// Template (verified 2026-05-06):
    /// `{base}/{mc}-{forge}/forge-{mc}-{forge}-installer.jar`
    pub fn installer_url(&self, mc_version: &str, forge_version: &str) -> String {
        format!(
            "{}/{mc_version}-{forge_version}/forge-{mc_version}-{forge_version}-installer.jar",
            self.maven_base.trim_end_matches('/')
        )
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

    fn make_client(server: &MockServer) -> ForgeMetaClient {
        ForgeMetaClient::new_with_base_urls(
            server.base_url(),
            format!("{}/promotions_slim.json", server.base_url()),
        )
        .expect("client build")
    }

    /// A minimal but representative maven-metadata.xml fixture with three
    /// MC versions so we can verify the MC prefix filter.
    const FORGE_META_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>net.minecraftforge</groupId>
  <artifactId>forge</artifactId>
  <versioning>
    <versions>
      <version>1.20.1-47.4.20</version>
      <version>1.20.1-47.3.0</version>
      <version>1.21.8-58.0.3</version>
      <version>1.16.5-36.2.42</version>
    </versions>
  </versioning>
</metadata>"#;

    // ---- list_loader_versions ----

    #[tokio::test]
    async fn test_list_loader_versions_filters_by_mc_prefix() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/maven-metadata.xml");
            then.status(200)
                .header("content-type", "application/xml")
                .body(FORGE_META_XML);
        });
        // Promotions returns empty (no stable pins)
        server.mock(|when, then| {
            when.method(GET).path("/promotions_slim.json");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"promos":{}}"#);
        });

        let client = make_client(&server);
        let versions = client.list_loader_versions("1.20.1").await.expect("ok");

        // Fixture has two 1.20.1 entries (47.4.20 and 47.3.0) and two non-1.20.1 entries;
        // only the 1.20.1 entries should be returned.
        assert_eq!(versions.len(), 2, "only 1.20.1 entries returned: {versions:?}");
        assert!(versions.iter().all(|v| !v.stable), "no promo pin => all unstable");
        assert!(versions.iter().all(|v| v.build.is_none()));
        // Ensure non-matching MC versions were excluded
        assert!(versions.iter().all(|v| v.version != "58.0.3"), "1.21.8 entry must be excluded");
        assert!(versions.iter().all(|v| v.version != "36.2.42"), "1.16.5 entry must be excluded");
    }

    #[tokio::test]
    async fn test_promotions_marks_recommended_as_stable() {
        let server = MockServer::start();
        // Two Forge 1.20.1 versions
        let xml = r#"<?xml version="1.0"?><metadata><versioning><versions>
            <version>1.20.1-47.4.20</version>
            <version>1.20.1-47.3.0</version>
        </versions></versioning></metadata>"#;
        server.mock(|when, then| {
            when.method(GET).path("/maven-metadata.xml");
            then.status(200).body(xml);
        });
        // 47.4.20 is "recommended"
        server.mock(|when, then| {
            when.method(GET).path("/promotions_slim.json");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"promos":{"1.20.1-recommended":"47.4.20"}}"#);
        });

        let client = make_client(&server);
        let versions = client.list_loader_versions("1.20.1").await.expect("ok");
        assert_eq!(versions.len(), 2);

        // Find by version string
        let v4_20 = versions.iter().find(|v| v.version == "47.4.20").expect("47.4.20");
        let v3_0 = versions.iter().find(|v| v.version == "47.3.0").expect("47.3.0");
        assert!(v4_20.stable, "47.4.20 matches recommended => stable");
        assert!(!v3_0.stable, "47.3.0 has no pin => unstable");
    }

    #[tokio::test]
    async fn test_promotions_marks_latest_as_stable() {
        let server = MockServer::start();
        let xml = r#"<?xml version="1.0"?><metadata><versioning><versions>
            <version>1.20.1-47.4.20</version>
            <version>1.20.1-47.3.0</version>
        </versions></versioning></metadata>"#;
        server.mock(|when, then| {
            when.method(GET).path("/maven-metadata.xml");
            then.status(200).body(xml);
        });
        // 47.4.20 is "latest" (not "recommended")
        server.mock(|when, then| {
            when.method(GET).path("/promotions_slim.json");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"promos":{"1.20.1-latest":"47.4.20"}}"#);
        });

        let client = make_client(&server);
        let versions = client.list_loader_versions("1.20.1").await.expect("ok");

        let v4_20 = versions.iter().find(|v| v.version == "47.4.20").expect("47.4.20");
        let v3_0 = versions.iter().find(|v| v.version == "47.3.0").expect("47.3.0");
        assert!(v4_20.stable, "47.4.20 matches latest => stable");
        assert!(!v3_0.stable, "47.3.0 has no pin => unstable");
    }

    #[tokio::test]
    async fn test_installer_url_format() {
        // Pure-function test — no HTTP mock needed
        let client = ForgeMetaClient::new_with_base_urls(
            "https://maven.minecraftforge.net/net/minecraftforge/forge",
            "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json",
        )
        .expect("client build");
        let url = client.installer_url("1.20.1", "47.4.20");
        assert!(
            url.ends_with("/1.20.1-47.4.20/forge-1.20.1-47.4.20-installer.jar"),
            "unexpected URL: {url}"
        );
        assert!(
            url.contains("forge-1.20.1-47.4.20-installer.jar"),
            "filename wrong: {url}"
        );
    }

    #[tokio::test]
    async fn test_http_500_returns_maven_metadata_fetch() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/maven-metadata.xml");
            then.status(500).body("internal server error");
        });

        let client = make_client(&server);
        let r = client.list_loader_versions("1.20.1").await;
        assert!(
            matches!(r, Err(LoaderError::MavenMetadataFetch { .. })),
            "expected MavenMetadataFetch, got: {r:?}"
        );
    }

    #[tokio::test]
    async fn test_malformed_xml_returns_empty_list() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/maven-metadata.xml");
            then.status(200)
                .header("content-type", "application/xml")
                .body("<not-metadata />");
        });
        server.mock(|when, then| {
            when.method(GET).path("/promotions_slim.json");
            then.status(200).body(r#"{"promos":{}}"#);
        });

        let client = make_client(&server);
        let r = client.list_loader_versions("1.20.1").await.expect("ok — empty is not an error");
        assert!(r.is_empty(), "malformed XML => empty list (D-05 graceful state): {r:?}");
    }

    #[tokio::test]
    async fn test_promotions_failure_falls_back_to_unstable() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/maven-metadata.xml");
            then.status(200).body(FORGE_META_XML);
        });
        // Promotions endpoint returns 500
        server.mock(|when, then| {
            when.method(GET).path("/promotions_slim.json");
            then.status(500).body("error");
        });

        let client = make_client(&server);
        let versions = client.list_loader_versions("1.20.1").await.expect("ok — promotions failure is graceful");
        // Fixture has two 1.20.1 entries; both should be returned, all unstable
        assert_eq!(versions.len(), 2, "expected 1.20.1 entries despite promo failure");
        for v in &versions {
            assert!(!v.stable, "all stable=false when promotions unavailable: {v:?}");
        }
    }

    #[tokio::test]
    async fn test_env_override_base_urls() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/maven-metadata.xml");
            then.status(200)
                .header("content-type", "application/xml")
                .body(FORGE_META_XML);
        });
        let promotions_path = "/net/minecraftforge/forge/promotions_slim.json";
        server.mock(|when, then| {
            when.method(GET).path(promotions_path);
            then.status(200).body(r#"{"promos":{}}"#);
        });

        let prior_base = std::env::var(FORGE_MAVEN_BASE_URL_ENV).ok();
        let prior_promo = std::env::var(FORGE_PROMOTIONS_URL_ENV).ok();

        std::env::set_var(FORGE_MAVEN_BASE_URL_ENV, server.base_url());
        std::env::set_var(
            FORGE_PROMOTIONS_URL_ENV,
            format!("{}{promotions_path}", server.base_url()),
        );

        let client = ForgeMetaClient::new().expect("client from env");
        let result = client.list_loader_versions("1.20.1").await;

        // Restore env
        match prior_base {
            Some(v) => std::env::set_var(FORGE_MAVEN_BASE_URL_ENV, v),
            None => std::env::remove_var(FORGE_MAVEN_BASE_URL_ENV),
        }
        match prior_promo {
            Some(v) => std::env::set_var(FORGE_PROMOTIONS_URL_ENV, v),
            None => std::env::remove_var(FORGE_PROMOTIONS_URL_ENV),
        }

        result.expect("env-override list_loader_versions should succeed");
    }
}
