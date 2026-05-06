//! Pure CurseForge web URL builder — no I/O.
//!
//! Used by the install path's `downloadUrl: null` fallback (09-RESEARCH.md
//! §"downloadUrl null UX" line 252). When the API returns a null download
//! URL AND the dedicated /download-url endpoint also returns 403/404, we
//! surface a `CurseForgeError::FileNotDownloadable { web_url, ... }` so the
//! `cf_install_failed_modal.rs` view can display the link the user must
//! open in their browser.

/// Build the CurseForge web URL for a file.
///
/// Both `mod_slug` and `file_id` are already in scope at the failure point
/// (the `Mod` response carries the slug; the `File` response carries the id).
/// Per 09-RESEARCH.md §"Web URL for restricted-download fallback" line 146.
pub fn web_url_for_file(mod_slug: &str, file_id: u64) -> String {
    format!("https://www.curseforge.com/minecraft/mc-mods/{mod_slug}/files/{file_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_url_canonical_shape() {
        assert_eq!(
            web_url_for_file("wonderful-world-mod", 4567890),
            "https://www.curseforge.com/minecraft/mc-mods/wonderful-world-mod/files/4567890"
        );
    }

    #[test]
    fn test_web_url_for_short_slug() {
        assert_eq!(
            web_url_for_file("ftb", 1),
            "https://www.curseforge.com/minecraft/mc-mods/ftb/files/1"
        );
    }

    #[test]
    fn test_empty_slug_does_not_panic() {
        // Per 09-RESEARCH.md line 285 — empty slug yields a syntactically-valid
        // but-broken URL. The user sees the breakage and backs out; we do not panic.
        let url = web_url_for_file("", 1);
        assert!(
            url.starts_with("https://www.curseforge.com/"),
            "url should still be a valid CurseForge prefix: {url}"
        );
        assert!(url.ends_with("/files/1"));
    }

    #[test]
    fn test_max_u64_file_id_renders() {
        // Defensive: file_id is u64 from the wire; ensure no overflow / format issue.
        let url = web_url_for_file("x", u64::MAX);
        assert!(url.contains(&u64::MAX.to_string()));
    }
}
