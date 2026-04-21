//! End-to-end auth chain orchestrator.
//!
//! Two entry points (called by `src/auth/service.rs` in plan 04-07):
//!
//! 1. `run_full_auth` — after device-code completes, walk XBL → XSTS →
//!    MC login → entitlement → profile and produce an `Account`
//!    + `MsaTokens` snapshot.
//!
//! 2. `ensure_valid_mc_token` — called at each launch. Always runs
//!    the refresh path (refresh_token → new MSA access_token → re-run
//!    XBL/XSTS/MC) because Minecraft access tokens expire in 24h and
//!    we cannot know the wall-clock precisely enough to skip refresh
//!    when it's fresh. Returns the new refresh_token alongside the
//!    MsaTokens so the caller can persist it.
//!
//! No persistence here; chain.rs is pure "walk the endpoints".
//!
//! Pitfalls: token values never appear in `tracing::` macro field args
//! (pitfall 16); `#[tracing::instrument(skip_all)]` on both entry points.

use std::time::SystemTime;

use crate::auth::{
    device_code::{self, MSA_BASE_URL},
    mc_services::{self, McLoginResponse, McProfile},
    xbox::{self, XstsTokens},
};
use crate::auth::{Account, AuthError, MsaTokens, StorageBackend};

/// Configuration for the auth chain. In production, all base URLs use
/// the production constants. In tests, each is overridden to a single
/// `MockServer::base_url()` shared across all endpoints (path routing
/// disambiguates).
#[derive(Debug, Clone)]
pub struct AuthChainConfig {
    /// `reqwest::Client` with TLS + User-Agent configured.
    pub http: reqwest::Client,
    /// Base URL for MSA OAuth endpoints (devicecode, token).
    pub msa_base_url: String,
    /// Base URL for Xbox Live (`/user/authenticate`).
    pub xbl_base_url: String,
    /// Base URL for XSTS (`/xsts/authorize`).
    pub xsts_base_url: String,
    /// Base URL for Minecraft services (login, entitlement, profile).
    pub mc_base_url: String,
}

impl AuthChainConfig {
    /// Production configuration (real MS endpoints).
    pub fn production(http: reqwest::Client) -> Self {
        Self {
            http,
            msa_base_url: MSA_BASE_URL.to_string(),
            xbl_base_url: "https://user.auth.xboxlive.com".to_string(),
            xsts_base_url: "https://xsts.auth.xboxlive.com".to_string(),
            mc_base_url: "https://api.minecraftservices.com".to_string(),
        }
    }

    /// All four base URLs point at the same host (used by httpmock tests
    /// that register distinct paths on a single `MockServer`).
    pub fn single_host(http: reqwest::Client, base: &str) -> Self {
        Self {
            http,
            msa_base_url: base.to_string(),
            xbl_base_url: base.to_string(),
            xsts_base_url: base.to_string(),
            mc_base_url: base.to_string(),
        }
    }
}

/// Output of `run_full_auth`: a persistable Account record + a one-shot
/// MsaTokens snapshot + the MSA refresh_token (caller persists separately).
#[derive(Debug, Clone)]
pub struct AuthChainOutput {
    pub account: Account,
    pub tokens: MsaTokens,
    /// MSA refresh_token — passed to store.rs::save_refresh_token.
    pub refresh_token: String,
    /// MC access_token unix expiry (typically now + 86400).
    pub mc_token_expires_at: i64,
    /// MSA access_token unix expiry (typically now + 3600).
    pub msa_token_expires_at: i64,
}

