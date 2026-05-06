//! NeoForge meta API HTTP client.
//!
//! Endpoints (verified 2026-05-06):
//!   GET https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml
//!   GET https://maven.neoforged.net/releases/net/neoforged/neoforge/{v}/neoforge-{v}-installer.jar
//!
//! Override the base URL for tests via `MINELTUI_NEOFORGE_MAVEN_BASE_URL`.
//!
//! Pitfall 8 of 07-RESEARCH.md: NeoForge versions can have 4 segments
//! (`26.1.2.41-beta`); we filter via string-prefix only, never `split('.')`.
//!
//! MC → NeoForge prefix mapping (derived from observed releases):
//!   "1.20.1" → "20.1."  (NeoForge forked here; numbering "20.1.<build>")
//!   "1.21"   → "21."    (prefix matches "21.0.x" and any future "21.x.x")
//!   "1.21.4" → "21.4."
//!   "2.0"    → None     (NeoForge only exists for the 1.x release line)

use std::time::Duration;

use crate::loader::error::LoaderError;
use crate::loader::maven_metadata;
use crate::loader::types::LoaderVersionEntry;

pub const DEFAULT_NEOFORGE_MAVEN_BASE: &str =
    "https://maven.neoforged.net/releases/net/neoforged/neoforge";
pub const NEOFORGE_MAVEN_BASE_URL_ENV: &str = "MINELTUI_NEOFORGE_MAVEN_BASE_URL";

// -----------------------------------------------------------------------
// Pure helpers
// -----------------------------------------------------------------------

/// Convert an MC version string to the NeoForge version-number prefix.
///
/// NeoForge drops the leading `"1."` from the MC version and appends `"."`:
/// - `"1.21.4"` → `Some("21.4.")`
/// - `"1.21"`   → `Some("21.")`   matches `21.0.x`, `21.1.x`, …
/// - `"1.20.1"` → `Some("20.1.")`
/// - `"2.0"`    → `None`          NeoForge only exists for the `1.x` line
///
/// Pitfall 8: never split by `"."` beyond the first strip — NeoForge betas
/// have 4 segments (e.g., `"21.4.114-beta"` or `"26.1.2.41-beta"`); a plain
/// prefix match handles them correctly.
pub fn mc_to_neoforge_prefix(mc_version: &str) -> Option<String> {
    let stripped = mc_version.strip_prefix("1.")?; // "21.4"
    Some(format!("{stripped}.")) // "21.4."
}

/// Stability heuristic — string-only check (Pitfall 8: never split by `.`).
///
/// A NeoForge version is considered stable when its version string contains
/// none of the pre-release markers: `beta`, `rc`, `pre`, `alpha`
/// (case-insensitive). This handles both `"21.4.114-beta"` (3-segment) and
/// `"26.1.2.41-beta"` (4-segment) without splitting on `.`.
pub fn is_neoforge_stable(version: &str) -> bool {
    let lower = version.to_ascii_lowercase();
    !lower.contains("beta")
        && !lower.contains("rc")
        && !lower.contains("pre")
        && !lower.contains("alpha")
}

// -----------------------------------------------------------------------
// Client
// -----------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct NeoForgeMetaClient {
    http: reqwest::Client,
    base_url: String,
}

impl NeoForgeMetaClient {
    pub fn new() -> Result<Self, LoaderError> {
        let base_url = std::env::var(NEOFORGE_MAVEN_BASE_URL_ENV)
            .unwrap_or_else(|_| DEFAULT_NEOFORGE_MAVEN_BASE.to_owned());
        Self::new_with_base_url(base_url)
    }

