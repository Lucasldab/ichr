//! Keyring-primary + AES-256-GCM encrypted-file fallback for MSA refresh
//! tokens, plus a plain-JSON account metadata file for the TUI's account
//! list.
//!
//! Two storage tiers:
//!
//!   - **Secrets** (refresh tokens): `keyring::Entry::new("ichr", id)`
//!     → on any keyring error (libsecret daemon absent, etc. -- pitfall 21)
//!     → encrypted entry in `{config_dir}/accounts.enc` (AES-256-GCM with
//!     a 32-byte key derived from /etc/machine-id + a domain separator).
//!
//!   - **Metadata** (Account struct): `{config_dir}/accounts.json`, plain JSON,
//!     atomic-write semantics, no secrets. Holds mc_username, mc_uuid, expiry
//!     timestamps, is_active. The TUI reads this file to render the account
//!     picker without touching the secret tier.
//!
//! All keyring calls are wrapped in `spawn_blocking` because keyring 3.x
//! is synchronous and blocks the executor otherwise.

use std::collections::HashMap;
use std::path::PathBuf;

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use sha2::{Digest, Sha256};

use crate::auth::{Account, AuthError, StorageBackend};

pub const KEYRING_SERVICE: &str = "ichr";

/// Configuration injected into the storage layer. In production, built
/// from `AppPaths`. In tests, points to a tempdir + optionally forces
/// the fallback path.
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// Path to encrypted refresh-token file: `{config_dir}/accounts.enc`.
    pub accounts_enc_path: PathBuf,
    /// Path to plain-JSON account metadata file: `{config_dir}/accounts.json`.
    pub accounts_json_path: PathBuf,
    /// If true, skip keyring entirely -- always use the encrypted file.
    /// Used in CI / headless tests where libsecret is not available.
    pub force_fallback: bool,
}

impl StoreConfig {
    /// Build from the app's AppPaths, keyring enabled.
    pub fn from_paths(paths: &crate::persistence::paths::AppPaths) -> Self {
        Self {
            accounts_enc_path: paths.accounts_file(),
            accounts_json_path: paths.accounts_json_file(),
            force_fallback: false,
        }
    }
}

// ============================================================================
// Secret tier -- refresh tokens.
// ============================================================================

/// Store a refresh token for `account_id`. Returns the backend actually used.
/// Always tries keyring first (unless `config.force_fallback`).
#[tracing::instrument(name = "store_refresh_token", skip_all, fields(account_id = %account_id))]
pub async fn store_refresh_token(
    config: &StoreConfig,
    account_id: &str,
    token: &str,
) -> Result<StorageBackend, AuthError> {
    if !config.force_fallback {
        let aid = account_id.to_string();
        let tok = token.to_string();
        let keyring_result = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let entry = keyring::Entry::new(KEYRING_SERVICE, &aid).map_err(|e| e.to_string())?;
            entry.set_password(&tok).map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
        .map_err(|e| AuthError::StorageIo(format!("spawn_blocking: {e}")))?;

        match keyring_result {
            Ok(()) => return Ok(StorageBackend::Keyring),
            Err(e) => {
                tracing::warn!(error = %e, "keyring unavailable -- falling back to encrypted file");
            }
        }
    }
    // Fallback: encrypted file.
    store_in_encrypted_file(config, account_id, token).await?;
    Ok(StorageBackend::EncryptedFile)
}

