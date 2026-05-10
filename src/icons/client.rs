//! HTTP fetch + on-disk cache for project icons (Modrinth, CurseForge).
//!
//! Mirrors `MojangJreClient` in shape -- same `reqwest::Client::builder()`
//! settings (gzip, 30s/10s timeouts, ichr User-Agent) and the same
//! cache-first probe pattern that lets offline launches hit local files
//! without touching the network.
//!
//! The client knows nothing about ratatui-image; it only ferries bytes
//! to disk and reports the on-disk path. Decoding into a renderable
//! `Protocol` is the IconService's job (13-05).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;

use crate::error::AppError;
use crate::icons::IconSource;
use crate::mojang::cache::atomic_write;
use crate::persistence::paths::AppPaths;

/// Extensions probed in order when looking for a cached icon. The fetch
/// path writes whichever format the bytes actually decode as, so any
/// match here is a real cache hit.
///
/// Kept short on purpose -- adding `jpg` / `gif` / etc. would slow the
/// cache lookup with no payoff for the protocols Phase 13 actually
/// targets (Modrinth ships PNG and WebP; CurseForge logos are PNG).
const CANDIDATE_EXTS: &[&str] = &["png", "webp"];

/// HTTP facade for icon fetches.
#[derive(Debug, Clone)]
pub struct IconClient {
    http: reqwest::Client,
    /// Caps concurrent HTTP fetches across the whole app. Polite to
    /// Modrinth / CurseForge during navigation bursts and prevents the
    /// thundering-herd profile when a user scrolls fast through a busy
    /// browser.
    sem: Arc<Semaphore>,
}

impl IconClient {
    /// Bound on simultaneous in-flight icon fetches. Locked here so the
    /// IconService (13-05) and any direct caller share the same cap.
    pub const MAX_CONCURRENT_FETCHES: usize = 4;

    /// Construct with the launcher's User-Agent and matching reqwest
    /// settings as `MojangJreClient`.
    pub fn new() -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .user_agent(crate::mojang::client::USER_AGENT)
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AppError::Http(format!("reqwest build (icons): {e}")))?;
        Ok(Self {
            http,
            sem: Arc::new(Semaphore::new(Self::MAX_CONCURRENT_FETCHES)),
        })
    }

    /// Cache-first: if a file already exists at `paths.icon_path(source, project_id, *)`
    /// for any candidate extension, return it without a network call.
    /// Otherwise fetch `url`, sniff the format from the bytes, and write
    /// the file to disk under the detected extension.
    ///
    /// Errors surface as `AppError::Http` with the URL and status / body
    /// reason -- the caller (IconService) is responsible for mapping these
    /// into `tracing::warn!` + a blank icon area in the detail pane.
    #[tracing::instrument(skip_all, fields(source = ?source, project_id, url))]
    pub async fn ensure_cached(
        &self,
        paths: &AppPaths,
        source: IconSource,
        project_id: &str,
        url: &str,
    ) -> Result<PathBuf, AppError> {
        // Cache probe -- any matching extension wins.
        for ext in CANDIDATE_EXTS {
            let p = paths.icon_path(source, project_id, ext);
            if tokio::fs::try_exists(&p).await? {
                tracing::debug!(path = %p.display(), "icon cache hit");
                return Ok(p);
            }
        }

        // Concurrency-capped fetch.
        let _permit = self
            .sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| AppError::Http(format!("icon semaphore acquire: {e}")))?;

        let bytes = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| AppError::Http(format!("GET icon {url}: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Http(format!("icon {url} status: {e}")))?
            .bytes()
            .await
            .map_err(|e| AppError::Http(format!("icon {url} body: {e}")))?
            .to_vec();

        let ext = sniff_extension(&bytes).unwrap_or_else(|| {
            tracing::warn!(
                url,
                "icon bytes did not decode to a known format -- caching as .bin"
            );
            "bin"
        });

        let dest = paths.icon_path(source, project_id, ext);
        atomic_write(&dest, &bytes).await?;
        tracing::debug!(path = %dest.display(), "icon cached");
        Ok(dest)
    }
}

