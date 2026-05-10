//! Icon rendering domain (Phase 13).
//!
//! Pieces shipped so far:
//!
//! - `IconSource` -- enum tagging where an icon came from. Mirrors the
//!   on-disk cache directory split (`{cache}/icons/modrinth/...` vs
//!   `{cache}/icons/curseforge/...`).
//! - `client::IconClient` -- HTTP fetcher with cache-first probe. Owns
//!   the `reqwest::Client` and a `Semaphore(4)` cap on concurrent
//!   downloads.
//!
//! Subsequent plans (13-04 / 13-05 / 13-06) layer terminal protocol
//! detection, in-memory `Protocol` LRU, and detail-pane render on top.
//! List-row icons are deferred to Phase B per
//! `.planning/spikes/001-icon-rendering-quality/README.md` (halfblocks
//! quality is unusable, so per-row icons must be protocol-gated and
//! require rewriting the existing `Table`-based list views).

pub mod client;

pub use client::IconClient;

/// Where an icon was sourced from. Used as the cache directory shard so
/// two projects sharing an id across registries can never collide on
/// disk, and so a future `ichr cache clear icons modrinth` command can
/// scope its work cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconSource {
    Modrinth,
    Curseforge,
}

impl IconSource {
    /// Stable lowercase slug used as the cache subdirectory name.
    pub fn slug(self) -> &'static str {
        match self {
            IconSource::Modrinth => "modrinth",
            IconSource::Curseforge => "curseforge",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_stable_lowercase() {
        assert_eq!(IconSource::Modrinth.slug(), "modrinth");
        assert_eq!(IconSource::Curseforge.slug(), "curseforge");
    }
}
