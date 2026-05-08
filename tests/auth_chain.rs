//! End-to-end integration test for the Phase 4 auth chain.
//!
//! Uses httpmock to simulate every Microsoft endpoint against a single
//! MockServer host (all paths disambiguated by URL path). Runs the full
//! device-code → MSA chain → persistence cycle and asserts the resulting
//! Account + MsaTokens are correctly populated.

use std::time::Duration;

use httpmock::prelude::*;
use mineltui::auth::chain::AuthChainConfig;
use mineltui::auth::service::{AccountAuthEvent, AccountService};
use mineltui::auth::store::StoreConfig;
use mineltui::auth::AuthContext;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

async fn register_all_mocks(server: &MockServer) {
    // 1. devicecode
    server
        .mock_async(|when, then| {
            when.method(POST).path("/consumers/oauth2/v2.0/devicecode");
            then.status(200).body(
                r#"{"user_code":"WXYZ-0000","verification_uri":"https://microsoft.com/link","device_code":"dc","expires_in":900,"interval":1,"message":"m"}"#,
            );
        })
        .await;
    // 2. token (device-code success)
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
    // 2b. token (refresh) — used by resolve_msa_tokens_for_launch
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/consumers/oauth2/v2.0/token")
                .body_includes("grant_type=refresh_token");
            then.status(200).body(
                r#"{"token_type":"Bearer","scope":"XboxLive.signin offline_access","expires_in":3600,"access_token":"msa-acc-2","refresh_token":"msa-ref-2"}"#,
            );
        })
        .await;
    // 3. XBL
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/user/authenticate")
                .body_includes("\"RpsTicket\":\"d=");
            then.status(200)
                .body(r#"{"Token":"xbl","DisplayClaims":{"xui":[{"uhs":"uhs-42"}]}}"#);
        })
        .await;
    // 4. XSTS
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/xsts/authorize")
                .body_includes("\"SandboxId\":\"RETAIL\"");
            then.status(200)
                .body(r#"{"Token":"xsts","DisplayClaims":{"xui":[{"uhs":"uhs-42"}]}}"#);
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
}

fn store(td: &TempDir) -> StoreConfig {
    StoreConfig {
        accounts_enc_path: td.path().join("accounts.enc"),
        accounts_json_path: td.path().join("accounts.json"),
        force_fallback: true,
    }
}

#[tokio::test]
async fn test_auth_chain_end_to_end_add_then_resolve_for_launch() {
    let server = MockServer::start_async().await;
    register_all_mocks(&server).await;
    let td = TempDir::new().unwrap();
    let svc = AccountService::new_with_config(
        AuthChainConfig::single_host(http(), &server.base_url()),
        store(&td),
    );

    // 1) add account via device-code
    let (tx, mut rx) = mpsc::channel::<AccountAuthEvent>(32);
    let token = CancellationToken::new();
    let out = svc.start_device_code_auth(token, tx).await.unwrap();
    assert_eq!(out.account.mc_username, "PlayerOne");
    assert_eq!(out.account.mc_uuid, "c6bf8193-0000-4000-8000-00000000abcd");
    assert!(out.account.is_active); // first added auto-activates

    // 2) verify events
    let mut saw_started = false;
    while let Ok(ev) = rx.try_recv() {
        if let AccountAuthEvent::Started { user_code, .. } = ev {
            assert_eq!(user_code, "WXYZ-0000");
            saw_started = true;
        }
    }
    assert!(saw_started);

    // 3) list shows the account
    let list = svc.list_accounts().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "c6bf819300004000800000000000abcd");
    assert!(list[0].is_active);

    // 4) resolve auth context for launch → Msa
    let ctx = svc
        .resolve_auth_context_for_launch("fallback")
        .await
        .unwrap();
    match ctx {
        AuthContext::Msa { account_id } => {
            assert_eq!(account_id, "c6bf819300004000800000000000abcd");
        }
        other => panic!("expected Msa, got {other:?}"),
    }

    // 5) resolve MsaTokens for launch (exercises refresh path)
    let tokens = svc
        .resolve_msa_tokens_for_launch("c6bf819300004000800000000000abcd")
        .await
        .unwrap();
    assert_eq!(tokens.mc_access_token, "mc-tok");
    assert_eq!(tokens.mc_uuid, "c6bf8193-0000-4000-8000-00000000abcd");
    assert_eq!(tokens.mc_player_name, "PlayerOne");
    assert_eq!(tokens.xuid, "uhs-42");
    assert_eq!(tokens.user_hash, "uhs-42");
    assert_eq!(tokens.user_type, "msa");
}

#[tokio::test]
async fn test_auth_chain_xsts_error_surfaces_readable_message() {
    let server = MockServer::start_async().await;
    // devicecode + token
    server
        .mock_async(|when, then| {
            when.method(POST).path("/consumers/oauth2/v2.0/devicecode");
            then.status(200).body(
                r#"{"user_code":"X","verification_uri":"u","device_code":"dc","expires_in":900,"interval":1}"#,
            );
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/consumers/oauth2/v2.0/token");
            then.status(200).body(
                r#"{"token_type":"Bearer","scope":"XboxLive.signin offline_access","expires_in":3600,"access_token":"a","refresh_token":"r"}"#,
            );
        })
        .await;
    // XBL ok
    server
        .mock_async(|when, then| {
            when.method(POST).path("/user/authenticate");
            then.status(200)
                .body(r#"{"Token":"x","DisplayClaims":{"xui":[{"uhs":"u"}]}}"#);
        })
        .await;
    // XSTS 401 — no Xbox profile
    server
        .mock_async(|when, then| {
            when.method(POST).path("/xsts/authorize");
            then.status(401).body(
                r#"{"Identity":"0","XErr":2148916233,"Message":"no profile","Redirect":"r"}"#,
            );
        })
        .await;
    let td = TempDir::new().unwrap();
    let svc = AccountService::new_with_config(
        AuthChainConfig::single_host(http(), &server.base_url()),
        store(&td),
    );
    let (tx, _rx) = mpsc::channel::<AccountAuthEvent>(32);
    let err = svc
        .start_device_code_auth(CancellationToken::new(), tx)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("xbox profile"),
        "expected human-readable XSTS message; got: {msg}"
    );
}
