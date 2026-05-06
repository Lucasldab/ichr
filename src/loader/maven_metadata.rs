//! Maven metadata XML — minimal hand-rolled substring parser.
//!
//! Source schema: https://maven.apache.org/repositories/metadata.html
//!
//! The fixed-shape `<metadata><versioning><versions><version>...</version>...`
//! tree means a substring scan for `<version>...</version>` text nodes is
//! sufficient. We intentionally do NOT pull `quick-xml` — the document is
//! tiny (~50KB) and the failure modes (truncated input, bad encoding) are
//! handled by returning whatever was parsed up to the corruption point.
//!
//! Pitfall 8 of 07-RESEARCH.md: NeoForge versions can have 4 segments
//! (`26.1.2.41-beta`); this parser treats every text node as opaque, so
//! 4-segment versions are returned untouched.

/// Extract every `<version>...</version>` text node from a Maven metadata
/// XML document. Preserves order of appearance. Whitespace within text
/// nodes is trimmed. Never panics; malformed input returns whatever was
/// parsed up to the corruption point.
pub fn extract_versions(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0;
    let open = b"<version>";
    let open_len = open.len();
    while i + open_len <= bytes.len() {
        if &bytes[i..i + open_len] == open {
            let start = i + open_len;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'<' {
                end += 1;
            }
            if end <= bytes.len() {
                if let Ok(s) = std::str::from_utf8(&bytes[start..end]) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        out.push(trimmed.to_string());
                    }
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORGE_FIXTURE: &str = include_str!("../../tests/fixtures/forge_maven_metadata.xml");
    const NEOFORGE_FIXTURE: &str =
        include_str!("../../tests/fixtures/neoforge_maven_metadata.xml");

    #[test]
    fn test_extract_versions_empty() {
        assert!(extract_versions("").is_empty());
    }

    #[test]
    fn test_extract_versions_no_metadata() {
        assert!(extract_versions("<root><other>x</other></root>").is_empty());
    }

    #[test]
    fn test_extract_versions_three_inline() {
        let xml = r#"<metadata><versioning><versions>
            <version>1.0.0</version>
            <version>1.1.0</version>
            <version>2.0.0-beta</version>
        </versions></versioning></metadata>"#;
        let v = extract_versions(xml);
        assert_eq!(
            v,
            vec![
                "1.0.0".to_string(),
                "1.1.0".to_string(),
                "2.0.0-beta".to_string()
            ]
        );
    }

    #[test]
    fn test_extract_versions_truncated_does_not_panic() {
        // Open tag with no closing tag — must not panic
        let xml = "<metadata><versions><version>1.0.0";
        let _ = extract_versions(xml); // panic-free
    }

    #[test]
    fn test_extract_versions_forge_fixture_contains_known_releases() {
        let v = extract_versions(FORGE_FIXTURE);
        assert!(
            v.iter().any(|x| x == "1.20.1-47.4.20"),
            "Forge 1.20.1-47.4.20 missing: {v:?}"
        );
        assert!(v.iter().any(|x| x == "1.16.5-36.2.42"), "Forge 1.16.5-36.2.42 missing");
    }

    #[test]
    fn test_extract_versions_neoforge_fixture_contains_4_segment_beta() {
        let v = extract_versions(NEOFORGE_FIXTURE);
        assert!(
            v.iter().any(|x| x == "21.1.228"),
            "NeoForge 21.1.228 missing: {v:?}"
        );
        // Pitfall 8 — 4-segment betas must round-trip untouched
        assert!(
            v.iter().any(|x| x == "26.1.2.41-beta"),
            "NeoForge 4-segment beta missing"
        );
    }
}
