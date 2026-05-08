//! High-level Account facade -- composes device_code + chain + store for the TUI.
//!
//! The TUI calls exactly three operations on `AccountService`:
//!
//!   - `start_device_code_auth` (from AddAccount Effect): drives the full
//!     device-code → MSA-chain → persist flow, streaming progress events to
//!     the provided mpsc channel so the TUI can render the countdown modal.
//!   - `list_accounts` / `remove_account` / `activate_account`: read & mutate
//!     the persisted account list.
//!   - `resolve_auth_context_for_launch` + `resolve_msa_tokens_for_launch`:
//!     used by the launcher (plan 04-08) at launch time to pick the active
//!     account and refresh its tokens.
//!
//! All stateful concerns live on disk (via `store.rs`). This struct holds
//! only cheap clones of reqwest + path config.

use std::time::SystemTime;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::auth::chain::{self, AuthChainConfig, AuthChainOutput};
use crate::auth::device_code::{self, DeviceCodeProgress, DeviceCodeStart};
use crate::auth::store::{self, StoreConfig};
use crate::auth::{Account, AuthContext, AuthError, MsaTokens, StorageBackend};
use crate::persistence::paths::AppPaths;

/// High-level events emitted during `start_device_code_auth` that the
/// TUI translates into `Action` variants.
#[derive(Debug, Clone)]
pub enum AccountAuthEvent {
    /// Device code acquired; show code + uri + countdown modal.
    Started {
        user_code: String,
        verification_uri: String,
        /// Seconds until the user_code expires.
        expires_in: u64,
    },
    /// Transition message for the chain stages (to update the modal status).
    Progress { stage: String },
}

/// Success return from `start_device_code_auth`.
#[derive(Debug, Clone)]
pub struct AccountAddOutcome {
    pub account: Account,
    pub storage: StorageBackend,
}

/// Account management facade. Cheap to clone (Arc-wrapped http client).
#[derive(Clone)]
pub struct AccountService {
    chain_config: AuthChainConfig,
    store_config: StoreConfig,
}

impl AccountService {
    /// Build from an `AppPaths` + a prebuilt `reqwest::Client`. The
    /// client should use rustls-tls (per project constraint) and carry
    /// the ichr User-Agent.
    pub fn new(paths: &AppPaths, http: reqwest::Client) -> Self {
        Self {
            chain_config: AuthChainConfig::production(http),
            store_config: StoreConfig::from_paths(paths),
        }
    }

    /// Test constructor -- inject configs directly.
    pub fn new_with_config(chain_config: AuthChainConfig, store_config: StoreConfig) -> Self {
        Self {
            chain_config,
            store_config,
        }
    }

    // ================================================================
    // Add-account flow (AUTH-01, AUTH-02).
    // ================================================================