    pub fn new_with_base_url(base_url: impl Into<String>) -> Result<Self, LoaderError> {
        let http = reqwest::Client::builder()
            .user_agent(crate::mojang::client::USER_AGENT)
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| LoaderError::MavenMetadataFetch {
                reason: format!("reqwest build (neoforge): {e}"),
            })?;
        Ok(Self { http, base_url: base_url.into() })
    }

    /// List NeoForge loader versions compatible with `mc_version`.
    ///
    /// Fetches `maven-metadata.xml`, maps `mc_version` to a NeoForge version
    /// prefix via `mc_to_neoforge_prefix`, and filters entries by that prefix.
    /// Stability is derived from the version string via `is_neoforge_stable`.
    /// Returns empty `Vec` for unknown or non-`1.x` MC versions (D-05).
    #[tracing::instrument(skip_all, fields(mc_version = %mc_version))]
    pub async fn list_loader_versions(
        &self,
        mc_version: &str,
    ) -> Result<Vec<LoaderVersionEntry>, LoaderError> {
        // Derive prefix; return empty for non-1.x MC (no NeoForge exists there)
        let prefix = match mc_to_neoforge_prefix(mc_version) {
            Some(p) => p,
            None => return Ok(Vec::new()),
        };

        let url = format!("{}/maven-metadata.xml", self.base_url.trim_end_matches('/'));
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

        let all = maven_metadata::extract_versions(&xml);
        Ok(all
            .into_iter()
            .filter(|v| v.starts_with(&prefix))
            .map(|v| LoaderVersionEntry {
                stable: is_neoforge_stable(&v),
                version: v,
                build: None,
            })
            .collect())
    }

    /// Return the installer JAR URL for a given `neoforge_version`.
    ///
    /// Template (verified 2026-05-06):
    /// `{base}/{v}/neoforge-{v}-installer.jar`
    pub fn installer_url(&self, neoforge_version: &str) -> String {
        format!(
            "{}/{neoforge_version}/neoforge-{neoforge_version}-installer.jar",
            self.base_url.trim_end_matches('/')
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

    fn make_client(server: &MockServer) -> NeoForgeMetaClient {
        NeoForgeMetaClient::new_with_base_url(server.base_url()).expect("client build")
    }

    /// A representative maven-metadata.xml fixture covering multiple MC prefixes,
    /// 3-segment stable, 3-segment beta, and the Pitfall 8 4-segment beta.
    const NEOFORGE_META_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>net.neoforged</groupId>
  <artifactId>neoforge</artifactId>
  <versioning>
    <versions>
      <version>20.1.4</version>
      <version>21.0.166</version>
      <version>21.1.228</version>
      <version>21.4.114</version>
      <version>21.4.114-beta</version>
      <version>26.1.2.41-beta</version>
    </versions>
  </versioning>
</metadata>"#;

    // ---- mc_to_neoforge_prefix ----

    #[test]
    fn test_mc_to_neoforge_prefix_3_segment() {
        assert_eq!(mc_to_neoforge_prefix("1.21.4"), Some("21.4.".to_string()));
        assert_eq!(mc_to_neoforge_prefix("1.20.1"), Some("20.1.".to_string()));
        assert_eq!(mc_to_neoforge_prefix("1.21.1"), Some("21.1.".to_string()));
    }

    #[test]
    fn test_mc_to_neoforge_prefix_2_segment() {
        // 2-segment MC "1.21" → "21." (matches "21.0.x", "21.1.x", …)
        assert_eq!(mc_to_neoforge_prefix("1.21"), Some("21.".to_string()));
        assert_eq!(mc_to_neoforge_prefix("1.20"), Some("20.".to_string()));
    }

    #[test]
    fn test_mc_to_neoforge_prefix_rejects_non_1x() {
        assert_eq!(mc_to_neoforge_prefix("2.0"), None, "NeoForge doesn't exist for 2.x");
        assert_eq!(mc_to_neoforge_prefix(""), None, "empty string must return None");
        assert_eq!(mc_to_neoforge_prefix("0.9"), None, "pre-1.x MC must return None");
    }

    // ---- is_neoforge_stable ----

    #[test]
    fn test_is_neoforge_stable_classifier() {
        // Stable versions
        assert!(is_neoforge_stable("21.4.114"), "plain version => stable");
        assert!(is_neoforge_stable("21.1.228"), "21.1 stable => stable");
        assert!(is_neoforge_stable("20.1.4"), "20.1 stable => stable");

        // Pre-release markers
        assert!(!is_neoforge_stable("21.4.114-beta"), "beta => unstable");
        // Pitfall 8: 4-segment beta must also be caught (never split by '.')
        assert!(!is_neoforge_stable("26.1.2.41-beta"), "4-segment beta => unstable");
        assert!(!is_neoforge_stable("21.0.0-pre.1"), "pre => unstable");
        assert!(!is_neoforge_stable("21.1.0-rc.1"), "rc => unstable");
        assert!(!is_neoforge_stable("21.0.0-alpha.15"), "alpha => unstable");
        // Case-insensitive check
        assert!(!is_neoforge_stable("21.4.0-BETA.7"), "BETA (upper) => unstable");
    }

    // ---- list_loader_versions ----

    #[tokio::test]
    async fn test_list_loader_versions_filters_by_mc_prefix() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/maven-metadata.xml");
            then.status(200)
                .header("content-type", "application/xml")
                .body(NEOFORGE_META_XML);
        });

        let client = make_client(&server);
        // Query for 1.21.4 → prefix "21.4." → should match 21.4.114 and 21.4.114-beta
        let versions = client.list_loader_versions("1.21.4").await.expect("ok");

        assert_eq!(versions.len(), 2, "expected exactly two 1.21.4 entries: {versions:?}");

        let stable_entry = versions.iter().find(|v| v.version == "21.4.114").expect("21.4.114");
        let beta_entry = versions.iter().find(|v| v.version == "21.4.114-beta").expect("21.4.114-beta");

        assert!(stable_entry.stable, "21.4.114 has no pre-release marker => stable");
        assert!(!beta_entry.stable, "21.4.114-beta => unstable");
        assert_eq!(stable_entry.build, None);
        assert_eq!(beta_entry.build, None);

        // Ensure non-matching entries are excluded
        assert!(
            versions.iter().all(|v| !v.version.starts_with("20.")),
            "20.x entries must not appear"
        );
        assert!(
            versions.iter().all(|v| !v.version.starts_with("21.0.")),
            "21.0.x entries must not appear"
        );
        assert!(
            versions.iter().all(|v| !v.version.starts_with("21.1.")),
            "21.1.x entries must not appear"
        );
    }

    #[tokio::test]
    async fn test_list_loader_versions_unknown_mc_returns_empty() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/maven-metadata.xml");
            then.status(200)
                .header("content-type", "application/xml")
                .body(NEOFORGE_META_XML);
        });

        let client = make_client(&server);
        // "1.99.0" is unknown — prefix "99.0." — no fixture entries match
        let versions = client.list_loader_versions("1.99.0").await.expect("ok");
        assert!(versions.is_empty(), "unknown MC version => empty Vec (not an error): {versions:?}");
    }

    #[tokio::test]
    async fn test_list_loader_versions_non_1x_mc_returns_empty_without_http() {
        // For non-1.x MC, mc_to_neoforge_prefix returns None → must return empty
        // without making any HTTP call (no server mock set up deliberately)
        let client = NeoForgeMetaClient::new_with_base_url("http://127.0.0.1:1")
            .expect("client build");
        let versions = client.list_loader_versions("2.0").await.expect("ok");
        assert!(versions.is_empty(), "non-1.x MC => empty Vec (no HTTP): {versions:?}");
    }

    #[tokio::test]
    async fn test_installer_url_format() {
        // Pure-function test — no HTTP mock needed
        let client = NeoForgeMetaClient::new_with_base_url(
            "https://maven.neoforged.net/releases/net/neoforged/neoforge",
        )
        .expect("client build");
        let url = client.installer_url("21.1.228");
        assert!(
            url.ends_with("/21.1.228/neoforge-21.1.228-installer.jar"),
            "unexpected URL: {url}"
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
        let r = client.list_loader_versions("1.21.4").await;
        assert!(
            matches!(r, Err(LoaderError::MavenMetadataFetch { .. })),
            "expected MavenMetadataFetch, got: {r:?}"
        );
    }

    #[tokio::test]
    async fn test_env_override_base_url() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/maven-metadata.xml");
            then.status(200)
                .header("content-type", "application/xml")
                .body(NEOFORGE_META_XML);
        });

        let prior = std::env::var(NEOFORGE_MAVEN_BASE_URL_ENV).ok();
        std::env::set_var(NEOFORGE_MAVEN_BASE_URL_ENV, server.base_url());

        let client = NeoForgeMetaClient::new().expect("client from env");
        let result = client.list_loader_versions("1.21.4").await;

        match prior {
            Some(v) => std::env::set_var(NEOFORGE_MAVEN_BASE_URL_ENV, v),
            None => std::env::remove_var(NEOFORGE_MAVEN_BASE_URL_ENV),
        }

        result.expect("env-override list_loader_versions should succeed");
    }
}
