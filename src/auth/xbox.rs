//! Xbox Live authentication + XSTS authorization.
//!
//! Both POSTs are hand-rolled against reqwest 0.13; no crate abstracts the
//! Xbox-specific leg of the MSA auth chain correctly as of 2026-04.
//!
//! Pitfalls enforced here:
//!   - pitfall 1: `RpsTicket` MUST be prefixed with `"d="` (non-negotiable)
//!   - pitfall 2: XSTS 401 MUST be parsed for `XErr` u64 + mapped via xsts_errors
//!   - anti-pattern 3: XSTS `SandboxId` = `"RETAIL"` and `RelyingParty` =
//!     `"rp://api.minecraftservices.com/"` are non-negotiable
//!   - anti-pattern header: all POSTs set `x-xbl-contract-version: 1`
//!
//! Base URLs are parameters so httpmock tests can redirect to localhost.

use serde::Deserialize;

use crate::auth::xsts_errors::map_xerr;
use crate::auth::AuthError;

pub const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
pub const XSTS_AUTHORIZE_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";

pub const XSTS_MC_RELYING_PARTY: &str = "rp://api.minecraftservices.com/";
pub const XSTS_SANDBOX: &str = "RETAIL";

/// Parsed XBL `user/authenticate` response (also re-used for XSTS which has
/// the same shape for the 200 path).
#[derive(Debug, Clone, Deserialize)]
struct XblResponseRaw {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Debug, Clone, Deserialize)]
struct DisplayClaims {
    xui: Vec<XuiEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct XuiEntry {
    uhs: String,
}

/// Parsed XSTS 401 error body.
#[derive(Debug, Clone, Deserialize)]
struct XstsErrorBody {
    #[serde(rename = "XErr")]
    x_err: u64,
    /// Present in the MS response but not used -- we always call `map_xerr`
    /// for a consistent user-facing message instead of forwarding the raw text.
    #[serde(rename = "Message")]
    #[allow(dead_code)]
    message: Option<String>,
}

/// Typed XBL token tuple.
#[derive(Debug, Clone)]
pub struct XblTokens {
    /// The XBL-issued JWT; feed into XSTS request.
    pub token: String,
    /// User hash; carries through to MC `identityToken` format.
    pub user_hash: String,
}

/// Typed XSTS token tuple (different token, same uhs as XBL).
#[derive(Debug, Clone)]
pub struct XstsTokens {
    /// The XSTS-issued JWT; feed into MC `login_with_xbox` identityToken.
    pub token: String,
    /// User hash; used in `XBL3.0 x={uhs};{token}` identityToken format.
    pub user_hash: String,
}

/// Xbox Live authentication (step 3 of the chain).
///
/// `base_url` lets httpmock tests point at localhost; pass `XBL_AUTH_URL`
/// for production (the path `/user/authenticate` is appended here).
///
/// `#[tracing::instrument(skip_all)]` prevents token values from leaking
/// into span fields (pitfall 16).
#[tracing::instrument(name = "xbl_authenticate", skip_all)]
pub async fn authenticate_xbox_live(
    client: &reqwest::Client,
    base_url: &str,
    ms_access_token: &str,
) -> Result<XblTokens, AuthError> {
    let url = format!("{}/user/authenticate", base_url.trim_end_matches('/'));
    // CRITICAL: RpsTicket MUST carry the "d=" prefix (pitfall 1).
    let body = serde_json::json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={ms_access_token}")
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT"
    });
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("x-xbl-contract-version", "1")
        .json(&body)
        .send()
        .await
        .map_err(|e| AuthError::Http(format!("xbl POST: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        tracing::warn!(status = %status, "xbox live authentication failed");
        return Err(AuthError::XboxLive(format!(
            "HTTP {status}: {}",
            truncate_for_msg(&body_text, 200)
        )));
    }
    let parsed: XblResponseRaw = resp
        .json()
        .await
        .map_err(|e| AuthError::MalformedResponse(format!("xbl body: {e}")))?;
    let uhs = parsed
        .display_claims
        .xui
        .first()
        .ok_or_else(|| AuthError::MalformedResponse("xbl DisplayClaims.xui empty".into()))?
        .uhs
        .clone();
    Ok(XblTokens {
        token: parsed.token,
        user_hash: uhs,
    })
}

/// XSTS authorization (step 4 of the chain).
///
/// On 401 parses the XErr code and produces `AuthError::XstsDenied`
/// with a user-readable message from `xsts_errors::map_xerr`.
#[tracing::instrument(name = "xsts_authorize", skip_all)]
pub async fn authenticate_xsts(
    client: &reqwest::Client,
    base_url: &str,
    xbl_token: &str,
) -> Result<XstsTokens, AuthError> {
    let url = format!("{}/xsts/authorize", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "Properties": {
            "SandboxId": XSTS_SANDBOX,
            "UserTokens": [xbl_token]
        },
        "RelyingParty": XSTS_MC_RELYING_PARTY,
        "TokenType": "JWT"
    });
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("x-xbl-contract-version", "1")
        .json(&body)
        .send()
        .await
        .map_err(|e| AuthError::Http(format!("xsts POST: {e}")))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        let err_body: XstsErrorBody = resp
            .json()
            .await
            .map_err(|e| AuthError::MalformedResponse(format!("xsts 401 body: {e}")))?;
        let msg = map_xerr(err_body.x_err);
        tracing::warn!(xerr = err_body.x_err, "xsts denied");
        return Err(AuthError::XstsDenied {
            xerr: err_body.x_err,
            message: msg,
        });
    }
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(AuthError::Http(format!(
            "xsts HTTP {status}: {}",
            truncate_for_msg(&body_text, 200)
        )));
    }
    let parsed: XblResponseRaw = resp
        .json()
        .await
        .map_err(|e| AuthError::MalformedResponse(format!("xsts body: {e}")))?;
    let uhs = parsed
        .display_claims
        .xui
        .first()
        .ok_or_else(|| AuthError::MalformedResponse("xsts DisplayClaims.xui empty".into()))?
        .uhs
        .clone();
    Ok(XstsTokens {
        token: parsed.token,
        user_hash: uhs,
    })
}

