//! Pure mapping of XSTS `XErr` numeric codes to user-readable strings.
//!
//! XSTS (`https://xsts.auth.xboxlive.com/xsts/authorize`) returns HTTP 401
//! with a JSON body `{ "Identity": "0", "XErr": <u64>, "Message": "...", ... }`
//! when authorization is denied. Raw HTTP 401 is useless to users; the
//! caller (`src/auth/xbox.rs`) parses the XErr field and calls this
//! function to produce a human-readable message for the TUI.
//!
//! Codes documented at https://minecraft.wiki/w/Microsoft_authentication
//! (cross-referenced with Prism Launcher's xstsErrors.cpp table).
//!
//! This module is PURE SYNC — no async, no I/O, no reqwest. It is the
//! simplest auth module and is used as a spike target for early testing
//! in Phase 4.

/// Map an XSTS `XErr` numeric code to a user-facing error message.
///
/// Returns a string suitable for direct rendering in the TUI
/// `AccountAuthFailed` modal. Unknown codes produce a fallback
/// that includes the raw code and instructs the user to report it.
pub fn map_xerr(code: u64) -> String {
    match code {
        2148916227 => "This Microsoft account has been banned from Xbox services.".into(),
        2148916233 => "This Microsoft account does not have an Xbox profile. \
             Visit https://xbox.com/profile to create one, then try again."
            .into(),
        2148916235 => "Xbox Live is not available in your country or region.".into(),
        2148916236 => "Adult verification is required on this account (South Korea). \
             Please verify your age at https://account.microsoft.com."
            .into(),
        2148916237 => "Adult re-verification is required on this account (South Korea). \
             Please re-verify your age at https://account.microsoft.com."
            .into(),
        2148916238 => "This account belongs to a child. A parent or family account \
             manager must approve Xbox Live access at https://account.microsoft.com/family."
            .into(),
        2148916262 => "This account has been banned from Xbox Live services.".into(),
        other => format!(
            "Xbox authentication failed (XErr: {other}). \
             If this persists, please report this error code to the mineltui issue tracker."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_banned_account() {
        let s = map_xerr(2148916227);
        assert!(!s.is_empty());
        assert!(s.to_lowercase().contains("banned"), "got: {s}");
    }

    #[test]
    fn test_no_xbox_profile() {
        let s = map_xerr(2148916233);
        assert!(
            s.to_lowercase().contains("xbox profile"),
            "expected 'xbox profile' in output; got: {s}"
        );
        assert!(s.contains("xbox.com"), "expected xbox.com URL; got: {s}");
    }

    #[test]
    fn test_country_region() {
        let s = map_xerr(2148916235);
        assert!(
            s.to_lowercase().contains("country") || s.to_lowercase().contains("region"),
            "got: {s}"
        );
    }

    #[test]
    fn test_adult_verification_2148916236() {
        let s = map_xerr(2148916236);
        assert!(
            s.to_lowercase().contains("adult") || s.to_lowercase().contains("verif"),
            "got: {s}"
        );
    }

    #[test]
    fn test_adult_verification_2148916237() {
        let s = map_xerr(2148916237);
        assert!(
            s.to_lowercase().contains("adult") || s.to_lowercase().contains("verif"),
            "got: {s}"
        );
    }

    #[test]
    fn test_child_account() {
        let s = map_xerr(2148916238);
        assert!(
            s.to_lowercase().contains("family") || s.to_lowercase().contains("child"),
            "got: {s}"
        );
    }

    #[test]
    fn test_xbox_live_ban() {
        let s = map_xerr(2148916262);
        assert!(!s.is_empty());
        assert!(s.to_lowercase().contains("ban"), "got: {s}");
    }

    #[test]
    fn test_unknown_xerr_contains_raw_code() {
        let s = map_xerr(9999999999);
        assert!(
            s.contains("9999999999"),
            "unknown code fallback must include the raw number; got: {s}"
        );
        assert!(
            s.to_lowercase().contains("report"),
            "unknown code fallback must tell user to report; got: {s}"
        );
    }

    #[test]
    fn test_all_xerr_codes_distinct() {
        let codes = [
            2148916227u64,
            2148916233,
            2148916235,
            2148916236,
            2148916237,
            2148916238,
            2148916262,
        ];
        let mut seen = std::collections::HashSet::new();
        for c in codes {
            let s = map_xerr(c);
            assert!(!s.is_empty(), "code {c} produced empty string");
            assert!(
                seen.insert(s.clone()),
                "code {c} produced duplicate string: {s}"
            );
        }
    }

    #[test]
    fn test_no_panic_on_zero() {
        let s = map_xerr(0);
        assert!(!s.is_empty());
        assert!(s.contains("0"));
    }

    #[test]
    fn test_no_panic_on_max_u64() {
        let s = map_xerr(u64::MAX);
        assert!(!s.is_empty());
        assert!(s.contains(&u64::MAX.to_string()));
    }
}
