//! Fabric meta API HTTP client. (8.4-marker)
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
// to_mojang_shape — translate fabric-meta wire shape to Mojang on-disk shape.
//
// Phase 8.4 (round-4 BLOCKER closure GAP-LIBRARY-SHAPE-08): the launcher's
// Library struct deserialises ONLY the Mojang shape (downloads.artifact.{path,
// url, sha1?, size?}). Phase 6 used to write the verbatim fabric-meta wire
// shape (top-level url/sha1/size per library, no `downloads` block) into
// versions/{loader-id}/{loader-id}.json — silently dropping every library's
// hash + url on the launcher's deserialise step, leaving downloads.artifact
// == None, leaving the loader's own JAR off the classpath, leaving JVM unable
// to find KnotClient. This translator runs at the atomic_write boundary so
// the on-disk JSON is Mojang shape; the launcher reads ONE shape.
//
// FORBIDS (round-4 plan):
//   1. NEVER drop a library entry. count_in_translated == count_in_source.
//   2. NEVER add Optional flat fields to Library. Translation is the path.
// -----------------------------------------------------------------------

pub fn to_mojang_shape(raw_bytes: &[u8]) -> Result<Vec<u8>, LoaderError> {
    use serde_json::{Map, Value};

    let mut root: Value = serde_json::from_slice(raw_bytes).map_err(|e| {
        LoaderError::MetaParse {
            loader: "fabric",
            reason: format!("translate: parse profile json: {e}"),
        }
    })?;
    let obj = root.as_object_mut().ok_or_else(|| LoaderError::MetaParse {
        loader: "fabric",
        reason: "translate: top-level not an object".into(),
    })?;

    let Some(libs_value) = obj.get_mut("libraries") else {
        // No libraries field — pass through unchanged (Mojang accepts this).
        return serde_json::to_vec(&root).map_err(|e| LoaderError::MetaParse {
            loader: "fabric",
            reason: format!("translate: serialize: {e}"),
        });
    };
    let libs = libs_value.as_array_mut().ok_or_else(|| LoaderError::MetaParse {
        loader: "fabric",
        reason: "translate: libraries is not an array".into(),
    })?;

    for (idx, entry) in libs.iter_mut().enumerate() {
        let entry_obj = entry.as_object_mut().ok_or_else(|| LoaderError::MetaParse {
            loader: "fabric",
            reason: format!("translate: libraries[{idx}] is not an object"),
        })?;
        let name = entry_obj
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LoaderError::MetaParse {
                loader: "fabric",
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
            .unwrap_or_else(|| fabric_default_repo(&name).to_string());
        let url = crate::loader::maven::maven_download_url(&repo, &name)?;

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

        // Replace the entry: keep `name`, replace everything else with `downloads`.
        let mut new_entry = Map::new();
        new_entry.insert("name".into(), Value::String(name));
        new_entry.insert("downloads".into(), Value::Object(downloads));
        *entry = Value::Object(new_entry);
    }

    serde_json::to_vec(&root).map_err(|e| LoaderError::MetaParse {
        loader: "fabric",
        reason: format!("translate: serialize: {e}"),
    })
}

/// Repository fallback for Fabric library entries that omit `url`.
/// Quilt-coordinate libraries appear in some Fabric profiles when a future
/// Fabric release imports a Quilt artifact (rare but observed); fall back to
/// Quilt's repo in that case to keep the URL valid.
fn fabric_default_repo(name: &str) -> &'static str {
    if name.starts_with("org.quiltmc:") {
        "https://maven.quiltmc.org/"
    } else {
        "https://maven.fabricmc.net/"
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

    // ---- to_mojang_shape ----

    /// Real fabric-meta JSON sample BYTE-EQUIVALENT to a fragment of
    /// ~/.local/share/mineltui/versions/fabric-loader-0.19.2-1.20.4/
    ///   fabric-loader-0.19.2-1.20.4.json (round-4 plan FORBID #4: production
    /// shape only; do not synthesise; do not fatten).
    const REAL_FABRIC_META_BYTES: &[u8] = br#"{"id":"fabric-loader-0.19.2-1.20.4","inheritsFrom":"1.20.4","releaseTime":"2026-05-07T02:57:33+0000","time":"2026-05-07T02:57:33+0000","type":"release","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient","arguments":{"game":[],"jvm":["-DFabricMcEmu= net.minecraft.client.main.Main "]},"libraries":[{"name":"org.ow2.asm:asm:9.9","url":"https://maven.fabricmc.net/","md5":"6d1dd0482c03a6dc1807d9d004456021","sha1":"c29635c8a7afa03d74b33c1884df8abb2b3f3dcc","sha256":"03d99a74ad1ee5c71334ef67437f4ef4fe3488caa7c96d8645abc73c8e2017d4","sha512":"197a4fb3ecb34d05ac555c6a510e69affcb1e476f24c5e935ad513ecdabf74b45aa1b0e0b25dbe91224fc6db7959b2677ea5876ee49e7487265e2a29c560c21c","size":126122},{"name":"net.fabricmc:sponge-mixin:0.17.2+mixin.0.8.7","url":"https://maven.fabricmc.net/","md5":"4b6b96074976cc7aa096b9e569ca623e","sha1":"edf98d1d98229e46e36c61774ae2b54dcd852981","sha256":"95cef6aebd9da1559cf9c4624eafae2ce1242d0167e3587d5d62c488e45b6999","sha512":"89044dca9a63bd5f2ceec09bfcb5807f1b294026665294bae7a9a980da89bd86c6d441eb38c92c89ca0efe86884c0730dab348d27633ef1e3970ed9eb5c30a4e","size":1540039},{"name":"net.fabricmc:intermediary:1.20.4","url":"https://maven.fabricmc.net/"},{"name":"net.fabricmc:fabric-loader:0.19.2","url":"https://maven.fabricmc.net/"}]}"#;

    #[test]
    fn test_to_mojang_shape_translates_real_fabric_sample_into_mojang_library() {
        let translated = to_mojang_shape(REAL_FABRIC_META_BYTES).expect("translate ok");

        // Library count invariant (round-4 FORBID #1):
        // count_in_translated == count_in_source.
        let source_v: serde_json::Value = serde_json::from_slice(REAL_FABRIC_META_BYTES).unwrap();
        let translated_v: serde_json::Value = serde_json::from_slice(&translated).unwrap();
        let source_n = source_v["libraries"].as_array().unwrap().len();
        let translated_n = translated_v["libraries"].as_array().unwrap().len();
        assert_eq!(source_n, translated_n, "library count must be preserved");
        assert_eq!(source_n, 4);

        // The translated JSON must deserialise into the launcher's
        // Mojang VersionJson with downloads.artifact populated for every entry.
        use crate::mojang::types::VersionJson;
        let v: VersionJson = serde_json::from_slice(&translated)
            .expect("translated bytes parse as Mojang VersionJson");
        assert_eq!(v.id, "fabric-loader-0.19.2-1.20.4");
        assert_eq!(v.inherits_from.as_deref(), Some("1.20.4"));
        for lib in &v.libraries {
            let art = lib.downloads.artifact.as_ref().unwrap_or_else(|| {
                panic!("library {} must have downloads.artifact after translation", lib.name)
            });
            // Path matches the Maven-coord transform.
            let expected_path = crate::loader::maven::maven_coord_to_path(&lib.name).unwrap();
            assert_eq!(art.path, expected_path, "path for {}", lib.name);
            // URL ends with the path (a regular Maven repo URL).
            assert!(art.url.ends_with(&art.path),
                "url {} should end with {}", art.url, art.path);
        }
    }

    #[test]
    fn test_to_mojang_shape_preserves_sha1_and_size_when_present() {
        let translated = to_mojang_shape(REAL_FABRIC_META_BYTES).expect("translate ok");
        use crate::mojang::types::VersionJson;
        let v: VersionJson = serde_json::from_slice(&translated).unwrap();

        // org.ow2.asm:asm:9.9 has full hashes in fabric-meta.
        let asm = v.libraries.iter().find(|l| l.name == "org.ow2.asm:asm:9.9").unwrap();
        let art = asm.downloads.artifact.as_ref().unwrap();
        assert_eq!(art.sha1.as_deref(), Some("c29635c8a7afa03d74b33c1884df8abb2b3f3dcc"));
        assert_eq!(art.size, Some(126122));
    }

    #[test]
    fn test_to_mojang_shape_emits_none_sha1_when_source_has_no_sha1() {
        let translated = to_mojang_shape(REAL_FABRIC_META_BYTES).expect("translate ok");
        use crate::mojang::types::VersionJson;
        let v: VersionJson = serde_json::from_slice(&translated).unwrap();

        // intermediary has only name + url (no sha1) in fabric-meta.
        let inter = v.libraries.iter().find(|l| l.name == "net.fabricmc:intermediary:1.20.4").unwrap();
        let art = inter.downloads.artifact.as_ref().unwrap();
        assert!(art.sha1.is_none(), "intermediary has no upstream sha1");
        assert!(art.size.is_none(), "intermediary has no upstream size");
        // path + url are still populated.
        assert_eq!(art.path, "net/fabricmc/intermediary/1.20.4/intermediary-1.20.4.jar");
        assert!(art.url.starts_with("https://maven.fabricmc.net/"));
    }

    #[test]
    fn test_to_mojang_shape_preserves_top_level_fields() {
        let translated = to_mojang_shape(REAL_FABRIC_META_BYTES).expect("translate ok");
        let v: serde_json::Value = serde_json::from_slice(&translated).unwrap();
        // Top-level fields preserved verbatim.
        assert_eq!(v["id"], "fabric-loader-0.19.2-1.20.4");
        assert_eq!(v["inheritsFrom"], "1.20.4");
        assert_eq!(v["mainClass"], "net.fabricmc.loader.impl.launch.knot.KnotClient");
        assert_eq!(v["type"], "release");
        // arguments.jvm preserved as-is.
        assert_eq!(v["arguments"]["jvm"][0], "-DFabricMcEmu= net.minecraft.client.main.Main ");
    }
}
