//! CurseForge installer helpers.
//!
//! Phase 9 plan 09-04: implements `resolve_download_url` — the load-bearing
//! function for MOD-04 success criterion 3. Plan 09-05 (CurseForgeService)
//! calls this at the top of `install_mod_into_instance` and either obtains
//! the URL or surfaces `CurseForgeError::FileNotDownloadable` before
//! attempting any download.
//!
//! Per 09-RESEARCH.md §"downloadUrl null UX" lines 247-289 — the three
//! resolution cases:
//!
//!   1. Inline URL present (most common, one-fewer round-trip)
//!   2. Inline null + dedicated /download-url endpoint returns Some(url)
//!      (transient null — file is downloadable, the file response is just stale)
//!   3. Inline null + dedicated endpoint returns None (403/404 — the author
//!      has disabled third-party distribution; user must open in browser)

use crate::mods::curseforge::client::CurseForgeClient;
use crate::mods::curseforge::error::CurseForgeError;
use crate::mods::curseforge::types::{CurseForgeFileEntry, CurseForgeProjectDetail};
use crate::mods::curseforge::url::web_url_for_file;

/// Result of the download-URL resolution. v1 carries only the success case;
/// the failure case surfaces as `Err(CurseForgeError::FileNotDownloadable)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadResolution {
    Resolved(String),
}

