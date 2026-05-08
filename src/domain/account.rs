//! Account domain types. Phase 4 fleshes out the Microsoft-auth shape;
//! the Offline case is retained as `AccountKind::Offline` for the
//! `AuthContext::Offline { username }` launcher path (Phase 3 legacy).
//!
//! An `Account` always represents a **Microsoft account** on disk. Offline
//! launches never persist an `Account`; they use the instance display_name
//! at launch time.

use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AccountKind {
    Microsoft,
    Offline,
}

/// Which storage backend holds this account's refresh token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StorageBackend {
    /// OS keychain (libsecret / DPAPI) via the `keyring` crate.
    Keyring,
    /// Encrypted file at `{config_dir}/accounts.enc` (AES-256-GCM,
    /// machine-ID-derived key). Used when no secret service is running.
    EncryptedFile,
}

/// A persisted Microsoft account record. Refresh tokens are stored out-of-band
/// (keyring entry name = `id`, or encrypted file blob keyed by `id`) -- they
/// never live inside this struct in plaintext.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    /// Stable identifier -- the MS account object ID (or fallback to MC UUID).
    /// Also the keyring entry account name.
    pub id: String,
    /// Minecraft player name (e.g., "PlayerOne").
    pub mc_username: String,
    /// Minecraft UUID in hyphenated form (e.g., "c6bf8193-..." -- 36 chars).
    pub mc_uuid: String,
    /// Unix epoch seconds at which the cached MC access token expires.
    /// Chain orchestrator uses this to decide whether to refresh.
    pub mc_token_expires_at: i64,
    /// Unix epoch seconds at which the MSA access token expires.
    pub msa_token_expires_at: i64,
    /// When the account was first added (unix epoch seconds).
    pub added_at: i64,
    /// When the token was last refreshed (unix epoch seconds).
    pub last_refreshed_at: i64,
    /// Is this the currently-active account (used by launcher)?
    pub is_active: bool,
    /// Which backend holds the refresh token.
    pub storage: StorageBackend,
}

impl Account {
    /// Convert a `SystemTime` to Unix epoch seconds (0 if pre-epoch).
    pub fn to_unix(t: SystemTime) -> i64 {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_serde_roundtrip() {
        let acc = Account {
            id: "ms-object-id-123".to_string(),
            mc_username: "PlayerOne".to_string(),
            mc_uuid: "c6bf8193-0000-3000-8000-000000000001".to_string(),
            mc_token_expires_at: 1_700_000_000,
            msa_token_expires_at: 1_699_999_000,
            added_at: 1_699_900_000,
            last_refreshed_at: 1_699_950_000,
            is_active: true,
            storage: StorageBackend::EncryptedFile,
        };
        let json = serde_json::to_string(&acc).unwrap();
        let parsed: Account = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, acc.id);
        assert_eq!(parsed.mc_uuid, acc.mc_uuid);
        assert!(parsed.is_active);
        assert!(matches!(parsed.storage, StorageBackend::EncryptedFile));
    }

    #[test]
    fn test_storage_backend_serde() {
        let k = StorageBackend::Keyring;
        let json = serde_json::to_string(&k).unwrap();
        assert_eq!(json, "\"Keyring\"");
    }
}
