//! Microsoft OAuth 2.0 Device Code flow (RFC 8628).
//!
//! Hand-rolled against reqwest 0.13 to preserve control of the polling
//! loop and base-URL override (oauth2 5.0 adds indirection that fights
//! the httpmock test pattern used here).
//!
//! State machine (`poll_for_token`):
//!   1. sleep(interval) — or cancel immediately if token fired
//!   2. POST /consumers/oauth2/v2.0/token grant_type=device_code
//!   3. if 200 → `Complete { access_token, refresh_token, expires_in }`
//!   4. if 400:
//!      - authorization_pending → emit `AuthorizationPending`, continue
//!      - slow_down              → interval += 5 (RFC 8628 §3.5), emit `SlowDown`, continue
//!      - expired_token          → `Err(DeviceCodeExpired)`
//!      - access_denied          → `Err(DeviceCodeFailed("user denied access"))`
//!      - other                  → `Err(DeviceCodeFailed(oauth_error_description))`
//!   5. `tokio::select!` with cancel_token → `Err(UserCancelled)` on cancel
//!
//! Pitfalls enforced:
//!   - pitfall 3 (04-RESEARCH): slow_down MUST add 5s to interval (RFC 8628 §3.5)
//!   - pitfall 5 (PITFALLS.md): scope MUST include `offline_access` or no
//!     refresh_token is issued
//!   - pitfall 16: no raw token values in tracing macro field args
//!   - client_id default: legacy Mojang launcher ID; env override documented (A1)

use serde::Deserialize;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::auth::AuthError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default MSA client ID — the legacy Mojang public launcher app ID.
/// Conventional for third-party launchers (Prism, ATLauncher, GDLauncher).
/// Override via `MINELTUI_MSA_CLIENT_ID` to use your own Azure AD registration.
pub const DEFAULT_MSA_CLIENT_ID: &str = "00000000402b5328";

/// Environment variable name for client ID override.
pub const MSA_CLIENT_ID_ENV: &str = "MINELTUI_MSA_CLIENT_ID";

/// OAuth scope: XboxLive.signin to reach Xbox Live; offline_access to receive
/// a refresh_token (omitting offline_access means no refresh_token is issued).
pub const MSA_SCOPE: &str = "XboxLive.signin offline_access";

/// Production MSA consumer endpoint base URL.
pub const MSA_BASE_URL: &str = "https://login.microsoftonline.com";

/// Path for the device-code endpoint.
pub const DEVICE_CODE_PATH: &str = "/consumers/oauth2/v2.0/devicecode";

/// Path for the token endpoint (poll + refresh).
pub const TOKEN_PATH: &str = "/consumers/oauth2/v2.0/token";

/// grant_type value for the device-code polling POST.
pub const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Returns the MSA client ID, honoring the `MINELTUI_MSA_CLIENT_ID` env
/// override.  Default: `"00000000402b5328"` (legacy Mojang public ID;
/// conventional for third-party launchers).
pub fn client_id() -> String {
    std::env::var(MSA_CLIENT_ID_ENV).unwrap_or_else(|_| DEFAULT_MSA_CLIENT_ID.to_string())
}

/// Device-code flow start state — fields sent to the TUI for display.
#[derive(Debug, Clone)]
pub struct DeviceCodeStart {
    /// Short code to show the user (e.g., "ABCD-1234").
    pub user_code: String,
    /// URI to open in a browser (e.g., "https://microsoft.com/link").
    pub verification_uri: String,
    /// Opaque device code used to poll the token endpoint.
    pub device_code: String,
    /// Initial polling interval in seconds.
    pub interval: u64,
    /// Seconds until user_code expires.
    pub expires_in: u64,
}

/// Progress events emitted by the polling loop.
///
/// `Complete` is the terminal success state; errors bubble through the
/// `Result` return value of `poll_for_token`.
#[derive(Debug, Clone)]
pub enum DeviceCodeProgress {
    /// Authorization has not been granted yet; poller keeps waiting.
    AuthorizationPending { interval: u64 },
    /// Server asked the poller to slow down (RFC 8628 §3.5: +5s to interval).
    SlowDown { new_interval: u64 },
    /// Authorization granted — tokens ready.
    Complete {
        access_token: String,
        refresh_token: String,
        expires_in: i64,
    },
}