/// Detect a file extension from raw image bytes via `image`'s magic-number
/// sniffer. Returns `None` for any format Phase 13 doesn't cache (we don't
/// want to fabricate a `.tiff` file).
fn sniff_extension(bytes: &[u8]) -> Option<&'static str> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    match reader.format()? {
        image::ImageFormat::Png => Some("png"),
        image::ImageFormat::WebP => Some("webp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::GET;
    use httpmock::MockServer;
    use tempfile::TempDir;

    fn make_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    fn tiny_png_bytes() -> Vec<u8> {
        let img = image::DynamicImage::new_rgba8(2, 2);
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("png encode");
        bytes
    }

    #[tokio::test]
    async fn ensure_cached_fetches_and_writes_png() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();
        let png = tiny_png_bytes();

        let m = server.mock(|when, then| {
            when.method(GET).path("/icon.png");
            then.status(200)
                .header("content-type", "image/png")
                .body(png.clone());
        });

        let client = IconClient::new().expect("client build");
        let url = format!("{}/icon.png", server.base_url());

        let path = client
            .ensure_cached(&paths, IconSource::Modrinth, "AANobbMI", &url)
            .await
            .expect("fetch");

        assert_eq!(
            path,
            paths.icon_path(IconSource::Modrinth, "AANobbMI", "png")
        );
        assert!(path.exists(), "cache file must be written");
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, png, "bytes on disk must match served bytes");
        assert_eq!(m.calls(), 1, "first call must hit the network");
    }

    #[tokio::test]
    async fn second_call_uses_cache_no_network() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();
        let png = tiny_png_bytes();

        let m = server.mock(|when, then| {
            when.method(GET).path("/icon.png");
            then.status(200).body(png.clone());
        });

        let client = IconClient::new().expect("client build");
        let url = format!("{}/icon.png", server.base_url());

        let p1 = client
            .ensure_cached(&paths, IconSource::Modrinth, "AANobbMI", &url)
            .await
            .expect("first fetch");
        let p2 = client
            .ensure_cached(&paths, IconSource::Modrinth, "AANobbMI", &url)
            .await
            .expect("second fetch");

        assert_eq!(p1, p2, "cache hit must return the same path");
        assert_eq!(m.calls(), 1, "second call must NOT hit the network");
    }

    #[tokio::test]
    async fn cache_hit_with_prepopulated_file_skips_network() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        // Pre-populate the cache slot. Mock would 500 if the client
        // contacted it -- the test fails loudly if cache-first is broken.
        let cached = paths.icon_path(IconSource::Curseforge, "238222", "png");
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        std::fs::write(&cached, b"already-cached-bytes\n").unwrap();

        let m = server.mock(|when, then| {
            when.method(GET).path("/icon.png");
            then.status(500).body("must not be called");
        });

        let client = IconClient::new().expect("client build");
        let url = format!("{}/icon.png", server.base_url());

        let path = client
            .ensure_cached(&paths, IconSource::Curseforge, "238222", &url)
            .await
            .expect("cached path must resolve without network");

        assert_eq!(path, cached);
        assert_eq!(
            m.calls(),
            0,
            "the GET must NOT happen when a cached file is present"
        );
    }

    #[tokio::test]
    async fn http_404_returns_app_error_http() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let _m = server.mock(|when, then| {
            when.method(GET).path("/missing.png");
            then.status(404);
        });

        let client = IconClient::new().expect("client build");
        let url = format!("{}/missing.png", server.base_url());

        let result = client
            .ensure_cached(&paths, IconSource::Modrinth, "ZZZ", &url)
            .await;

        match result {
            Err(AppError::Http(msg)) => {
                assert!(
                    msg.contains("404") || msg.to_ascii_lowercase().contains("not found"),
                    "404 error message must mention status: {msg}"
                );
            }
            other => panic!("expected AppError::Http, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_format_caches_as_bin() {
        // Bytes that don't match any image magic number -- the sniffer
        // returns None and the client falls back to .bin, so the URL is
        // cached but the IconService will later refuse to decode it.
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let garbage = b"this is not an image\n".to_vec();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/garbage.png");
            then.status(200).body(garbage.clone());
        });

        let client = IconClient::new().expect("client build");
        let url = format!("{}/garbage.png", server.base_url());

        let path = client
            .ensure_cached(&paths, IconSource::Modrinth, "garbage", &url)
            .await
            .expect("client should write whatever bytes came back");

        assert_eq!(
            path,
            paths.icon_path(IconSource::Modrinth, "garbage", "bin")
        );
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, garbage);
    }
}