/// Load a refresh token for `account_id`. Checks keyring first (unless
/// `force_fallback`); falls back to encrypted file; `AccountNotFound`
/// if neither holds a value.
#[tracing::instrument(name = "load_refresh_token", skip_all, fields(account_id = %account_id))]
pub async fn load_refresh_token(
    config: &StoreConfig,
    account_id: &str,
) -> Result<String, AuthError> {
    if !config.force_fallback {
        let aid = account_id.to_string();
        let keyring_result = tokio::task::spawn_blocking(move || -> Result<String, String> {
            let entry = keyring::Entry::new(KEYRING_SERVICE, &aid).map_err(|e| e.to_string())?;
            entry.get_password().map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| AuthError::StorageIo(format!("spawn_blocking: {e}")))?;

        if let Ok(tok) = keyring_result {
            return Ok(tok);
        }
    }
    match load_from_encrypted_file(config, account_id).await {
        Ok(tok) => Ok(tok),
        Err(AuthError::AccountNotFound(_)) => {
            Err(AuthError::AccountNotFound(account_id.to_string()))
        }
        Err(e) => Err(e),
    }
}

/// Remove from BOTH tiers. Idempotent (no error if absent).
#[tracing::instrument(name = "delete_refresh_token", skip_all, fields(account_id = %account_id))]
pub async fn delete_refresh_token(config: &StoreConfig, account_id: &str) -> Result<(), AuthError> {
    if !config.force_fallback {
        let aid = account_id.to_string();
        let _ = tokio::task::spawn_blocking(move || -> Result<(), String> {
            match keyring::Entry::new(KEYRING_SERVICE, &aid) {
                Ok(e) => {
                    let _ = e.delete_credential();
                    Ok(())
                }
                Err(err) => Err(err.to_string()),
            }
        })
        .await;
    }
    delete_from_encrypted_file(config, account_id).await?;
    Ok(())
}

// ============================================================================
// Encrypted file fallback.
// ============================================================================

/// Encrypted file format: JSON map of { account_id: base64(nonce || ciphertext) }.
/// The map itself is plaintext JSON; the *values* are encrypted blobs.
/// This keeps per-account granularity -- adding one account doesn't require
/// re-encrypting others.
async fn store_in_encrypted_file(
    config: &StoreConfig,
    account_id: &str,
    token: &str,
) -> Result<(), AuthError> {
    let mut map = load_encrypted_map(config).await?;
    let key_bytes = derive_machine_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|_| AuthError::DecryptFailed)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, token.as_bytes())
        .map_err(|_| AuthError::DecryptFailed)?;
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce);
    combined.extend_from_slice(&ciphertext);
    map.insert(account_id.to_string(), BASE64.encode(&combined));
    write_encrypted_map(config, &map).await
}

async fn load_from_encrypted_file(
    config: &StoreConfig,
    account_id: &str,
) -> Result<String, AuthError> {
    let map = load_encrypted_map(config).await?;
    let blob = map
        .get(account_id)
        .ok_or_else(|| AuthError::AccountNotFound(account_id.to_string()))?;
    let combined = BASE64
        .decode(blob)
        .map_err(|e| AuthError::StorageIo(format!("base64 decode: {e}")))?;
    if combined.len() < 12 {
        return Err(AuthError::DecryptFailed);
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let key_bytes = derive_machine_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|_| AuthError::DecryptFailed)?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| AuthError::DecryptFailed)?;
    String::from_utf8(plaintext).map_err(|_| AuthError::DecryptFailed)
}

async fn delete_from_encrypted_file(
    config: &StoreConfig,
    account_id: &str,
) -> Result<(), AuthError> {
    let mut map = load_encrypted_map(config).await?;
    if map.remove(account_id).is_none() {
        return Ok(());
    }
    write_encrypted_map(config, &map).await
}

async fn load_encrypted_map(config: &StoreConfig) -> Result<HashMap<String, String>, AuthError> {
    let path = &config.accounts_enc_path;
    if !tokio::fs::try_exists(path).await.unwrap_or(false) {
        return Ok(HashMap::new());
    }
    let raw = tokio::fs::read(path)
        .await
        .map_err(|e| AuthError::StorageIo(format!("read {path:?}: {e}")))?;
    if raw.is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_slice(&raw).map_err(|e| AuthError::StorageIo(format!("parse {path:?}: {e}")))
}

async fn write_encrypted_map(
    config: &StoreConfig,
    map: &HashMap<String, String>,
) -> Result<(), AuthError> {
    let path = &config.accounts_enc_path;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AuthError::StorageIo(format!("create_dir_all: {e}")))?;
    }
    let bytes = serde_json::to_vec_pretty(map)
        .map_err(|e| AuthError::StorageIo(format!("serialize: {e}")))?;
    // Atomic write: write to tmp, rename.
    let tmp = path.with_extension("enc.tmp");
    tokio::fs::write(&tmp, &bytes)
        .await
        .map_err(|e| AuthError::StorageIo(format!("write tmp: {e}")))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| AuthError::StorageIo(format!("rename: {e}")))?;
    // 0600 perms on Linux.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
    Ok(())
}