/// MSA token response shape (success path for both poll and refresh).
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds until the access_token expires. Typically 3600.
    pub expires_in: i64,
}

// ---------------------------------------------------------------------------
// Private types
// ---------------------------------------------------------------------------

/// Raw device-code endpoint response.
#[derive(Debug, Deserialize)]
struct DeviceCodeResponseRaw {
    user_code: String,
    verification_uri: String,
    device_code: String,
    expires_in: u64,
    interval: u64,
}

/// MSA OAuth error response (any 4xx from the token endpoint).
#[derive(Debug, Deserialize)]
struct OauthErrorResponse {
    error: String,
    #[serde(default)]
    error_description: String,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Step 1: request a device code.
///
/// The TUI displays the returned `user_code` and `verification_uri` in a
/// modal and starts a countdown from `expires_in`.
///
/// `base_url` should be `MSA_BASE_URL` in production.  Tests inject a
/// `httpmock` server URL here.
#[tracing::instrument(name = "msa_request_device_code", skip_all)]
pub async fn request_device_code(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<DeviceCodeStart, AuthError> {
    let url = format!("{}{DEVICE_CODE_PATH}", base_url.trim_end_matches('/'));
    let form = [
        ("client_id", client_id()),
        ("scope", MSA_SCOPE.to_string()),
    ];

    let resp = client
        .post(&url)
        .form(&form)
        .send()
        .await
        .map_err(|e| AuthError::Http(format!("devicecode POST: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AuthError::DeviceCodeRequest(format!(
            "HTTP {status}: {body}"
        )));
    }

    let parsed: DeviceCodeResponseRaw = resp
        .json()
        .await
        .map_err(|e| AuthError::MalformedResponse(format!("devicecode body: {e}")))?;

    Ok(DeviceCodeStart {
        user_code: parsed.user_code,
        verification_uri: parsed.verification_uri,
        device_code: parsed.device_code,
        interval: parsed.interval,
        expires_in: parsed.expires_in,
    })
}

/// Step 2: poll the token endpoint until success, expiry, denial, or
/// cancellation.
///
/// Emits `DeviceCodeProgress::AuthorizationPending` on each poll that gets
/// the pending error; emits `SlowDown` when the server asks for a slower
/// poll (interval += 5s per RFC 8628 §3.5).
///
/// Returns `Err(AuthError::UserCancelled)` when `cancel_token` fires.
///
/// `base_url` should be `MSA_BASE_URL` in production.
#[tracing::instrument(name = "msa_poll_for_token", skip_all)]
pub async fn poll_for_token(
    client: &reqwest::Client,
    base_url: &str,
    device_code: &str,
    initial_interval: u64,
    cancel_token: CancellationToken,
    event_tx: mpsc::Sender<DeviceCodeProgress>,
) -> Result<TokenResponse, AuthError> {
    let url = format!("{}{TOKEN_PATH}", base_url.trim_end_matches('/'));
    let mut interval_secs = initial_interval.max(1);
    let cid = client_id();

    loop {
        // Wait `interval_secs` OR cancellation — whichever arrives first.
        tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                return Err(AuthError::UserCancelled);
            }
            _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {}
        }

        let form = [
            ("grant_type", DEVICE_CODE_GRANT.to_string()),
            ("client_id", cid.clone()),
            ("device_code", device_code.to_string()),
        ];

        let resp = client
            .post(&url)
            .form(&form)
            .send()
            .await
            .map_err(|e| AuthError::Http(format!("token POST: {e}")))?;

        let status = resp.status();

        if status.is_success() {
            let body: TokenResponse = resp
                .json()
                .await
                .map_err(|e| AuthError::MalformedResponse(format!("token body: {e}")))?;
            // Emit Complete event (best-effort; receiver may have dropped).
            let _ = event_tx
                .send(DeviceCodeProgress::Complete {
                    access_token: body.access_token.clone(),
                    refresh_token: body.refresh_token.clone(),
                    expires_in: body.expires_in,
                })
                .await;
            return Ok(body);
        }