/// Resolve the actual CDN URL for a CurseForge file, with the `downloadUrl: null`
/// fallback chain. Returns `Err(FileNotDownloadable)` carrying the user-facing
/// web URL when the file is restricted at the API level.
///
/// Per 09-RESEARCH.md §"downloadUrl null UX" lines 247-289 — the three cases:
///
///   1. `file.download_url == Some(non-empty)` → `Ok(Resolved(url))`,
///      ZERO additional HTTP calls.
///   2. `file.download_url == None` (or empty) AND `client.get_file_download_url`
///      returns `Ok(Some(url))` → `Ok(Resolved(url))` (transient null recovered).
///   3. Same as case 2 but inner returns `Ok(None)` (403/404 at the client layer)
///      → `Err(CurseForgeError::FileNotDownloadable { web_url, mod_slug, file_id })`
///      with `web_url` built via `web_url_for_file`.
///
/// Any HTTP error from `get_file_download_url` (e.g. RateLimited, 5xx) bubbles
/// up via `?` — the FileNotDownloadable variant is reserved for the explicit
/// 403/404 restricted-distribution signal.
pub async fn resolve_download_url(
    client: &CurseForgeClient,
    mod_detail: &CurseForgeProjectDetail,
    file: &CurseForgeFileEntry,
) -> Result<DownloadResolution, CurseForgeError> {
    // Case 1: inline URL present and non-empty.
    // Pitfall 5: when inline URL present, do NOT call the fallback endpoint.
    if let Some(u) = &file.download_url {
        if !u.is_empty() {
            return Ok(DownloadResolution::Resolved(u.clone()));
        }
    }

    // Case 2 / 3: inline absent or empty, try dedicated endpoint.
    match client
        .get_file_download_url(mod_detail.id, file.id)
        .await?
    {
        Some(url) => Ok(DownloadResolution::Resolved(url)),
        None => Err(CurseForgeError::FileNotDownloadable {
            web_url: web_url_for_file(&mod_detail.slug, file.id),
            mod_slug: mod_detail.slug.clone(),
            file_id: file.id,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::curseforge::types::{CurseForgeAuthor, CurseForgeHash, CurseForgeLinks};
    use httpmock::prelude::*;

    fn make_client(server: &MockServer) -> CurseForgeClient {
        CurseForgeClient::new_with_base_url("test-key", server.base_url())
            .expect("client construction")
    }

    fn detail(id: u64, slug: &str) -> CurseForgeProjectDetail {
        CurseForgeProjectDetail {
            id,
            slug: slug.to_string(),
            name: "X".into(),
            summary: String::new(),
            description: String::new(),
            download_count: 0,
            authors: vec![CurseForgeAuthor {
                id: 1,
                name: "A".into(),
                url: String::new(),
            }],
            links: CurseForgeLinks::default(),
        }
    }

    fn file(id: u64, download_url: Option<String>) -> CurseForgeFileEntry {
        CurseForgeFileEntry {
            id,
            display_name: "X 1.0".into(),
            file_name: "x.jar".into(),
            release_type: 1,
            file_status: 4,
            hashes: vec![CurseForgeHash {
                value: "abc".into(),
                algo: 1,
            }],
            file_date: "2026-01-01T00:00:00Z".into(),
            file_length: 1024,
            download_count: 1,
            download_url,
            game_versions: vec!["1.20.4".into()],
            dependencies: vec![],
            is_available: true,
        }
    }

    // Case 1: inline URL present — ZERO calls to /download-url endpoint.
    #[tokio::test]
    async fn test_inline_url_present_resolves_without_fallback_call() {
        let server = MockServer::start();
        // Prepare a mock that MUST NOT be called (assert_hits(0) at end).
        let unwanted = server.mock(|when, then| {
            when.method(GET).path("/v1/mods/443959/files/1/download-url");
            then.status(200)
                .body(r#"{"data":"https://should-not-be-called/x.jar"}"#);
        });
        let c = make_client(&server);
        let d = detail(443959, "x");
        let f = file(1, Some("https://edge.forgecdn.net/files/inline.jar".into()));
        let r = resolve_download_url(&c, &d, &f).await.unwrap();
        assert_eq!(
            r,
            DownloadResolution::Resolved("https://edge.forgecdn.net/files/inline.jar".into())
        );
        unwanted.assert_calls(0); // Pitfall 5: when inline URL present, we MUST NOT call the fallback endpoint
    }

    // Case 2: inline null + dedicated endpoint returns Some(url).
    #[tokio::test]
    async fn test_inline_null_with_fallback_some_resolves() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/443959/files/1/download-url");
            then.status(200)
                .body(r#"{"data":"https://edge.forgecdn.net/files/fallback.jar"}"#);
        });
        let c = make_client(&server);
        let d = detail(443959, "x");
        let f = file(1, None);
        let r = resolve_download_url(&c, &d, &f).await.unwrap();
        assert_eq!(
            r,
            DownloadResolution::Resolved("https://edge.forgecdn.net/files/fallback.jar".into())
        );
    }

    // Case 2.5: inline EMPTY string (treated as null) + dedicated endpoint returns Some(url).
    #[tokio::test]
    async fn test_inline_empty_string_treated_as_null() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/443959/files/1/download-url");
            then.status(200)
                .body(r#"{"data":"https://edge.forgecdn.net/files/fallback.jar"}"#);
        });
        let c = make_client(&server);
        let d = detail(443959, "x");
        let f = file(1, Some("".into()));
        let r = resolve_download_url(&c, &d, &f).await.unwrap();
        assert_eq!(
            r,
            DownloadResolution::Resolved("https://edge.forgecdn.net/files/fallback.jar".into())
        );
    }

    // Case 3: inline null + dedicated endpoint returns 403 (Ok(None) at client layer).
    #[tokio::test]
    async fn test_inline_null_with_fallback_403_returns_file_not_downloadable() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/v1/mods/443959/files/4567890/download-url");
            then.status(403).body(r#"{"error":"restricted"}"#);
        });
        let c = make_client(&server);
        let d = detail(443959, "wonderful-world-mod");
        let f = file(4567890, None);
        let r = resolve_download_url(&c, &d, &f).await;
        match r {
            Err(CurseForgeError::FileNotDownloadable {
                web_url,
                mod_slug,
                file_id,
            }) => {
                assert_eq!(
                    web_url,
                    "https://www.curseforge.com/minecraft/mc-mods/wonderful-world-mod/files/4567890"
                );
                assert_eq!(mod_slug, "wonderful-world-mod");
                assert_eq!(file_id, 4567890);
            }
            other => panic!("expected FileNotDownloadable with correct web_url, got {other:?}"),
        }
    }

    // Case 3 variant: inline null + dedicated endpoint returns 404 (also Ok(None)).
    #[tokio::test]
    async fn test_inline_null_with_fallback_404_returns_file_not_downloadable() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/443959/files/1/download-url");
            then.status(404).body("not found");
        });
        let c = make_client(&server);
        let d = detail(443959, "x");
        let f = file(1, None);
        let r = resolve_download_url(&c, &d, &f).await;
        assert!(
            matches!(r, Err(CurseForgeError::FileNotDownloadable { .. })),
            "expected FileNotDownloadable, got {r:?}"
        );
    }

    // Surfacing test: rate-limit on the fallback bubbles up cleanly (NOT FileNotDownloadable).
    #[tokio::test]
    async fn test_inline_null_with_fallback_429_bubbles_rate_limited() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/mods/443959/files/1/download-url");
            then.status(429)
                .header("Retry-After", "30")
                .body("rate limited");
        });
        let c = make_client(&server);
        let d = detail(443959, "x");
        let f = file(1, None);
        let r = resolve_download_url(&c, &d, &f).await;
        assert!(
            matches!(
                r,
                Err(CurseForgeError::RateLimited {
                    retry_after_secs: 30
                })
            ),
            "expected RateLimited, got {r:?}"
        );
    }
}