/// Walk the full chain starting from an MSA access_token (produced by
/// `device_code::poll_for_token`).
///
/// Steps executed:
///   3. Xbox Live authenticate (`xbox::authenticate_xbox_live`)
///   4. XSTS authorize (`xbox::authenticate_xsts`) — 401 paths already
///      mapped to `AuthError::XstsDenied` inside xbox.rs.
///   5. MC services login_with_xbox
///   6. Entitlement check (empty items => `AuthError::NoMinecraftLicense`)
///   7. Profile fetch (32-char hex -> hyphenated UUID)
#[tracing::instrument(name = "run_full_auth", skip_all)]
pub async fn run_full_auth(
    config: &AuthChainConfig,
    msa_access_token: &str,
    msa_refresh_token: String,
    msa_expires_in_sec: i64,
) -> Result<AuthChainOutput, AuthError> {
    // Step 3: XBL authenticate
    let xbl = xbox::authenticate_xbox_live(
        &config.http,
        &config.xbl_base_url,
        msa_access_token,
    )
    .await?;

    // Step 4: XSTS authorize
    let xsts: XstsTokens =
        xbox::authenticate_xsts(&config.http, &config.xsts_base_url, &xbl.token).await?;

    // Step 5: MC login
    let mc: McLoginResponse = mc_services::login_with_xbox(
        &config.http,
        &config.mc_base_url,
        &xsts.user_hash,
        &xsts.token,
    )
    .await?;

    // Step 6: entitlement
    mc_services::check_entitlement(&config.http, &config.mc_base_url, &mc.access_token)
        .await?;

    // Step 7: profile
    let profile: McProfile =
        mc_services::fetch_profile(&config.http, &config.mc_base_url, &mc.access_token)
            .await?;

    let mc_uuid = mc_services::format_uuid(&profile.id)?;
    let now = Account::to_unix(SystemTime::now());
    let mc_expires = now + mc.expires_in;
    let msa_expires = now + msa_expires_in_sec;

    let account = Account {
        id: profile.id.clone(),
        mc_username: profile.name.clone(),
        mc_uuid: mc_uuid.clone(),
        mc_token_expires_at: mc_expires,
        msa_token_expires_at: msa_expires,
        added_at: now,
        last_refreshed_at: now,
        is_active: false,
        // storage is filled in by store.rs after actually persisting.
        storage: StorageBackend::EncryptedFile,
    };

    let tokens = MsaTokens {
        mc_access_token: mc.access_token,
        mc_uuid,
        mc_player_name: profile.name,
        xuid: xsts.user_hash.clone(),
        user_hash: xsts.user_hash,
        user_type: "msa".to_string(),
    };

    tracing::info!(
        account_id = %account.id,
        mc_username = %account.mc_username,
        mc_expires_at = account.mc_token_expires_at,
        "auth chain completed"
    );

    Ok(AuthChainOutput {
        account,
        tokens,
        refresh_token: msa_refresh_token,
        mc_token_expires_at: mc_expires,
        msa_token_expires_at: msa_expires,
    })
}

