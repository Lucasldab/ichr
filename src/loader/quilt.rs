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
// to_mojang_shape — translate quilt-meta wire shape to Mojang on-disk shape.
//
// Phase 8.4 (round-4 BLOCKER closure GAP-LIBRARY-SHAPE-08). Quilt-meta
// library entries are uniformly {name, url} (no upstream hashes — verified
// by test_fetch_profile_no_hashes_on_libraries / Pattern 6). The translated
// entries emit downloads.artifact.sha1 = None and .size = None; the
// launcher's LibraryArtifact has those fields as Option<_> per 8.4.
//
// Identical to fabric::to_mojang_shape modulo the default repo (Quilt
// libraries default to the Quilt Maven; some pull from Fabric Maven).
// -----------------------------------------------------------------------

pub fn to_mojang_shape(raw_bytes: &[u8]) -> Result<Vec<u8>, LoaderError> {
    use serde_json::{Map, Value};

    let mut root: Value = serde_json::from_slice(raw_bytes).map_err(|e| {
        LoaderError::MetaParse {
            loader: "quilt",
            reason: format!("translate: parse profile json: {e}"),
        }
    })?;
    let obj = root.as_object_mut().ok_or_else(|| LoaderError::MetaParse {
        loader: "quilt",
        reason: "translate: top-level not an object".into(),
    })?;
    let Some(libs_value) = obj.get_mut("libraries") else {
        return serde_json::to_vec(&root).map_err(|e| LoaderError::MetaParse {
            loader: "quilt",
            reason: format!("translate: serialize: {e}"),
        });
    };
    let libs = libs_value.as_array_mut().ok_or_else(|| LoaderError::MetaParse {
        loader: "quilt",
        reason: "translate: libraries is not an array".into(),
    })?;

    for (idx, entry) in libs.iter_mut().enumerate() {
        let entry_obj = entry.as_object_mut().ok_or_else(|| LoaderError::MetaParse {
            loader: "quilt",
            reason: format!("translate: libraries[{idx}] is not an object"),
        })?;
        let name = entry_obj
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LoaderError::MetaParse {
                loader: "quilt",
                reason: format!("translate: libraries[{idx}] missing `name`"),
            })?
            .to_string();

        let path = crate::loader::maven::maven_coord_to_path(&name)?;
        let entry_url = entry_obj
            .get("url")
            .and_then(|v| v.as_str())
            .map(String::from);
        let repo = entry_url
            .clone()
            .unwrap_or_else(|| quilt_default_repo(&name).to_string());
        let url = crate::loader::maven::maven_download_url(&repo, &name)?;

        // Quilt-meta does not carry sha1/size — but tolerate them anyway
        // (forward-compat if Quilt ever adds them).
        let sha1 = entry_obj.get("sha1").and_then(|v| v.as_str()).map(String::from);
        let size = entry_obj.get("size").and_then(|v| v.as_u64());

        let mut artifact = Map::new();
        artifact.insert("path".into(), Value::String(path));
        artifact.insert("url".into(), Value::String(url));
        if let Some(s) = sha1 {
            artifact.insert("sha1".into(), Value::String(s));
        }
        if let Some(sz) = size {
            artifact.insert("size".into(), Value::Number(sz.into()));
        }
        let mut downloads = Map::new();
        downloads.insert("artifact".into(), Value::Object(artifact));

        let mut new_entry = Map::new();
        new_entry.insert("name".into(), Value::String(name));
        new_entry.insert("downloads".into(), Value::Object(downloads));
        *entry = Value::Object(new_entry);
    }

    serde_json::to_vec(&root).map_err(|e| LoaderError::MetaParse {
        loader: "quilt",
        reason: format!("translate: serialize: {e}"),
    })
}

