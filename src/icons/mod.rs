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
pub mod service;

pub use client::IconClient;
pub use service::IconService;

/// Predicate consumed by detail-pane renderers: should icons render at
/// all given the terminal's detected image protocol?
///
/// Returns `true` only when the picker exists AND the detected protocol
/// is something better than halfblocks. Halfblocks is intentionally
/// rejected: Spike 001 verified that halfblocks output at the sizes
/// usable in a TUI row is unrecognizable mush, so showing it to users
/// is worse than showing nothing.
///
/// `None` (detection failed -- terminal didn't respond, or terminfo
/// missing) is also treated as "icons disabled". Better to fall back
/// silently than to risk a broken rendering.
pub fn rendering_enabled(picker: Option<&ratatui_image::picker::Picker>) -> bool {
    matches!(
        picker.map(|p| p.protocol_type()),
        Some(t) if t != ratatui_image::picker::ProtocolType::Halfblocks
    )
}

#[cfg(test)]
mod predicate_tests {
    use super::*;
    use ratatui_image::picker::Picker;

    #[test]
    fn rendering_disabled_when_picker_is_none() {
        assert!(!rendering_enabled(None));
    }

    #[test]
    fn rendering_disabled_for_halfblocks_picker() {
        let picker = Picker::halfblocks();
        assert!(
            !rendering_enabled(Some(&picker)),
            "halfblocks must disable icons -- spike 001 verdict"
        );
    }
}

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