    /// Drive the full add-account flow. Emits `AccountAuthEvent::Started`
    /// (so the TUI can render the device-code modal) then
    /// `AccountAuthEvent::Progress` messages for each chain step.
    ///
    /// Respects `cancel_token`: cancellation in the polling window
    /// results in `AuthError::UserCancelled`.
    #[tracing::instrument(name = "add_account", skip_all)]
    pub async fn start_device_code_auth(
        &self,
        cancel_token: CancellationToken,
        event_tx: mpsc::Sender<AccountAuthEvent>,
    ) -> Result<AccountAddOutcome, AuthError> {
        // Step 1: device-code request
        let start: DeviceCodeStart = device_code::request_device_code(
            &self.chain_config.http,
            &self.chain_config.msa_base_url,
        )
        .await?;

        let _ = event_tx
            .send(AccountAuthEvent::Started {
                user_code: start.user_code.clone(),
                verification_uri: start.verification_uri.clone(),
                expires_in: start.expires_in,
            })
            .await;

        // Step 2: poll for token -- translate the lower-level progress
        // events into higher-level Progress events.
        let (dc_tx, mut dc_rx) = mpsc::channel::<DeviceCodeProgress>(16);
        let et_clone = event_tx.clone();
        let forward = tokio::spawn(async move {
            while let Some(ev) = dc_rx.recv().await {
                let stage = match ev {
                    DeviceCodeProgress::AuthorizationPending { .. } => {
                        "waiting for user".to_string()
                    }
                    DeviceCodeProgress::SlowDown { new_interval } => {
                        format!("slowing poll to {new_interval}s")
                    }
                    DeviceCodeProgress::Complete { .. } => "device code approved".to_string(),
                };
                let _ = et_clone.send(AccountAuthEvent::Progress { stage }).await;
            }
        });

        let tokens = device_code::poll_for_token(
            &self.chain_config.http,
            &self.chain_config.msa_base_url,
            &start.device_code,
            start.interval,
            cancel_token,
            dc_tx,
        )
        .await?;

        // Close the forwarder now that polling is done.
        forward.abort();

        let _ = event_tx
            .send(AccountAuthEvent::Progress {
                stage: "xbox live authenticate".into(),
            })
            .await;

        // Step 3–7: run the chain.
        let output: AuthChainOutput = chain::run_full_auth(
            &self.chain_config,
            &tokens.access_token,
            tokens.refresh_token,
            tokens.expires_in,
        )
        .await?;

        let _ = event_tx
            .send(AccountAuthEvent::Progress {
                stage: "saving account".into(),
            })
            .await;

        // Step 8: persist (refresh token + account metadata).
        let storage = store::store_refresh_token(
            &self.store_config,
            &output.account.id,
            &output.refresh_token,
        )
        .await?;

        let mut accounts = store::load_accounts(&self.store_config).await?;
        // If an account with this id already exists (re-auth / re-add),
        // replace it; otherwise push.
        let mut account_to_persist = output.account.clone();
        account_to_persist.storage = storage;
        // If this is the first account, activate it automatically.
        if accounts.is_empty() {
            account_to_persist.is_active = true;
        }
        if let Some(existing) = accounts.iter_mut().find(|a| a.id == account_to_persist.id) {
            account_to_persist.is_active = existing.is_active;
            *existing = account_to_persist.clone();
        } else {
            accounts.push(account_to_persist.clone());
        }
        store::save_accounts(&self.store_config, &accounts).await?;

        Ok(AccountAddOutcome {
            account: account_to_persist,
            storage,
        })
    }

    // ================================================================
    // Account management (AUTH-06).
    // ================================================================

    /// Read the account list from disk.
    #[tracing::instrument(name = "list_accounts", skip_all)]
    pub async fn list_accounts(&self) -> Result<Vec<Account>, AuthError> {
        store::load_accounts(&self.store_config).await
    }

    /// Remove account. Deletes both refresh token and metadata entry.
    /// If the removed account was `is_active`, no account is active
    /// afterward (caller should `activate_account` on a remaining id if desired).
    #[tracing::instrument(name = "remove_account", skip_all, fields(account_id = %account_id))]
    pub async fn remove_account(&self, account_id: &str) -> Result<(), AuthError> {
        store::delete_refresh_token(&self.store_config, account_id).await?;
        let accounts = store::load_accounts(&self.store_config).await?;
        let filtered: Vec<Account> = accounts
            .into_iter()
            .filter(|a| a.id != account_id)
            .collect();
        store::save_accounts(&self.store_config, &filtered).await?;
        Ok(())
    }

    /// Mark `account_id` as `is_active: true`, all others `false`.
    #[tracing::instrument(name = "activate_account", skip_all, fields(account_id = %account_id))]
    pub async fn activate_account(&self, account_id: &str) -> Result<(), AuthError> {
        let mut accounts = store::load_accounts(&self.store_config).await?;
        let mut found = false;
        for a in accounts.iter_mut() {
            if a.id == account_id {
                a.is_active = true;
                found = true;
            } else {
                a.is_active = false;
            }
        }
        if !found {
            return Err(AuthError::AccountNotFound(account_id.to_string()));
        }
        store::save_accounts(&self.store_config, &accounts).await?;
        Ok(())
    }