/// Repository fallback for Quilt library entries that omit `url`.
/// Most Quilt profiles do declare per-entry url; fallback covers the
/// edge case where they don't (defensive).
fn quilt_default_repo(name: &str) -> &'static str {
    if name.starts_with("net.fabricmc:") {
        "https://maven.fabricmc.net/"
    } else {
        "https://maven.quiltmc.org/repository/release/"
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

    // ---- to_mojang_shape ----

    /// Real quilt-meta JSON sample BYTE-EQUIVALENT to a fragment of
    /// ~/.local/share/mineltui/versions/quilt-loader-0.30.0-beta.7-1.20.4/
    ///   quilt-loader-0.30.0-beta.7-1.20.4.json (round-4 FORBID #4).
    const REAL_QUILT_META_BYTES: &[u8] = br#"{"id":"quilt-loader-0.30.0-beta.7-1.20.4","inheritsFrom":"1.20.4","type":"release","mainClass":"org.quiltmc.loader.impl.launch.knot.KnotClient","arguments":{"game":[]},"libraries":[{"name":"net.fabricmc:sponge-mixin:0.17.0+mixin.0.8.7","url":"https://maven.fabricmc.net/"},{"name":"org.quiltmc:quilt-json5:1.0.4+final","url":"https://maven.quiltmc.org/repository/release/"},{"name":"org.ow2.asm:asm:9.9","url":"https://maven.fabricmc.net/"},{"name":"org.quiltmc:quilt-loader:0.30.0-beta.7","url":"https://maven.quiltmc.org/repository/release/"}],"releaseTime":"2023-12-07T12:56:20+00:00","time":"2026-04-21T06:25:58+00:00"}"#;

    #[test]
    fn test_to_mojang_shape_translates_real_quilt_sample_with_no_hashes() {
        let translated = to_mojang_shape(REAL_QUILT_META_BYTES).expect("translate ok");

        let source_v: serde_json::Value = serde_json::from_slice(REAL_QUILT_META_BYTES).unwrap();
        let translated_v: serde_json::Value = serde_json::from_slice(&translated).unwrap();
        assert_eq!(
            source_v["libraries"].as_array().unwrap().len(),
            translated_v["libraries"].as_array().unwrap().len(),
            "library count preserved (FORBID #1)"
        );

        use crate::mojang::types::VersionJson;
        let v: VersionJson = serde_json::from_slice(&translated)
            .expect("translated quilt bytes parse as Mojang VersionJson");
        assert_eq!(v.id, "quilt-loader-0.30.0-beta.7-1.20.4");
        for lib in &v.libraries {
            let art = lib.downloads.artifact.as_ref()
                .unwrap_or_else(|| panic!("library {} must have artifact", lib.name));
            // Quilt has NO upstream hashes — sha1 + size MUST be None.
            assert!(art.sha1.is_none(), "quilt {} sha1 must be None", lib.name);
            assert!(art.size.is_none(), "quilt {} size must be None", lib.name);
            // path + url are still populated.
            let expected_path = crate::loader::maven::maven_coord_to_path(&lib.name).unwrap();
            assert_eq!(art.path, expected_path);
            assert!(art.url.ends_with(&art.path));
        }
    }

    #[test]
    fn test_to_mojang_shape_quilt_url_dispatches_repo_by_coordinate() {
        let translated = to_mojang_shape(REAL_QUILT_META_BYTES).expect("translate ok");
        use crate::mojang::types::VersionJson;
        let v: VersionJson = serde_json::from_slice(&translated).unwrap();

        // sponge-mixin pulls from Fabric Maven (per-entry url override).
        let mixin = v.libraries.iter()
            .find(|l| l.name == "net.fabricmc:sponge-mixin:0.17.0+mixin.0.8.7").unwrap();
        assert!(
            mixin.downloads.artifact.as_ref().unwrap().url
                .starts_with("https://maven.fabricmc.net/"),
            "sponge-mixin url must come from per-entry override"
        );

        // quilt-loader pulls from Quilt Maven (per-entry url override).
        let loader = v.libraries.iter()
            .find(|l| l.name == "org.quiltmc:quilt-loader:0.30.0-beta.7").unwrap();
        assert!(
            loader.downloads.artifact.as_ref().unwrap().url
                .starts_with("https://maven.quiltmc.org/"),
            "quilt-loader url must come from per-entry override"
        );
    }

    #[test]
    fn test_to_mojang_shape_quilt_top_level_fields_preserved() {
        let translated = to_mojang_shape(REAL_QUILT_META_BYTES).expect("translate ok");
        let v: serde_json::Value = serde_json::from_slice(&translated).unwrap();
        assert_eq!(v["id"], "quilt-loader-0.30.0-beta.7-1.20.4");
        assert_eq!(v["inheritsFrom"], "1.20.4");
        assert_eq!(v["mainClass"], "org.quiltmc.loader.impl.launch.knot.KnotClient");
        // arguments.game empty array preserved.
        assert!(v["arguments"]["game"].is_array());
    }
}