/// Refresh path: exchange the stored MSA refresh_token for new MSA
/// tokens, then re-run the XBL/XSTS/MC chain. Returns the new chain
/// output (including a fresh refresh_token — Microsoft rotates them).
///
/// Called at each launch. On `AuthError::RefreshFailed`, caller must
/// clear the stored account and prompt for re-auth (AUTH-03 behavior).
///
/// Decision: always runs the full refresh path regardless of token age
/// because Minecraft access tokens expire in 24h and we cannot know
/// the wall-clock elapsed time precisely enough to safely skip refresh.
#[tracing::instrument(name = "ensure_valid_mc_token", skip_all)]
pub async fn ensure_valid_mc_token(
    config: &AuthChainConfig,
    refresh_token: &str,
) -> Result<AuthChainOutput, AuthError> {
    let refreshed = device_code::refresh_access_token(
        &config.http,
        &config.msa_base_url,
        refresh_token,
    )
    .await?;
    // refresh returned a new MSA access_token and a (possibly rotated)
    // refresh_token. Re-run the chain.
    run_full_auth(
        config,
        &refreshed.access_token,
        refreshed.refresh_token,
        refreshed.expires_in,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
    }

    /// Register the five chain endpoints (XBL, XSTS, MC login, entitlement,
    /// profile) on a single MockServer.
    async fn setup_happy_path(server: &MockServer) {
        // XBL
        server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/user/authenticate")
                    .body_includes("\"RpsTicket\":\"d=");
                then.status(200).body(
                    r#"{"Token":"xbl-tok","DisplayClaims":{"xui":[{"uhs":"uhs-1"}]}}"#,
                );
            })
            .await;
        // XSTS
        server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/xsts/authorize")
                    .body_includes("\"SandboxId\":\"RETAIL\"");
                then.status(200).body(
                    r#"{"Token":"xsts-tok","DisplayClaims":{"xui":[{"uhs":"uhs-1"}]}}"#,
                );
            })
            .await;
        // MC login
        server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/authentication/login_with_xbox")
                    .body_includes("XBL3.0 x=uhs-1;xsts-tok");
                then.status(200).body(
                    r#"{"username":"u","access_token":"mc-tok","expires_in":86400,"token_type":"Bearer"}"#,
                );
            })
            .await;
        // Entitlement — both product_minecraft and game_minecraft required
        server
            .mock_async(|when, then| {
                when.method(GET).path("/entitlements/mcstore");
                then.status(200).body(
                    r#"{"items":[{"name":"product_minecraft","signature":"s"},{"name":"game_minecraft","signature":"s"}],"signature":"s"}"#,
                );
            })
            .await;
        // Profile
        server
            .mock_async(|when, then| {
                when.method(GET).path("/minecraft/profile");
                then.status(200).body(
                    r#"{"id":"c6bf819300004000800000000000abcd","name":"PlayerOne"}"#,
                );
            })
            .await;
    }

    #[tokio::test]
    async fn test_run_full_auth_happy_path() {
        let server = MockServer::start_async().await;
        setup_happy_path(&server).await;
        let cfg = AuthChainConfig::single_host(http_client(), &server.base_url());
        let out = run_full_auth(&cfg, "msa-acc", "msa-ref".to_string(), 3600)
            .await
            .unwrap();
        assert_eq!(out.account.mc_username, "PlayerOne");
        assert_eq!(out.account.mc_uuid, "c6bf8193-0000-4000-8000-00000000abcd");
        assert_eq!(out.tokens.mc_access_token, "mc-tok");
        assert_eq!(out.tokens.user_type, "msa");
        assert_eq!(out.tokens.xuid, "uhs-1");
        assert_eq!(out.tokens.user_hash, "uhs-1");
        assert_eq!(out.refresh_token, "msa-ref");
        // Timestamps are sane (non-zero, within 30s of now).
        let now = Account::to_unix(SystemTime::now());
        assert!((out.account.added_at - now).abs() < 30);
        // MC expiry = now + 86400
        assert!((out.mc_token_expires_at - (now + 86400)).abs() < 30);
        assert!((out.msa_token_expires_at - (now + 3600)).abs() < 30);
    }

    #[tokio::test]
    async fn test_run_full_auth_no_entitlement() {
        let server = MockServer::start_async().await;
        // XBL
        server
            .mock_async(|when, then| {
                when.method(POST).path("/user/authenticate");
                then.status(200).body(
                    r#"{"Token":"xbl","DisplayClaims":{"xui":[{"uhs":"u"}]}}"#,
                );
            })
            .await;
        // XSTS
        server
            .mock_async(|when, then| {
                when.method(POST).path("/xsts/authorize");
                then.status(200).body(
                    r#"{"Token":"xsts","DisplayClaims":{"xui":[{"uhs":"u"}]}}"#,
                );
            })
            .await;
        // MC login
        server
            .mock_async(|when, then| {
                when.method(POST).path("/authentication/login_with_xbox");
                then.status(200).body(
                    r#"{"username":"u","access_token":"mc-tok","expires_in":86400,"token_type":"Bearer"}"#,
                );
            })
            .await;
        // Entitlement empty
        server
            .mock_async(|when, then| {
                when.method(GET).path("/entitlements/mcstore");
                then.status(200).body(r#"{"items":[],"signature":""}"#);
            })
            .await;
        let cfg = AuthChainConfig::single_host(http_client(), &server.base_url());
        let err = run_full_auth(&cfg, "msa-acc", "r".into(), 3600)
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::NoMinecraftLicense), "got {err:?}");
    }

    #[tokio::test]
    async fn test_run_full_auth_xsts_denied_propagates_xerr() {
        let server = MockServer::start_async().await;
        // XBL
        server
            .mock_async(|when, then| {
                when.method(POST).path("/user/authenticate");
                then.status(200).body(
                    r#"{"Token":"xbl","DisplayClaims":{"xui":[{"uhs":"u"}]}}"#,
                );
            })
            .await;
        // XSTS 401 with XErr
        server
            .mock_async(|when, then| {
                when.method(POST).path("/xsts/authorize");
                then.status(401).body(
                    r#"{"Identity":"0","XErr":2148916233,"Message":"no xbox profile","Redirect":"r"}"#,
                );
            })
            .await;
        let cfg = AuthChainConfig::single_host(http_client(), &server.base_url());
        let err = run_full_auth(&cfg, "msa-acc", "r".into(), 3600)
            .await
            .unwrap_err();
        match err {
            AuthError::XstsDenied { xerr, message } => {
                assert_eq!(xerr, 2148916233);
                assert!(message.to_lowercase().contains("xbox profile"));
            }
            other => panic!("expected XstsDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_ensure_valid_mc_token_refresh_path() {
        let server = MockServer::start_async().await;
        // Refresh MSA token
        server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/consumers/oauth2/v2.0/token")
                    .body_includes("grant_type=refresh_token")
                    .body_includes("refresh_token=old-ref");
                then.status(200).body(
                    r#"{"token_type":"Bearer","scope":"XboxLive.signin offline_access","expires_in":3600,"access_token":"new-msa-acc","refresh_token":"new-ref"}"#,
                );
            })
            .await;
        // Full chain
        setup_happy_path(&server).await;
        let cfg = AuthChainConfig::single_host(http_client(), &server.base_url());
        let out = ensure_valid_mc_token(&cfg, "old-ref").await.unwrap();
        assert_eq!(out.refresh_token, "new-ref");
        assert_eq!(out.tokens.mc_access_token, "mc-tok");
        assert_eq!(out.account.mc_username, "PlayerOne");
    }

    #[tokio::test]
    async fn test_ensure_valid_mc_token_refresh_revoked() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/consumers/oauth2/v2.0/token")
                    .body_includes("grant_type=refresh_token");
                then.status(400).body(
                    r#"{"error":"invalid_grant","error_description":"token revoked"}"#,
                );
            })
            .await;
        let cfg = AuthChainConfig::single_host(http_client(), &server.base_url());
        let err = ensure_valid_mc_token(&cfg, "revoked-ref").await.unwrap_err();
        assert!(matches!(err, AuthError::RefreshFailed), "got {err:?}");
    }
}