        // 4xx — parse as OAuth error response.
        let err_body: OauthErrorResponse = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                return Err(AuthError::MalformedResponse(format!(
                    "token error body: {e}"
                )));
            }
        };

        match err_body.error.as_str() {
            "authorization_pending" => {
                let _ = event_tx
                    .send(DeviceCodeProgress::AuthorizationPending {
                        interval: interval_secs,
                    })
                    .await;
                // keep polling at current interval
            }
            "slow_down" => {
                // RFC 8628 §3.5: client MUST increase interval by at least 5s.
                interval_secs = interval_secs.saturating_add(5);
                let _ = event_tx
                    .send(DeviceCodeProgress::SlowDown {
                        new_interval: interval_secs,
                    })
                    .await;
            }
            "expired_token" => return Err(AuthError::DeviceCodeExpired),
            "access_denied" => {
                return Err(AuthError::DeviceCodeFailed(
                    "user denied access".to_string(),
                ))
            }
            other => {
                return Err(AuthError::DeviceCodeFailed(format!(
                    "{other}: {}",
                    err_body.error_description
                )));
            }
        }
    }
}

/// Step 2b (AUTH-03): exchange a stored refresh_token for a fresh
/// access_token.
///
/// Called by `chain::ensure_valid_mc_token` before the XBL/XSTS/MC chain
/// when the stored MC token is near expiry.  On `invalid_grant` returns
/// `AuthError::RefreshFailed` so the caller can direct the user to
/// re-authenticate.
///
/// `base_url` should be `MSA_BASE_URL` in production.
#[tracing::instrument(name = "msa_refresh_access_token", skip_all)]
pub async fn refresh_access_token(
    client: &reqwest::Client,
    base_url: &str,
    refresh_token: &str,
) -> Result<TokenResponse, AuthError> {
    let url = format!("{}{TOKEN_PATH}", base_url.trim_end_matches('/'));
    let form = [
        ("grant_type", "refresh_token".to_string()),
        ("client_id", client_id()),
        ("refresh_token", refresh_token.to_string()),
        ("scope", MSA_SCOPE.to_string()),
    ];

    let resp = client
        .post(&url)
        .form(&form)
        .send()
        .await
        .map_err(|e| AuthError::Http(format!("refresh POST: {e}")))?;

    let status = resp.status();

    if status.is_success() {
        return resp
            .json::<TokenResponse>()
            .await
            .map_err(|e| AuthError::MalformedResponse(format!("refresh body: {e}")));
    }

    let err_body: OauthErrorResponse = resp
        .json()
        .await
        .map_err(|e| AuthError::MalformedResponse(format!("refresh error body: {e}")))?;

    match err_body.error.as_str() {
        "invalid_grant" => Err(AuthError::RefreshFailed),
        other => Err(AuthError::DeviceCodeFailed(format!(
            "refresh {other}: {}",
            err_body.error_description
        ))),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    // ------------------------------------------------------------------
    // Unit tests (no HTTP)
    // ------------------------------------------------------------------

    /// Tests both the default client ID and the env override in a single
    /// sequential test to avoid parallel env-var interference.
    #[test]
    fn test_client_id_default_and_env_override() {
        // Ensure no override is set, then verify the default.
        std::env::remove_var(MSA_CLIENT_ID_ENV);
        assert_eq!(client_id(), DEFAULT_MSA_CLIENT_ID);

        // Set override and verify it is used.
        std::env::set_var(MSA_CLIENT_ID_ENV, "override-id-xyz");
        assert_eq!(client_id(), "override-id-xyz");

        // Clean up so other tests (if any) are not affected.
        std::env::remove_var(MSA_CLIENT_ID_ENV);
    }

    #[test]
    fn test_msa_scope_contains_xbox_live_signin() {
        assert!(MSA_SCOPE.contains("XboxLive.signin"));
    }

    #[test]
    fn test_msa_scope_contains_offline_access() {
        // RFC 8628 + pitfall 5: without offline_access no refresh_token is issued.
        assert!(MSA_SCOPE.contains("offline_access"));
    }

    // ------------------------------------------------------------------
    // request_device_code
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_request_device_code_success() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/consumers/oauth2/v2.0/devicecode")
                    .body_includes("scope=XboxLive.signin");
                then.status(200).body(
                    r#"{"user_code":"ABCD-1234","verification_uri":"https://microsoft.com/link","device_code":"dc-opaque","expires_in":900,"interval":5,"message":"m"}"#,
                );
            })
            .await;

        let got = request_device_code(&http_client(), &server.base_url())
            .await
            .unwrap();
        assert_eq!(got.user_code, "ABCD-1234");
        assert_eq!(got.verification_uri, "https://microsoft.com/link");
        assert_eq!(got.device_code, "dc-opaque");
        assert_eq!(got.interval, 5);
        assert_eq!(got.expires_in, 900);
    }

    #[tokio::test]
    async fn test_request_device_code_http_error() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/consumers/oauth2/v2.0/devicecode");
                then.status(400).body(r#"{"error":"invalid_client"}"#);
            })
            .await;

        let err = request_device_code(&http_client(), &server.base_url())
            .await
            .unwrap_err();
        assert!(
            matches!(err, AuthError::DeviceCodeRequest(_)),
            "got {err:?}"
        );
    }

    // ------------------------------------------------------------------
    // poll_for_token — happy path
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_poll_success_on_first_poll() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/consumers/oauth2/v2.0/token")
                    .body_includes("grant_type=urn");
                then.status(200).body(
                    r#"{"token_type":"Bearer","scope":"XboxLive.signin offline_access","expires_in":3600,"access_token":"acc-1","refresh_token":"ref-1"}"#,
                );
            })
            .await;

        let (tx, mut rx) = mpsc::channel::<DeviceCodeProgress>(4);
        let token = CancellationToken::new();
        let got = poll_for_token(
            &http_client(),
            &server.base_url(),
            "dc-opaque",
            1,
            token,
            tx,
        )
        .await
        .unwrap();

        assert_eq!(got.access_token, "acc-1");
        assert_eq!(got.refresh_token, "ref-1");
        assert_eq!(got.expires_in, 3600);

        // Complete event must have been emitted.
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, DeviceCodeProgress::Complete { .. }));
    }

    // ------------------------------------------------------------------
    // poll_for_token — authorization_pending → cancel
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_poll_authorization_pending_then_cancelled() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST).path("/consumers/oauth2/v2.0/token");
                then.status(400)
                    .body(r#"{"error":"authorization_pending","error_description":"x"}"#);
            })
            .await;

        let (tx, mut rx) = mpsc::channel::<DeviceCodeProgress>(8);
        let token = CancellationToken::new();
        let t2 = token.clone();
        tokio::spawn(async move {
            // Let one poll complete (interval=1s) then cancel.
            tokio::time::sleep(Duration::from_millis(1100)).await;
            t2.cancel();
        });

        let err = poll_for_token(&http_client(), &server.base_url(), "dc", 1, token, tx)
            .await
            .unwrap_err();

        assert!(matches!(err, AuthError::UserCancelled), "got {err:?}");

        // At least one AuthorizationPending event should have been emitted.
        let mut saw_pending = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, DeviceCodeProgress::AuthorizationPending { .. }) {
                saw_pending = true;
            }
        }
        assert!(saw_pending, "expected at least one AuthorizationPending event");
    }

    // ------------------------------------------------------------------
    // poll_for_token — slow_down (RFC 8628 §3.5: +5s to interval)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_poll_slow_down_bumps_interval() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST).path("/consumers/oauth2/v2.0/token");
                then.status(400)
                    .body(r#"{"error":"slow_down","error_description":"x"}"#);
            })
            .await;

        let (tx, mut rx) = mpsc::channel::<DeviceCodeProgress>(8);
        let token = CancellationToken::new();
        let t2 = token.clone();
        tokio::spawn(async move {
            // Wait long enough for one slow_down cycle (1s interval + small buffer).
            tokio::time::sleep(Duration::from_millis(1200)).await;
            t2.cancel();
        });

        let _ = poll_for_token(&http_client(), &server.base_url(), "dc", 1, token, tx).await;

        // Expect SlowDown event with new_interval == 1 + 5 = 6.
        let mut saw_slowdown = false;
        while let Ok(ev) = rx.try_recv() {
            if let DeviceCodeProgress::SlowDown { new_interval } = ev {
                assert_eq!(new_interval, 6, "slow_down must add exactly 5s (RFC 8628)");
                saw_slowdown = true;
            }
        }
        assert!(saw_slowdown, "expected SlowDown event with new_interval=6");
    }

    // ------------------------------------------------------------------
    // poll_for_token — expired_token
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_poll_expired_token() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST).path("/consumers/oauth2/v2.0/token");
                then.status(400)
                    .body(r#"{"error":"expired_token","error_description":"code expired"}"#);
            })
            .await;

        let (tx, _rx) = mpsc::channel::<DeviceCodeProgress>(4);
        let err = poll_for_token(
            &http_client(),
            &server.base_url(),
            "dc",
            1,
            CancellationToken::new(),
            tx,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AuthError::DeviceCodeExpired), "got {err:?}");
    }

    // ------------------------------------------------------------------
    // poll_for_token — access_denied
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_poll_access_denied() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST).path("/consumers/oauth2/v2.0/token");
                then.status(400)
                    .body(r#"{"error":"access_denied","error_description":"user refused"}"#);
            })
            .await;

        let (tx, _rx) = mpsc::channel::<DeviceCodeProgress>(4);
        let err = poll_for_token(
            &http_client(),
            &server.base_url(),
            "dc",
            1,
            CancellationToken::new(),
            tx,
        )
        .await
        .unwrap_err();

        match err {
            AuthError::DeviceCodeFailed(m) => {
                assert!(
                    m.to_lowercase().contains("denied"),
                    "message should mention 'denied', got: {m}"
                );
            }
            other => panic!("expected DeviceCodeFailed, got {other:?}"),
        }
    }

    // ------------------------------------------------------------------
    // poll_for_token — immediate cancellation
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_poll_cancellation_immediate() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST).path("/consumers/oauth2/v2.0/token");
                then.status(400)
                    .body(r#"{"error":"authorization_pending","error_description":"x"}"#);
            })
            .await;

        let (tx, _rx) = mpsc::channel::<DeviceCodeProgress>(4);
        let token = CancellationToken::new();
        let t2 = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            t2.cancel();
        });

        let err = poll_for_token(&http_client(), &server.base_url(), "dc", 1, token, tx)
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::UserCancelled), "got {err:?}");
    }

    // ------------------------------------------------------------------
    // refresh_access_token — success
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_refresh_success() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/consumers/oauth2/v2.0/token")
                    .body_includes("grant_type=refresh_token")
                    .body_includes("refresh_token=old-refresh");
                then.status(200).body(
                    r#"{"token_type":"Bearer","scope":"XboxLive.signin offline_access","expires_in":3600,"access_token":"new-acc","refresh_token":"new-ref"}"#,
                );
            })
            .await;

        let got = refresh_access_token(&http_client(), &server.base_url(), "old-refresh")
            .await
            .unwrap();
        assert_eq!(got.access_token, "new-acc");
        assert_eq!(got.refresh_token, "new-ref");
        assert_eq!(got.expires_in, 3600);
    }

    // ------------------------------------------------------------------
    // refresh_access_token — invalid_grant → RefreshFailed
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_refresh_invalid_grant() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST).path("/consumers/oauth2/v2.0/token");
                then.status(400)
                    .body(r#"{"error":"invalid_grant","error_description":"token revoked"}"#);
            })
            .await;

        let err = refresh_access_token(&http_client(), &server.base_url(), "bad-token")
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::RefreshFailed), "got {err:?}");
    }
}
