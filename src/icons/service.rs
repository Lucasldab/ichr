//! Icon service facade -- owns the HTTP client, the image-protocol
//! picker, and a bounded LRU of decoded `Protocol`s keyed by
//! `(IconSource, project_id)`.
//!
//! Three layers compose here:
//!
//!   1. `IconClient` (13-03) -- network fetch + on-disk cache by URL.
//!   2. `image::ImageReader::with_guessed_format()` -- bytes -> `DynamicImage`.
//!   3. `Picker::new_protocol()` (ratatui-image) -- `DynamicImage` -> `Protocol`,
//!      sized for a fixed Rect (the detail-pane avatar slot).
//!
//! Detail-pane render fns call `try_get(...)`. On a cache hit they get a
//! clone-cheap `Protocol` and render via `Image::new(&protocol)` immediately.
//! On a cache miss they enqueue an `Effect::FetchIcon` that the run-loop
//! routes to `fetch_and_decode`; on completion the run-loop sends
//! `Action::IconFetched`, which triggers a re-render and the next
//! `try_get` call returns Some.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::Resize;

use crate::error::AppError;
use crate::icons::{IconClient, IconSource};
use crate::persistence::paths::AppPaths;

/// Maximum number of decoded `Protocol`s held in memory at once.
///
/// Sized for the visible browser pages plus any in-flight detail-pane
/// navigations -- a user with very fast scroll bursts can blow past this
/// briefly, but the on-disk cache absorbs the re-decode penalty
/// (re-decoding a cached PNG is sub-millisecond on modern hardware).
pub const PROTOCOL_LRU_SIZE: usize = 64;

/// Cache key. Sources are split per registry so id collisions across
/// Modrinth and CurseForge can never shadow each other.
type IconKey = (IconSource, String);

/// Owning facade. Cheap to clone the inner Arcs; pass via `Arc<IconService>`
/// from `AppState` so spawned async tasks can share the LRU.
pub struct IconService {
    client: IconClient,
    /// Picker is `Clone` and effectively read-only after construction.
    /// Cloned per-call into `new_protocol`; cheap.
    picker: Picker,
    /// Decoded `Protocol` LRU. `Mutex` (not `tokio::sync::Mutex`) because
    /// the critical section is short -- single map ops -- and never crosses
    /// an await point.
    protocols: Arc<Mutex<LruCache<IconKey, Protocol>>>,
}

impl IconService {
    /// Construct from a previously-detected picker (see `tui::terminal::init`).
    /// Builds its own `IconClient` internally so callers don't have to wire
    /// the HTTP layer separately.
    pub fn new(picker: Picker) -> Result<Self, AppError> {
        let cap = NonZeroUsize::new(PROTOCOL_LRU_SIZE).expect("PROTOCOL_LRU_SIZE must be non-zero");
        Ok(Self {
            client: IconClient::new()?,
            picker,
            protocols: Arc::new(Mutex::new(LruCache::new(cap))),
        })
    }

    /// Returns the cached `Protocol` for `(source, project_id)` if it's
    /// already decoded in memory. Does NOT trigger a fetch -- callers
    /// dispatch `Effect::FetchIcon` separately and wait for
    /// `Action::IconFetched` before calling `try_get` again.
    pub fn try_get(&self, source: IconSource, project_id: &str) -> Option<Protocol> {
        let key: IconKey = (source, project_id.to_string());
        let mut guard = self
            .protocols
            .lock()
            .expect("icon LRU mutex poisoned -- earlier panic on this Mutex");
        guard.get(&key).cloned()
    }

