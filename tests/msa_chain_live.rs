//! Live MSA auth smoke test — REAL Microsoft endpoints, REAL account.
//!
//! Disabled by default (`#[ignore]`). Run before Phase 4 release with:
//!
//!     cargo test --test msa_chain_live -- --ignored --nocapture
//!
//! Requires: an internet-connected machine, a real Microsoft account,
//! and a browser to paste the device code into. Uses a tempdir so the
//! real `~/.config/mineltui/accounts.*` files are NOT touched.
//!
//! Phase 4 sign-off on `04-VALIDATION.md` requires a successful run of
//! this test (see VALIDATION.md Manual-Only Verifications).

use std::time::Duration;

use mineltui::auth::chain::AuthChainConfig;
use mineltui::auth::service::{AccountAuthEvent, AccountService};
use mineltui::auth::store::StoreConfig;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires real MS account + browser + internet — see module docs"]
async fn live_msa_round_trip_prints_code_and_expects_sign_in() {
    let http = reqwest::Client::builder()
        .user_agent("mineltui-live-test/0.1")
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let td = TempDir::new().unwrap();
    let svc = AccountService::new_with_config(
        AuthChainConfig::production(http),
        StoreConfig {
            accounts_enc_path: td.path().join("accounts.enc"),
            accounts_json_path: td.path().join("accounts.json"),
            force_fallback: true,
        },
    );

    let (tx, mut rx) = mpsc::channel::<AccountAuthEvent>(32);
    let token = CancellationToken::new();

    // Forward progress events to stdout so the operator can follow along.
    let logger = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                AccountAuthEvent::Started { user_code, verification_uri, expires_in } => {
                    println!("\n=============================================");
                    println!(">> Visit: {verification_uri}");
                    println!(">> Enter code: {user_code}");
                    println!(">> Code expires in {expires_in}s");
                    println!("=============================================\n");
                }
                AccountAuthEvent::Progress { stage } => {
                    println!("  ... {stage}");
                }
            }
        }
    });

    let out = svc
        .start_device_code_auth(token, tx)
        .await
        .expect("live auth should succeed — check console for device code prompt");
    logger.abort();

    println!("Signed in as {} ({})", out.account.mc_username, out.account.mc_uuid);
    assert!(!out.account.mc_username.is_empty());
    assert!(!out.account.mc_uuid.is_empty());
    // mc_uuid should be 36-char hyphenated form
    assert_eq!(out.account.mc_uuid.len(), 36);
    assert_eq!(out.account.mc_uuid.matches('-').count(), 4);
}
