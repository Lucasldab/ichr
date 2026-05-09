//! Offline authentication context.
//!
//! Generates a deterministic UUID matching Java's
//! `UUID.nameUUIDFromBytes(("OfflinePlayer:" + username).getBytes(UTF_8))` --
//! a version-3 (MD5-based name) UUID per RFC 4122 §4.3.
//!
//! Uses the `md-5` crate (RustCrypto family, import name `md5`).
//! Access token for offline mode: the string `"0"`.

use md5::{Digest, Md5};

/// Offline auth fields -- everything `SubstitutionContext` needs for an
/// unauthenticated (offline) launch.
#[derive(Debug, Clone)]
pub struct OfflineAuth {
    /// The username as entered by the user.
    pub username: String,
    /// Deterministic version-3 UUID derived from the username.
    pub uuid: String,
    /// Always `"0"` for offline mode.
    pub access_token: String,
    /// Always `"legacy"` for offline mode. Online MSA uses `"msa"`.
    pub user_type: String,
}

/// Compute the offline UUID for `username`.
///
/// Algorithm:
/// 1. Hash `"OfflinePlayer:{username}"` bytes with MD5 → 16-byte digest.
/// 2. Set version nibble (byte[6] bits 4-7) to `0b0011` (version 3).
/// 3. Set variant bits (byte[8] bits 6-7) to `0b10` (RFC 4122).
/// 4. Format as lowercase hyphenated UUID string (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).
///
/// This matches `java.util.UUID.nameUUIDFromBytes` byte-for-byte.
pub fn offline_uuid(username: &str) -> String {
    let input = format!("OfflinePlayer:{username}");
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    let mut bytes: [u8; 16] = digest.into();
    // version = 3 (name-based MD5): bits 4-7 of byte 6 = 0011
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    // variant = RFC 4122: bits 6-7 of byte 8 = 10
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

/// Build a full `OfflineAuth` for `username`.
pub fn offline_auth(username: &str) -> OfflineAuth {
    OfflineAuth {
        uuid: offline_uuid(username),
        username: username.to_string(),
        access_token: "0".to_string(),
        user_type: "legacy".to_string(),
    }
}

/// MSA auth fields -- adapter from `crate::auth::MsaTokens` to the fields
/// consumed by `SubstitutionContext`. Keeps all auth-field production in
/// one place alongside the offline equivalent (`OfflineAuth`).
#[derive(Debug, Clone)]
pub struct MsaAuth {
    /// `mc_player_name` from `MsaTokens`.
    pub username: String,
    /// Hyphenated MC UUID from `MsaTokens.mc_uuid`.
    pub uuid: String,
    /// Live MC access token from `MsaTokens.mc_access_token`.
    pub access_token: String,
    /// XSTS user hash (uhs) from `MsaTokens.xuid`.
    pub xuid: String,
    /// XSTS user hash (uhs) from `MsaTokens.user_hash` (same value as `xuid`).
    pub xbox_user_hash: String,
    /// MSA client ID -- from `crate::auth::device_code::client_id()`.
    pub clientid: String,
    /// Always `"msa"` for this variant. Offline uses `"legacy"`.
    pub user_type: String,
}

impl MsaAuth {
    /// Convert `crate::auth::MsaTokens` into the launcher-local `MsaAuth` shape.
    /// `clientid` is sourced from `crate::auth::device_code::client_id()`
    /// so any env-var override is respected automatically.
    pub fn from_tokens(tokens: &crate::auth::MsaTokens) -> Self {
        Self {
            username: tokens.mc_player_name.clone(),
            uuid: tokens.mc_uuid.clone(),
            access_token: tokens.mc_access_token.clone(),
            xuid: tokens.xuid.clone(),
            xbox_user_hash: tokens.user_hash.clone(),
            clientid: crate::auth::device_code::client_id(),
            user_type: tokens.user_type.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offline_uuid_deterministic() {
        assert_eq!(offline_uuid("Player"), offline_uuid("Player"));
    }

    #[test]
    fn test_offline_uuid_is_version_3() {
        let uuid = offline_uuid("Player");
        // UUID format: 8-4-4-4-12
        // The version nibble is the first character of the 3rd group (index 14).
        let chars: Vec<char> = uuid.chars().collect();
        // positions: 0-7 = first group, 8 = hyphen, 9-12 = second group,
        // 13 = hyphen, 14 = version digit
        assert_eq!(
            chars[14], '3',
            "version nibble at position 14 must be '3'; uuid={uuid}"
        );
    }

    #[test]
    fn test_offline_uuid_rfc4122_variant() {
        let uuid = offline_uuid("Player");
        let chars: Vec<char> = uuid.chars().collect();
        // positions: 0-7, 8=hyphen, 9-12, 13=hyphen, 14-17, 18=hyphen, 19 = variant digit
        let variant = chars[19];
        assert!(
            matches!(variant, '8' | '9' | 'a' | 'b'),
            "RFC 4122 variant nibble at position 19 must be 8,9,a, or b; got '{variant}'; uuid={uuid}"
        );
    }

    #[test]
    fn test_offline_uuid_shape() {
        // Assert form: xxxxxxxx-xxxx-3xxx-[89ab]xxx-xxxxxxxxxxxx
        let uuid = offline_uuid("TestUser");
        let parts: Vec<&str> = uuid.split('-').collect();
        assert_eq!(
            parts.len(),
            5,
            "UUID must have 5 hyphen-separated groups; uuid={uuid}"
        );
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);

        // All characters must be hex digits (lowercase).
        for part in &parts {
            assert!(
                part.chars().all(|c| c.is_ascii_hexdigit()),
                "non-hex char in {part}; uuid={uuid}"
            );
        }

        // version nibble
        assert!(
            parts[2].starts_with('3'),
            "3rd group must start with '3'; uuid={uuid}"
        );

        // variant nibble (first char of 4th group)
        let variant = parts[3].chars().next().unwrap();
        assert!(
            matches!(variant, '8' | '9' | 'a' | 'b'),
            "4th group first char must be 8/9/a/b; got '{variant}'; uuid={uuid}"
        );
    }

    #[test]
    fn test_offline_uuid_different_usernames() {
        let a = offline_uuid("Player");
        let b = offline_uuid("AnotherPlayer");
        assert_ne!(a, b, "different usernames must produce different UUIDs");
    }

    #[test]
    fn test_offline_auth_access_token_is_zero() {
        let auth = offline_auth("X");
        assert_eq!(auth.access_token, "0");
    }

    #[test]
    fn test_offline_auth_user_type_is_legacy() {
        let auth = offline_auth("X");
        assert_eq!(auth.user_type, "legacy");
    }

    #[test]
    fn test_offline_auth_username_preserved() {
        let auth = offline_auth("SomeUser");
        assert_eq!(auth.username, "SomeUser");
    }

    #[test]
    fn test_offline_auth_uuid_matches_offline_uuid() {
        let username = "SomeUser";
        let auth = offline_auth(username);
        assert_eq!(auth.uuid, offline_uuid(username));
    }

    #[test]
    fn test_msa_auth_from_tokens() {
        let t = crate::auth::MsaTokens {
            mc_access_token: "mc-tok".into(),
            mc_uuid: "11111111-1111-4111-8111-111111111111".into(),
            mc_player_name: "PlayerOne".into(),
            xuid: "uhs-1".into(),
            user_hash: "uhs-1".into(),
            user_type: "msa".into(),
        };
        let a = MsaAuth::from_tokens(&t);
        assert_eq!(a.username, "PlayerOne");
        assert_eq!(a.access_token, "mc-tok");
        assert_eq!(a.uuid, "11111111-1111-4111-8111-111111111111");
        assert_eq!(a.xuid, "uhs-1");
        assert_eq!(a.xbox_user_hash, "uhs-1");
        assert_eq!(a.user_type, "msa");
        // Default when ICHR_MSA_CLIENT_ID env var is unset -- pulled
        // from `auth::device_code::DEFAULT_MSA_CLIENT_ID` so the test
        // tracks the constant rather than restating its value.
        assert_eq!(a.clientid, crate::auth::device_code::DEFAULT_MSA_CLIENT_ID);
    }
}
