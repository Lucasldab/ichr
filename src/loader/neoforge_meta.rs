//! NeoForge meta API HTTP client.
//!
//! Endpoints (verified 2026-05-07 -- see 07-RESEARCH.md §Errata, GAP-7-B):
//!   GET https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge
//!     → JSON: {"isSnapshot": bool, "versions": [String, ...]}
//!   GET https://maven.neoforged.net/releases/net/neoforged/neoforge/{v}/neoforge-{v}-installer.jar
//!     → Installer JAR (different base -- the maven-files repo, not the JSON-API)
//!
//! Override base URLs for tests via:
//!   - `ICHR_NEOFORGE_MAVEN_BASE_URL`        (JSON-API base -- list_loader_versions)
//!   - `ICHR_NEOFORGE_MAVEN_FILES_BASE_URL`  (maven-files base -- installer_url)
//!
//! Why dual bases: the version listing endpoint and the installer JAR live at
//! different roots on `maven.neoforged.net`. Phase 7 originally assumed they
//! shared a base (the maven-files path); UAT 2026-05-07 caught the 404 and
//! Phase 7.1-01 split them.
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
use crate::loader::types::LoaderVersionEntry;

/// JSON-API base URL -- used by `list_loader_versions`.
pub const DEFAULT_NEOFORGE_MAVEN_BASE: &str =
    "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";
pub const NEOFORGE_MAVEN_BASE_URL_ENV: &str = "ICHR_NEOFORGE_MAVEN_BASE_URL";

/// Maven-FILES base URL -- used by `installer_url` only. Distinct from the
/// JSON-API base above because the version listing endpoint and the installer
/// JAR live in different parts of the `maven.neoforged.net` layout.
pub const DEFAULT_NEOFORGE_MAVEN_FILES_BASE: &str =
    "https://maven.neoforged.net/releases/net/neoforged/neoforge";
pub const NEOFORGE_MAVEN_FILES_BASE_URL_ENV: &str = "ICHR_NEOFORGE_MAVEN_FILES_BASE_URL";

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
/// Pitfall 8: never split by `"."` beyond the first strip -- NeoForge betas
/// have 4 segments (e.g., `"21.4.114-beta"` or `"26.1.2.41-beta"`); a plain
/// prefix match handles them correctly.
pub fn mc_to_neoforge_prefix(mc_version: &str) -> Option<String> {
    let stripped = mc_version.strip_prefix("1.")?; // "21.4"
    Some(format!("{stripped}.")) // "21.4."
}

/// Stability heuristic -- string-only check (Pitfall 8: never split by `.`).
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
    /// JSON-API base -- GET `{json_api_base}` returns the version list.
    json_api_base: String,
    /// Maven-files base -- used to construct installer JAR URLs.
    maven_files_base: String,
}

impl NeoForgeMetaClient {
    pub fn new() -> Result<Self, LoaderError> {
        let json_api_base = std::env::var(NEOFORGE_MAVEN_BASE_URL_ENV)
            .unwrap_or_else(|_| DEFAULT_NEOFORGE_MAVEN_BASE.to_owned());
        let maven_files_base = std::env::var(NEOFORGE_MAVEN_FILES_BASE_URL_ENV)
            .unwrap_or_else(|_| DEFAULT_NEOFORGE_MAVEN_FILES_BASE.to_owned());
        Self::new_with_base_urls(json_api_base, maven_files_base)
    }

    /// Test/back-compat constructor: sets BOTH bases to the supplied URL.
    /// httpmock test servers expose the entire mock surface under one root,
    /// so a single base works for tests that mock both endpoints; production
    /// uses different roots via `new_with_base_urls`.
    pub fn new_with_base_url(base_url: impl Into<String>) -> Result<Self, LoaderError> {
        let s = base_url.into();
        Self::new_with_base_urls(s.clone(), s)
    }