/// Derive a 32-byte AES-256 key from machine identity + a domain separator.
///
/// Linux: `/etc/machine-id` (32 hex chars). If absent, try
/// `/var/lib/dbus/machine-id`. If neither, fall back to a hash of
/// `$HOME` (acceptable per research A6 -- less secure but functional).
///
/// Windows: the keyring path uses DPAPI so the fallback should never
/// trigger. If it does (e.g., in a test with force_fallback), derive
/// from `%COMPUTERNAME%`.
pub fn derive_machine_key() -> Result<[u8; 32], AuthError> {
    let raw = read_machine_id();
    let mut hasher = Sha256::new();
    hasher.update(b"ichr-auth-v1:");
    hasher.update(&raw);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

fn read_machine_id() -> Vec<u8> {
    #[cfg(target_os = "linux")]
    {
        for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
            if let Ok(bytes) = std::fs::read(path) {
                if !bytes.is_empty() {
                    return bytes;
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(name) = std::env::var("COMPUTERNAME") {
            return name.into_bytes();
        }
    }
    // Last-ditch fallback: hash of HOME path (or USERPROFILE if HOME missing).
    if let Ok(home) = std::env::var("HOME") {
        return home.into_bytes();
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        return userprofile.into_bytes();
    }
    // Deterministic-but-low-entropy fallback -- at least the same value
    // across runs on the same host.
    b"ichr-no-machine-id".to_vec()
}

// ============================================================================
// Metadata tier -- Account records (plain JSON).
// ============================================================================

/// Atomically write the account list to `accounts.json` (non-secret).
#[tracing::instrument(name = "save_accounts", skip_all, fields(count = accounts.len()))]
pub async fn save_accounts(config: &StoreConfig, accounts: &[Account]) -> Result<(), AuthError> {
    let path = &config.accounts_json_path;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AuthError::StorageIo(format!("create_dir_all: {e}")))?;
    }
    let bytes = serde_json::to_vec_pretty(accounts)
        .map_err(|e| AuthError::StorageIo(format!("serialize: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &bytes)
        .await
        .map_err(|e| AuthError::StorageIo(format!("write tmp: {e}")))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| AuthError::StorageIo(format!("rename: {e}")))?;
    Ok(())
}

/// Load the account metadata list from `accounts.json`. Missing file → [].
#[tracing::instrument(name = "load_accounts", skip_all)]
pub async fn load_accounts(config: &StoreConfig) -> Result<Vec<Account>, AuthError> {
    let path = &config.accounts_json_path;
    if !tokio::fs::try_exists(path).await.unwrap_or(false) {
        return Ok(Vec::new());
    }
    let raw = tokio::fs::read(path)
        .await
        .map_err(|e| AuthError::StorageIo(format!("read {path:?}: {e}")))?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&raw).map_err(|e| AuthError::StorageIo(format!("parse {path:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(td: &TempDir) -> StoreConfig {
        StoreConfig {
            accounts_enc_path: td.path().join("accounts.enc"),
            accounts_json_path: td.path().join("accounts.json"),
            force_fallback: true,
        }
    }

    #[test]
    fn test_derive_machine_key_is_32_bytes() {
        let k = derive_machine_key().unwrap();
        assert_eq!(k.len(), 32);
    }

    #[test]
    fn test_derive_machine_key_deterministic() {
        let a = derive_machine_key().unwrap();
        let b = derive_machine_key().unwrap();
        assert_eq!(a, b, "derived key must be stable across calls");
    }

    #[tokio::test]
    async fn test_encrypted_file_roundtrip() {
        let td = TempDir::new().unwrap();
        let cfg = test_config(&td);
        store_refresh_token(&cfg, "acc-1", "secret-refresh-42")
            .await
            .unwrap();
        let got = load_refresh_token(&cfg, "acc-1").await.unwrap();
        assert_eq!(got, "secret-refresh-42");
    }

    #[tokio::test]
    async fn test_encrypted_bytes_not_plaintext() {
        let td = TempDir::new().unwrap();
        let cfg = test_config(&td);
        let secret = "UNIQUE-REFRESH-TOKEN-VALUE-12345";
        store_refresh_token(&cfg, "acc-1", secret).await.unwrap();
        let raw = std::fs::read(&cfg.accounts_enc_path).unwrap();
        let s = String::from_utf8_lossy(&raw);
        assert!(
            !s.contains(secret),
            "plaintext token found in encrypted file -- file contents: {s}"
        );
    }

    #[tokio::test]
    async fn test_multi_account_isolation() {
        let td = TempDir::new().unwrap();
        let cfg = test_config(&td);
        store_refresh_token(&cfg, "a", "token-a").await.unwrap();
        store_refresh_token(&cfg, "b", "token-b").await.unwrap();
        store_refresh_token(&cfg, "c", "token-c").await.unwrap();
        assert_eq!(load_refresh_token(&cfg, "a").await.unwrap(), "token-a");
        assert_eq!(load_refresh_token(&cfg, "b").await.unwrap(), "token-b");
        assert_eq!(load_refresh_token(&cfg, "c").await.unwrap(), "token-c");
    }

    #[tokio::test]
    async fn test_delete_returns_account_not_found_on_load() {
        let td = TempDir::new().unwrap();
        let cfg = test_config(&td);
        store_refresh_token(&cfg, "acc-1", "x").await.unwrap();
        delete_refresh_token(&cfg, "acc-1").await.unwrap();
        let err = load_refresh_token(&cfg, "acc-1").await.unwrap_err();
        assert!(matches!(err, AuthError::AccountNotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn test_delete_idempotent_on_absent() {
        let td = TempDir::new().unwrap();
        let cfg = test_config(&td);
        delete_refresh_token(&cfg, "nope").await.unwrap();
    }

    #[tokio::test]
    async fn test_load_refresh_token_account_not_found() {
        let td = TempDir::new().unwrap();
        let cfg = test_config(&td);
        let err = load_refresh_token(&cfg, "nope").await.unwrap_err();
        assert!(matches!(err, AuthError::AccountNotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn test_save_load_accounts_roundtrip() {
        let td = TempDir::new().unwrap();
        let cfg = test_config(&td);
        let acc1 = Account {
            id: "id-1".into(),
            mc_username: "PlayerOne".into(),
            mc_uuid: "c6bf8193-0000-4000-8000-00000000abcd".into(),
            mc_token_expires_at: 1,
            msa_token_expires_at: 2,
            added_at: 3,
            last_refreshed_at: 4,
            is_active: true,
            storage: StorageBackend::EncryptedFile,
        };
        save_accounts(&cfg, &[acc1.clone()]).await.unwrap();
        let loaded = load_accounts(&cfg).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "id-1");
        assert!(loaded[0].is_active);
    }

    #[tokio::test]
    async fn test_load_accounts_missing_file_returns_empty() {
        let td = TempDir::new().unwrap();
        let cfg = test_config(&td);
        let loaded = load_accounts(&cfg).await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_force_fallback_produces_encrypted_file_backend() {
        let td = TempDir::new().unwrap();
        let cfg = test_config(&td);
        let backend = store_refresh_token(&cfg, "acc-1", "tok").await.unwrap();
        assert!(matches!(backend, StorageBackend::EncryptedFile));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_accounts_enc_has_0600_perms() {
        use std::os::unix::fs::PermissionsExt;
        let td = TempDir::new().unwrap();
        let cfg = test_config(&td);
        store_refresh_token(&cfg, "acc-1", "t").await.unwrap();
        let perms = std::fs::metadata(&cfg.accounts_enc_path)
            .unwrap()
            .permissions();
        let mode = perms.mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0600 perms, got {mode:o}");
    }
}