    /// Fetch (network or disk-cache) → decode → encode protocol → insert
    /// into LRU. Runs the CPU-bound decode on `spawn_blocking` so the TUI
    /// loop doesn't stall.
    ///
    /// `target` is the on-screen `Rect` the protocol will eventually render
    /// into. ratatui-image bakes the size into the protocol; rendering
    /// into a different-sized Rect would need re-encoding.
    #[tracing::instrument(skip_all, fields(source = ?source, project_id, url))]
    pub async fn fetch_and_decode(
        &self,
        paths: &AppPaths,
        source: IconSource,
        project_id: &str,
        url: &str,
        target: Rect,
    ) -> Result<(), AppError> {
        let path = self
            .client
            .ensure_cached(paths, source, project_id, url)
            .await?;
        let bytes = tokio::fs::read(&path).await?;

        // CPU-bound: decode + re-encode for the chosen protocol. Off the
        // tokio runtime so a slow image (rare; icons are tiny) can't stall
        // input handling.
        let img = tokio::task::spawn_blocking(move || -> Result<image::DynamicImage, AppError> {
            let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|e| AppError::Http(format!("icon decode (sniff): {e}")))?;
            reader
                .decode()
                .map_err(|e| AppError::Http(format!("icon decode: {e}")))
        })
        .await
        .map_err(|e| AppError::Http(format!("icon decode join: {e}")))??;

        let protocol = self
            .picker
            .new_protocol(img, target, Resize::Fit(None))
            .map_err(|e| AppError::Http(format!("icon new_protocol: {e}")))?;

        let key: IconKey = (source, project_id.to_string());
        self.protocols
            .lock()
            .expect("icon LRU mutex poisoned")
            .put(key, protocol);
        Ok(())
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
        let img = image::DynamicImage::new_rgba8(8, 8);
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("png encode");
        bytes
    }

    fn target_rect() -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 8,
            height: 4,
        }
    }

    #[tokio::test]
    async fn try_get_returns_none_before_fetch() {
        let svc = IconService::new(Picker::halfblocks()).expect("service");
        assert!(svc.try_get(IconSource::Modrinth, "AANobbMI").is_none());
    }

    #[tokio::test]
    async fn fetch_and_decode_populates_cache() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();
        let png = tiny_png_bytes();

        let _m = server.mock(|when, then| {
            when.method(GET).path("/icon.png");
            then.status(200).body(&png[..]);
        });

        let svc = IconService::new(Picker::halfblocks()).expect("service");
        let url = format!("{}/icon.png", server.base_url());

        svc.fetch_and_decode(
            &paths,
            IconSource::Modrinth,
            "AANobbMI",
            &url,
            target_rect(),
        )
        .await
        .expect("fetch_and_decode");

        let proto = svc.try_get(IconSource::Modrinth, "AANobbMI");
        assert!(proto.is_some(), "protocol must be cached after fetch");
    }

    #[tokio::test]
    async fn lru_evicts_oldest_when_capacity_exceeded() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();
        let png = tiny_png_bytes();

        let svc = IconService::new(Picker::halfblocks()).expect("service");

        // Fill the LRU to capacity + 1 so the first inserted key is evicted.
        // Each id gets its own unique mock path so httpmock matches it verbatim.
        for i in 0..(PROTOCOL_LRU_SIZE + 1) {
            let id = format!("p{i}");
            let path = format!("/icon-{i}.png");
            let _m = server.mock(|when, then| {
                when.method(GET).path(path.clone());
                then.status(200).body(&png[..]);
            });
            let url = format!("{}{}", server.base_url(), path);
            svc.fetch_and_decode(&paths, IconSource::Modrinth, &id, &url, target_rect())
                .await
                .expect("fetch");
        }

        // The first inserted key (`p0`) must have been evicted.
        assert!(
            svc.try_get(IconSource::Modrinth, "p0").is_none(),
            "oldest entry must be evicted past capacity"
        );
        // The most recent insertion is still present.
        let last_id = format!("p{}", PROTOCOL_LRU_SIZE);
        assert!(
            svc.try_get(IconSource::Modrinth, &last_id).is_some(),
            "newest entry must still be cached"
        );
    }

    #[tokio::test]
    async fn fetch_failure_propagates_no_partial_cache() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let _m = server.mock(|when, then| {
            when.method(GET).path("/missing.png");
            then.status(404);
        });

        let svc = IconService::new(Picker::halfblocks()).expect("service");
        let url = format!("{}/missing.png", server.base_url());

        let result = svc
            .fetch_and_decode(&paths, IconSource::Modrinth, "ZZZ", &url, target_rect())
            .await;

        assert!(result.is_err(), "404 must propagate as Err");
        assert!(
            svc.try_get(IconSource::Modrinth, "ZZZ").is_none(),
            "failed fetch must not insert a partial entry"
        );
    }
}