    pub fn new_with_base_urls(
        json_api_base: impl Into<String>,
        maven_files_base: impl Into<String>,
    ) -> Result<Self, LoaderError> {
        let http = reqwest::Client::builder()
            .user_agent(crate::mojang::client::USER_AGENT)
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| LoaderError::MavenMetadataFetch {
                reason: format!("reqwest build (neoforge): {e}"),
            })?;
        Ok(Self {
            http,
            json_api_base: json_api_base.into(),
            maven_files_base: maven_files_base.into(),
        })
    }

    /// List NeoForge loader versions compatible with `mc_version`.
    ///
    /// GETs the NeoForge JSON-API endpoint at `json_api_base`, maps `mc_version`
    /// to a NeoForge version prefix via `mc_to_neoforge_prefix`, and filters
    /// entries by that prefix. Stability is derived from the version string
    /// via `is_neoforge_stable`. Returns empty `Vec` for unknown or non-`1.x`
    /// MC versions (D-05 graceful state).
    ///
    /// The `isSnapshot` field at the JSON document root is currently always
    /// `false` for the `releases` namespace; the parser reads it (so serde
    /// catches a future schema change) but per-version stability stays derived
    /// via `is_neoforge_stable` (Pitfall 8).
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

        // The JSON-API base IS the endpoint URL -- no path suffix. Trim a
        // trailing `/` so a base configured as `.../neoforge/` (with slash)
        // and `.../neoforge` (without) both work.
        let url = self.json_api_base.trim_end_matches('/').to_string();

        // Private DTO for transport -- not part of the module's public API.
        // `isSnapshot` is camelCase in the upstream payload; map explicitly via
        // `#[serde(rename)]` (no `rename_all` magic -- see Phase 7.1-01 plan
        // "Explicit Forbids" §2). Without `#[serde(default)]` a missing field
        // surfaces as a parse error -- that's the contract: `isSnapshot` is
        // REQUIRED by upstream and a future schema drift MUST fail loudly.
        #[derive(serde::Deserialize)]
        struct VersionsResponse {
            #[serde(rename = "isSnapshot")]
            #[allow(dead_code)]
            is_snapshot: bool,
            versions: Vec<String>,
        }

        let body: VersionsResponse = self
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
            .json::<VersionsResponse>()
            .await
            .map_err(|e| LoaderError::MavenMetadataFetch {
                reason: format!("parse {url}: {e}"),
            })?;

        Ok(body
            .versions
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
    /// `{maven_files_base}/{v}/neoforge-{v}-installer.jar`
    pub fn installer_url(&self, neoforge_version: &str) -> String {
        format!(
            "{}/{neoforge_version}/neoforge-{neoforge_version}-installer.jar",
            self.maven_files_base.trim_end_matches('/')
        )
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    //! Tests for `NeoForgeMetaClient`.
    //!
    //! The JSON fixture `tests/fixtures/neoforge_meta_versions.json` is a
    //! BYTE-EQUIVALENT capture of the production response -- DO NOT hand-edit.
    //! If the upstream JSON shape changes, RECAPTURE via:
    //!
    //! ```bash
    //! curl -s 'https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge' \
    //!   | python3 -c 'import json,sys; d=json.load(sys.stdin); v=d["versions"]; \
    //!     sel=[x for x in v if x.startswith(("20.2.","21.0.","21.1.","21.4.","26.1."))][:18]; \
    //!     print(json.dumps({"isSnapshot": d["isSnapshot"], "versions": sel}, separators=(",",":")), end="")' \
    //!   > tests/fixtures/neoforge_meta_versions.json
    //! ```

    use super::*;
    use httpmock::Method::GET;
    use httpmock::MockServer;

    /// Byte-equivalent capture of the production NeoForge JSON-API response.
    /// Captured 2026-05-07. Trimmed to 18 entries covering every prefix Phase 7
    /// uses (20.2.x, 21.0.x, 21.1.x, 21.4.x) plus the 4-segment beta family
    /// (26.1.x.x including `26.1.2.41-beta` -- Pitfall 8 anchor).
    const NEOFORGE_META_JSON: &str =
        include_str!("../../tests/fixtures/neoforge_meta_versions.json");

    fn make_client(server: &MockServer) -> NeoForgeMetaClient {
        NeoForgeMetaClient::new_with_base_url(server.base_url()).expect("client build")
    }

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
        assert_eq!(
            mc_to_neoforge_prefix("2.0"),
            None,
            "NeoForge doesn't exist for 2.x"
        );
        assert_eq!(
            mc_to_neoforge_prefix(""),
            None,
            "empty string must return None"
        );
        assert_eq!(
            mc_to_neoforge_prefix("0.9"),
            None,
            "pre-1.x MC must return None"
        );
    }

    // ---- is_neoforge_stable ----

    #[test]
    fn test_is_neoforge_stable_classifier() {
        // Stable versions
        assert!(is_neoforge_stable("21.4.121"), "plain version => stable");
        assert!(is_neoforge_stable("21.1.228"), "21.1 stable => stable");
        assert!(is_neoforge_stable("20.1.4"), "20.1 stable => stable");

        // Pre-release markers
        assert!(!is_neoforge_stable("21.4.114-beta"), "beta => unstable");
        // Pitfall 8: 4-segment beta must also be caught (never split by '.')
        assert!(
            !is_neoforge_stable("26.1.2.41-beta"),
            "4-segment beta => unstable"
        );
        assert!(!is_neoforge_stable("21.0.0-pre.1"), "pre => unstable");
        assert!(!is_neoforge_stable("21.1.0-rc.1"), "rc => unstable");
        assert!(!is_neoforge_stable("21.0.0-alpha.15"), "alpha => unstable");
        // Case-insensitive check
        assert!(
            !is_neoforge_stable("21.4.0-BETA.7"),
            "BETA (upper) => unstable"
        );
    }

    // ---- list_loader_versions ----

    #[tokio::test]
    async fn test_list_loader_versions_filters_by_mc_prefix() {
        let server = MockServer::start();
        // The JSON-API base IS the endpoint URL (no path suffix). httpmock
        // serves the body for any GET request -- production uses one URL, so
        // a path-agnostic mock matches the production request.
        server.mock(|when, then| {
            when.method(GET);
            then.status(200)
                .header("content-type", "application/json")
                .body(NEOFORGE_META_JSON);
        });

        let client = make_client(&server);
        // Query for 1.21.4 → prefix "21.4." → fixture has 5 entries:
        // 21.4.0-beta, 21.4.114-beta, 21.4.121, 21.4.122, 21.4.123
        let versions = client.list_loader_versions("1.21.4").await.expect("ok");

        assert!(
            versions.len() >= 2,
            "expected ≥2 1.21.4 entries (got {}): {versions:?}",
            versions.len()
        );

        let stable_entry = versions
            .iter()
            .find(|v| v.version == "21.4.121")
            .expect("21.4.121 present in fixture");
        let beta_entry = versions
            .iter()
            .find(|v| v.version == "21.4.114-beta")
            .expect("21.4.114-beta present in fixture");

        assert!(
            stable_entry.stable,
            "21.4.121 has no pre-release marker => stable"
        );
        assert!(!beta_entry.stable, "21.4.114-beta => unstable");
        assert_eq!(stable_entry.build, None);
        assert_eq!(beta_entry.build, None);

        // Pitfall 8 anchor: 1.21.4-prefix query MUST NOT pick up 26.x family
        assert!(
            versions.iter().all(|v| !v.version.starts_with("26.")),
            "26.x entries must not appear: {versions:?}"
        );
        // Ensure non-matching 21.x entries are excluded
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
            when.method(GET);
            then.status(200)
                .header("content-type", "application/json")
                .body(NEOFORGE_META_JSON);
        });

        let client = make_client(&server);
        // "1.99.0" is unknown -- prefix "99.0." -- no fixture entries match
        let versions = client.list_loader_versions("1.99.0").await.expect("ok");
        assert!(
            versions.is_empty(),
            "unknown MC version => empty Vec (not an error): {versions:?}"
        );
    }

    #[tokio::test]
    async fn test_list_loader_versions_non_1x_mc_returns_empty_without_http() {
        // For non-1.x MC, mc_to_neoforge_prefix returns None → must return empty
        // without making any HTTP call (no server mock set up deliberately)
        let client =
            NeoForgeMetaClient::new_with_base_url("http://127.0.0.1:1").expect("client build");
        let versions = client.list_loader_versions("2.0").await.expect("ok");
        assert!(
            versions.is_empty(),
            "non-1.x MC => empty Vec (no HTTP): {versions:?}"
        );
    }

    #[tokio::test]
    async fn test_installer_url_format() {
        // Pure-function test -- no HTTP mock needed. installer_url builds from
        // the maven-files base, which `new_with_base_url` sets to the same
        // value as the JSON-API base for back-compat.
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
    async fn test_installer_url_uses_maven_files_base_not_json_api_base() {
        // Verify the dual-base contract: `new_with_base_urls` sets distinct
        // bases; installer_url uses the maven-files one only.
        let client = NeoForgeMetaClient::new_with_base_urls(
            "https://json-api.example.com/api/maven/versions/releases/net/neoforged/neoforge",
            "https://files.example.com/releases/net/neoforged/neoforge",
        )
        .expect("client build");
        let url = client.installer_url("21.4.121");
        assert_eq!(
            url,
            "https://files.example.com/releases/net/neoforged/neoforge/\
21.4.121/neoforge-21.4.121-installer.jar"
        );
        assert!(
            !url.contains("json-api.example.com"),
            "installer_url must NOT use the JSON-API base: {url}"
        );
    }

    #[tokio::test]
    async fn test_http_500_returns_maven_metadata_fetch() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET);
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
            when.method(GET);
            then.status(200)
                .header("content-type", "application/json")
                .body(NEOFORGE_META_JSON);
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

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_list_loader_versions_parses_camelcase_isSnapshot_field() {
        // Schema canary: pins the camelCase contract. If upstream renames
        // `isSnapshot` to `is_snapshot` or `snapshot`, serde returns parse
        // error and this test breaks at the `.expect("ok")` below.
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET);
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"isSnapshot":false,"versions":["21.4.999"]}"#);
        });
        let client = make_client(&server);
        let v = client.list_loader_versions("1.21.4").await.expect("ok");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].version, "21.4.999");
        assert!(v[0].stable, "21.4.999 has no pre-release marker => stable");
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_list_loader_versions_rejects_missing_isSnapshot_field() {
        // Inverse canary: a server returning a body that OMITS `isSnapshot`
        // MUST surface as a parse error -- we want to know if upstream stops
        // sending the field, not silently pretend it's `false`. The transport
        // DTO has NO `#[serde(default)]` on `is_snapshot` for exactly this
        // reason: missing field = loud failure.
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET);
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"versions":["21.4.999"]}"#); // missing isSnapshot
        });
        let client = make_client(&server);
        let r = client.list_loader_versions("1.21.4").await;
        assert!(
            matches!(r, Err(LoaderError::MavenMetadataFetch { .. })),
            "missing isSnapshot must surface as MavenMetadataFetch parse error: {r:?}"
        );
    }
}