    // ================================================================
    // Launch-time integration (AUTH-03).
    // ================================================================

    /// Returns `AuthContext::Msa { account_id }` if any account is active;
    /// otherwise `AuthContext::Offline { username: default_username }`.
    /// This is the exact logic `launcher::service::launch_instance` needs.
    #[tracing::instrument(name = "resolve_auth_context_for_launch", skip_all)]
    pub async fn resolve_auth_context_for_launch(
        &self,
        default_username: &str,
    ) -> Result<AuthContext, AuthError> {
        let accounts = store::load_accounts(&self.store_config).await?;
        if let Some(active) = accounts.iter().find(|a| a.is_active) {
            Ok(AuthContext::Msa {
                account_id: active.id.clone(),
            })
        } else {
            Ok(AuthContext::Offline {
                username: default_username.to_string(),
            })
        }
    }

    /// Resolve the current MsaTokens for the given account_id.
    /// Always refreshes via `chain::ensure_valid_mc_token` -- MC tokens expire
    /// in 24h so a fresh chain walk at each launch is the simplest correct policy.
    ///
    /// Updates the persisted refresh_token + account expiry timestamps
    /// before returning.
    #[tracing::instrument(name = "resolve_msa_tokens_for_launch", skip_all, fields(account_id = %account_id))]
    pub async fn resolve_msa_tokens_for_launch(
        &self,
        account_id: &str,
    ) -> Result<MsaTokens, AuthError> {
        let refresh_token = store::load_refresh_token(&self.store_config, account_id).await?;
        let output = chain::ensure_valid_mc_token(&self.chain_config, &refresh_token).await?;

        // Persist the rotated refresh_token.
        let _ = store::store_refresh_token(
            &self.store_config,
            &output.account.id,
            &output.refresh_token,
        )
        .await?;
        // Update persisted Account timestamps.
        let mut accounts = store::load_accounts(&self.store_config).await?;
        if let Some(existing) = accounts.iter_mut().find(|a| a.id == output.account.id) {
            existing.mc_token_expires_at = output.mc_token_expires_at;
            existing.msa_token_expires_at = output.msa_token_expires_at;
            existing.last_refreshed_at = Account::to_unix(SystemTime::now());
            existing.mc_username = output.account.mc_username.clone();
            existing.mc_uuid = output.account.mc_uuid.clone();
        }
        store::save_accounts(&self.store_config, &accounts).await?;

        Ok(output.tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use tempfile::TempDir;

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
    }

    fn store_cfg(td: &TempDir) -> StoreConfig {
        StoreConfig {
            accounts_enc_path: td.path().join("accounts.enc"),
            accounts_json_path: td.path().join("accounts.json"),
            force_fallback: true,
        }
    }

    #[tokio::test]
    async fn test_list_accounts_empty() {
        let td = TempDir::new().unwrap();
        let chain_cfg = AuthChainConfig::single_host(http_client(), "http://x");
        let svc = AccountService::new_with_config(chain_cfg, store_cfg(&td));
        let got = svc.list_accounts().await.unwrap();
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn test_activate_account_not_found() {
        let td = TempDir::new().unwrap();
        let chain_cfg = AuthChainConfig::single_host(http_client(), "http://x");
        let svc = AccountService::new_with_config(chain_cfg, store_cfg(&td));
        let err = svc.activate_account("missing").await.unwrap_err();
        assert!(matches!(err, AuthError::AccountNotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn test_remove_account_idempotent_on_absent() {
        let td = TempDir::new().unwrap();
        let chain_cfg = AuthChainConfig::single_host(http_client(), "http://x");
        let svc = AccountService::new_with_config(chain_cfg, store_cfg(&td));
        svc.remove_account("nope").await.unwrap();
    }

    #[tokio::test]
    async fn test_resolve_auth_context_no_active_returns_offline() {
        let td = TempDir::new().unwrap();
        let chain_cfg = AuthChainConfig::single_host(http_client(), "http://x");
        let svc = AccountService::new_with_config(chain_cfg, store_cfg(&td));
        let got = svc
            .resolve_auth_context_for_launch("fallback-name")
            .await
            .unwrap();
        match got {
            AuthContext::Offline { username } => assert_eq!(username, "fallback-name"),
            other => panic!("expected Offline, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_resolve_auth_context_with_active_returns_msa() {
        let td = TempDir::new().unwrap();
        let cfg = store_cfg(&td);
        let chain_cfg = AuthChainConfig::single_host(http_client(), "http://x");
        let svc = AccountService::new_with_config(chain_cfg, cfg.clone());

        // Seed accounts.json with two accounts, second active.
        let a1 = Account {
            id: "id-A".into(),
            mc_username: "A".into(),
            mc_uuid: "11111111-1111-4111-8111-111111111111".into(),
            mc_token_expires_at: 0,
            msa_token_expires_at: 0,
            added_at: 0,
            last_refreshed_at: 0,
            is_active: false,
            storage: StorageBackend::EncryptedFile,
        };
        let a2 = Account {
            id: "id-B".into(),
            mc_username: "B".into(),
            mc_uuid: "22222222-2222-4222-8222-222222222222".into(),
            mc_token_expires_at: 0,
            msa_token_expires_at: 0,
            added_at: 0,
            last_refreshed_at: 0,
            is_active: true,
            storage: StorageBackend::EncryptedFile,
        };
        store::save_accounts(&cfg, &[a1, a2]).await.unwrap();

        let got = svc
            .resolve_auth_context_for_launch("fallback")
            .await
            .unwrap();
        match got {
            AuthContext::Msa { account_id } => assert_eq!(account_id, "id-B"),
            other => panic!("expected Msa, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_activate_account_sets_exclusive_active() {
        let td = TempDir::new().unwrap();
        let cfg = store_cfg(&td);
        let chain_cfg = AuthChainConfig::single_host(http_client(), "http://x");
        let svc = AccountService::new_with_config(chain_cfg, cfg.clone());
        let base = Account {
            id: "X".into(),
            mc_username: "X".into(),
            mc_uuid: "33333333-3333-4333-8333-333333333333".into(),
            mc_token_expires_at: 0,
            msa_token_expires_at: 0,
            added_at: 0,
            last_refreshed_at: 0,
            is_active: true,
            storage: StorageBackend::EncryptedFile,
        };
        let other = Account {
            id: "Y".into(),
            mc_username: "Y".into(),
            mc_uuid: "44444444-4444-4444-8444-444444444444".into(),
            mc_token_expires_at: 0,
            msa_token_expires_at: 0,
            added_at: 0,
            last_refreshed_at: 0,
            is_active: false,
            storage: StorageBackend::EncryptedFile,
        };
        store::save_accounts(&cfg, &[base, other]).await.unwrap();
        svc.activate_account("Y").await.unwrap();
        let got = svc.list_accounts().await.unwrap();
        assert!(got.iter().find(|a| a.id == "Y").unwrap().is_active);
        assert!(!got.iter().find(|a| a.id == "X").unwrap().is_active);
    }

    /// End-to-end: mocked 7 endpoints + tempdir store, verify AccountAddOutcome.
    #[tokio::test]
    async fn test_start_device_code_auth_end_to_end() {
        let server = MockServer::start_async().await;
        // 1. devicecode
        server
            .mock_async(|when, then| {
                when.method(POST).path("/consumers/oauth2/v2.0/devicecode");
                then.status(200).body(
                    r#"{"user_code":"ABCD","verification_uri":"https://ms/link","device_code":"dc","expires_in":900,"interval":1,"message":"m"}"#,
                );
            })
            .await;
        // 2. token (immediate success)
        server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/consumers/oauth2/v2.0/token")
                    .body_includes("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code");
                then.status(200).body(
                    r#"{"token_type":"Bearer","scope":"XboxLive.signin offline_access","expires_in":3600,"access_token":"msa-acc","refresh_token":"msa-ref"}"#,
                );
            })
            .await;
        // 3. XBL
        server
            .mock_async(|when, then| {
                when.method(POST).path("/user/authenticate");
                then.status(200)
                    .body(r#"{"Token":"xbl","DisplayClaims":{"xui":[{"uhs":"uhs-1"}]}}"#);
            })
            .await;
        // 4. XSTS
        server
            .mock_async(|when, then| {
                when.method(POST).path("/xsts/authorize");
                then.status(200)
                    .body(r#"{"Token":"xsts","DisplayClaims":{"xui":[{"uhs":"uhs-1"}]}}"#);
            })
            .await;
        // 5. MC login
        server
            .mock_async(|when, then| {
                when.method(POST).path("/authentication/login_with_xbox");
                then.status(200).body(
                    r#"{"username":"u","access_token":"mc-tok","expires_in":86400,"token_type":"Bearer"}"#,
                );
            })
            .await;
        // 6. Entitlement
        server
            .mock_async(|when, then| {
                when.method(GET).path("/entitlements/mcstore");
                then.status(200).body(
                    r#"{"items":[{"name":"product_minecraft","signature":"s"},{"name":"game_minecraft","signature":"s"}],"signature":"s"}"#,
                );
            })
            .await;
        // 7. Profile
        server
            .mock_async(|when, then| {
                when.method(GET).path("/minecraft/profile");
                then.status(200)
                    .body(r#"{"id":"c6bf819300004000800000000000abcd","name":"PlayerOne"}"#);
            })
            .await;

        let td = TempDir::new().unwrap();
        let chain_cfg = AuthChainConfig::single_host(http_client(), &server.base_url());
        let svc = AccountService::new_with_config(chain_cfg, store_cfg(&td));

        let (tx, mut rx) = mpsc::channel::<AccountAuthEvent>(32);
        let token = CancellationToken::new();
        let out = svc.start_device_code_auth(token, tx).await.unwrap();

        assert_eq!(out.account.mc_username, "PlayerOne");
        assert_eq!(out.account.mc_uuid, "c6bf8193-0000-4000-8000-00000000abcd");
        assert!(
            out.account.is_active,
            "first added account should auto-activate"
        );

        // Drain events -- at least one Started + >= 1 Progress.
        let mut saw_started = false;
        let mut saw_progress = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                AccountAuthEvent::Started {
                    user_code,
                    verification_uri,
                    ..
                } => {
                    assert_eq!(user_code, "ABCD");
                    assert_eq!(verification_uri, "https://ms/link");
                    saw_started = true;
                }
                AccountAuthEvent::Progress { .. } => saw_progress = true,
            }
        }
        assert!(saw_started, "expected AccountAuthEvent::Started");
        assert!(
            saw_progress,
            "expected at least one AccountAuthEvent::Progress"
        );

        // Verify persistence.
        let listed = svc.list_accounts().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "c6bf819300004000800000000000abcd");
    }

    #[tokio::test]
    async fn test_remove_account_deletes_metadata_and_token() {
        let td = TempDir::new().unwrap();
        let cfg = store_cfg(&td);
        let chain_cfg = AuthChainConfig::single_host(http_client(), "http://x");
        let svc = AccountService::new_with_config(chain_cfg, cfg.clone());
        // Seed.
        let acc = Account {
            id: "R".into(),
            mc_username: "R".into(),
            mc_uuid: "55555555-5555-4555-8555-555555555555".into(),
            mc_token_expires_at: 0,
            msa_token_expires_at: 0,
            added_at: 0,
            last_refreshed_at: 0,
            is_active: true,
            storage: StorageBackend::EncryptedFile,
        };
        store::save_accounts(&cfg, &[acc]).await.unwrap();
        store::store_refresh_token(&cfg, "R", "rt").await.unwrap();
        svc.remove_account("R").await.unwrap();
        assert!(svc.list_accounts().await.unwrap().is_empty());
        let err = store::load_refresh_token(&cfg, "R").await.unwrap_err();
        assert!(matches!(err, AuthError::AccountNotFound(_)));
    }
}