/// Truncate a string to `n` chars for safe inclusion in error messages.
/// Does not attempt to redact -- caller must ensure the string is safe
/// (e.g. only pass status text or error body outside token contexts).
fn truncate_for_msg(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}...", &s[..n])
    }
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

    #[tokio::test]
    async fn test_xbl_success() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/user/authenticate")
                    .header("content-type", "application/json")
                    .header("x-xbl-contract-version", "1");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"Token":"xbl-tok","DisplayClaims":{"xui":[{"uhs":"user-hash-1"}]}}"#);
            })
            .await;

        let client = http_client();
        let got = authenticate_xbox_live(&client, &server.base_url(), "msa-access")
            .await
            .unwrap();
        assert_eq!(got.token, "xbl-tok");
        assert_eq!(got.user_hash, "user-hash-1");
    }

    #[tokio::test]
    async fn test_xbl_rps_ticket_d_prefix() {
        // CRITICAL: verify the serialized body contains "d=" + token.
        let server = MockServer::start_async().await;
        let m = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/user/authenticate")
                    .body_includes("\"RpsTicket\":\"d=msa-token-42\"");
                then.status(200)
                    .body(r#"{"Token":"t","DisplayClaims":{"xui":[{"uhs":"h"}]}}"#);
            })
            .await;
        let _ = authenticate_xbox_live(&http_client(), &server.base_url(), "msa-token-42")
            .await
            .unwrap();
        m.assert_async().await;
    }

    #[tokio::test]
    async fn test_xsts_success() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/xsts/authorize")
                    .body_includes("\"SandboxId\":\"RETAIL\"")
                    .body_includes("\"RelyingParty\":\"rp://api.minecraftservices.com/\"");
                then.status(200).body(
                    r#"{"Token":"xsts-tok","DisplayClaims":{"xui":[{"uhs":"user-hash-2"}]}}"#,
                );
            })
            .await;
        let got = authenticate_xsts(&http_client(), &server.base_url(), "xbl-token")
            .await
            .unwrap();
        assert_eq!(got.token, "xsts-tok");
        assert_eq!(got.user_hash, "user-hash-2");
    }

    #[tokio::test]
    async fn test_xsts_401_maps_xerr() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST).path("/xsts/authorize");
                then.status(401)
                    .header("content-type", "application/json")
                    .body(
                        r#"{"Identity":"0","XErr":2148916233,"Message":"The account doesn't have an Xbox profile","Redirect":"https://start.ui.xboxlive.com/CreateAccount"}"#,
                    );
            })
            .await;
        let err = authenticate_xsts(&http_client(), &server.base_url(), "xbl-token")
            .await
            .unwrap_err();
        match err {
            AuthError::XstsDenied { xerr, message } => {
                assert_eq!(xerr, 2148916233);
                assert!(
                    message.to_lowercase().contains("xbox profile"),
                    "expected xbox profile message from map_xerr; got: {message}"
                );
            }
            other => panic!("expected XstsDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_xsts_unknown_xerr_still_mapped() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST).path("/xsts/authorize");
                then.status(401)
                    .body(r#"{"Identity":"0","XErr":9999999999,"Message":"x","Redirect":"r"}"#);
            })
            .await;
        let err = authenticate_xsts(&http_client(), &server.base_url(), "xbl-token")
            .await
            .unwrap_err();
        match err {
            AuthError::XstsDenied { xerr, message } => {
                assert_eq!(xerr, 9999999999);
                assert!(message.contains("9999999999"), "got: {message}");
            }
            other => panic!("expected XstsDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_xbl_500_returns_xbox_live_error() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST).path("/user/authenticate");
                then.status(500).body("boom");
            })
            .await;
        let err = authenticate_xbox_live(&http_client(), &server.base_url(), "tok")
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::XboxLive(_)), "got {err:?}");
    }
}
