//! Minecraft services auth (login + entitlement + profile).
//!
//! After XSTS succeeds, the chain exchanges the XSTS token for a
//! Minecraft services access token, verifies entitlement (the MS
//! account must actually own Minecraft Java Edition — pitfall 5),
//! and fetches the player profile (UUID + display name).
//!
//! Pitfalls enforced here:
//!   - pitfall 5: `check_entitlement` treats `items: []` as a hard error
//!     with a user-facing message (`AuthError::NoMinecraftLicense`)
//!   - pitfall 16: no token values in `tracing::` macro arguments
//!   - identityToken format: `"XBL3.0 x=<uhs>;<xsts_token>"` — verbatim
//!     (anti-pattern 4 in research: wrong RelyingParty OR wrong format
//!     produces an invalid MC token that Minecraft servers reject).

use serde::Deserialize;

use crate::auth::AuthError;

pub const MC_AUTH_URL: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
pub const MC_ENTITLEMENT_URL: &str =
    "https://api.minecraftservices.com/entitlements/mcstore";
pub const MC_PROFILE_URL: &str =
    "https://api.minecraftservices.com/minecraft/profile";

/// Parsed MC services login response.
#[derive(Debug, Clone, Deserialize)]
pub struct McLoginResponse {
    pub access_token: String,
    /// Seconds until expiry (typically 86400 = 24h).
    pub expires_in: i64,
}

/// Parsed entitlement response. Only the `items` field is consulted; we
/// deliberately do NOT verify the signature — per Microsoft's documentation
/// that is server-side validation and the signature format changes.
#[derive(Debug, Clone, Deserialize)]
pub struct EntitlementResponse {
    pub items: Vec<EntitlementItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntitlementItem {
    pub name: String,
}

/// Parsed profile response. `id` is 32-char hex (no hyphens).
#[derive(Debug, Clone, Deserialize)]
pub struct McProfile {
    /// 32-char hex UUID without hyphens (e.g. "c6bf8193...").
    pub id: String,
    /// Player display name.
    pub name: String,
}

/// Step 5 of the chain: exchange XSTS token for a Minecraft access token.
///
/// The POST body contains `"identityToken":"XBL3.0 x={user_hash};{xsts_token}"`.
/// This exact format is required — wrong prefix or separator produces a valid
/// HTTP 200 but an MC token that game servers reject.
#[tracing::instrument(name = "mc_login", skip_all)]
pub async fn login_with_xbox(
    client: &reqwest::Client,
    base_url: &str,
    user_hash: &str,
    xsts_token: &str,
) -> Result<McLoginResponse, AuthError> {
    let url = format!(
        "{}/authentication/login_with_xbox",
        base_url.trim_end_matches('/')
    );
    // CRITICAL: identityToken MUST be "XBL3.0 x={uhs};{xsts_token}"
    let identity_token = format!("XBL3.0 x={user_hash};{xsts_token}");
    let body = serde_json::json!({ "identityToken": identity_token });
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AuthError::Http(format!("mc login POST: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(AuthError::McLogin(format!(
            "HTTP {status}: {}",
            truncate_for_msg(&body_text, 200)
        )));
    }
    resp.json()
        .await
        .map_err(|e| AuthError::MalformedResponse(format!("mc login body: {e}")))
}

/// Step 6 of the chain: verify MS account actually owns Minecraft Java Edition.
///
/// Returns `Ok(())` when `items` contains entries named `product_minecraft`
/// AND `game_minecraft`. Returns `AuthError::NoMinecraftLicense` on empty items
/// or when either required entry is missing (pitfall 5: even Game Pass users
/// need to have launched the official launcher once to create the license entry).
#[tracing::instrument(name = "mc_entitlement", skip_all)]
pub async fn check_entitlement(
    client: &reqwest::Client,
    base_url: &str,
    mc_access_token: &str,
) -> Result<(), AuthError> {
    let url = format!("{}/entitlements/mcstore", base_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .bearer_auth(mc_access_token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| AuthError::Http(format!("entitlement GET: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(AuthError::Http(format!(
            "entitlement HTTP {status}: {}",
            truncate_for_msg(&body_text, 200)
        )));
    }
    let parsed: EntitlementResponse = resp
        .json()
        .await
        .map_err(|e| AuthError::MalformedResponse(format!("entitlement body: {e}")))?;
    if parsed.items.is_empty() {
        return Err(AuthError::NoMinecraftLicense);
    }
    let has_product = parsed.items.iter().any(|i| i.name == "product_minecraft");
    let has_game = parsed.items.iter().any(|i| i.name == "game_minecraft");
    if !has_product || !has_game {
        return Err(AuthError::NoMinecraftLicense);
    }
    Ok(())
}

/// Step 7 of the chain: fetch the Minecraft player profile (UUID + name).
///
/// Validates that the returned `id` is a 32-char hex string. Call
/// `format_uuid` on the returned `id` to get the standard hyphenated form.
#[tracing::instrument(name = "mc_profile", skip_all)]
pub async fn fetch_profile(
    client: &reqwest::Client,
    base_url: &str,
    mc_access_token: &str,
) -> Result<McProfile, AuthError> {
    let url = format!("{}/minecraft/profile", base_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .bearer_auth(mc_access_token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| AuthError::Http(format!("profile GET: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(AuthError::ProfileFetch(format!(
            "HTTP {status}: {}",
            truncate_for_msg(&body_text, 200)
        )));
    }
    let parsed: McProfile = resp
        .json()
        .await
        .map_err(|e| AuthError::MalformedResponse(format!("profile body: {e}")))?;
    if parsed.id.len() != 32 || !parsed.id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AuthError::MalformedResponse(format!(
            "profile id not 32-char hex: len={}",
            parsed.id.len()
        )));
    }
    Ok(parsed)
}

/// Format a 32-char hex UUID into 8-4-4-4-12 hyphenated form.
///
/// Returns `AuthError::MalformedResponse` if the input is not exactly 32
/// ASCII hex characters. The raw 32-char `id` comes from `fetch_profile`.
pub fn format_uuid(hex32: &str) -> Result<String, AuthError> {
    if hex32.len() != 32 || !hex32.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AuthError::MalformedResponse(format!(
            "format_uuid: expected 32 hex chars, got len={}",
            hex32.len()
        )));
    }
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex32[0..8],
        &hex32[8..12],
        &hex32[12..16],
        &hex32[16..20],
        &hex32[20..32]
    ))
}

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

    // ── format_uuid pure tests ────────────────────────────────────────────────

    #[test]
    fn test_format_uuid_ok() {
        let hex = "c6bf819300004000800000000000abcd";
        let got = format_uuid(hex).unwrap();
        assert_eq!(got, "c6bf8193-0000-4000-8000-00000000abcd");
    }

    #[test]
    fn test_format_uuid_rejects_wrong_length() {
        assert!(format_uuid("short").is_err());
        assert!(format_uuid(&"a".repeat(33)).is_err());
    }

    #[test]
    fn test_format_uuid_rejects_non_hex() {
        let bad = "c6bf819300004000800000000000abcZ";
        assert!(format_uuid(bad).is_err());
    }

    // ── httpmock tests ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_login_success_and_identity_token_format() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/authentication/login_with_xbox")
                    .body_includes("\"identityToken\":\"XBL3.0 x=uhs-1;xsts-2\"");
                then.status(200).body(
                    r#"{"username":"u","access_token":"mc-tok","expires_in":86400,"token_type":"Bearer"}"#,
                );
            })
            .await;

        let got = login_with_xbox(&http_client(), &server.base_url(), "uhs-1", "xsts-2")
            .await
            .unwrap();
        assert_eq!(got.access_token, "mc-tok");
        assert_eq!(got.expires_in, 86400);
    }

    #[tokio::test]
    async fn test_login_non_200_returns_mc_login_error() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(POST).path("/authentication/login_with_xbox");
                then.status(401).body("Unauthorized");
            })
            .await;
        let err = login_with_xbox(&http_client(), &server.base_url(), "uhs", "tok")
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::McLogin(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn test_entitlement_valid_returns_ok() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/entitlements/mcstore")
                    .header("authorization", "Bearer mc-tok");
                then.status(200).body(
                    r#"{"items":[{"name":"product_minecraft","signature":"s1"},{"name":"game_minecraft","signature":"s2"}],"signature":"s"}"#,
                );
            })
            .await;
        check_entitlement(&http_client(), &server.base_url(), "mc-tok")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_entitlement_empty_returns_no_license() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(GET).path("/entitlements/mcstore");
                then.status(200).body(r#"{"items":[],"signature":""}"#);
            })
            .await;
        let err = check_entitlement(&http_client(), &server.base_url(), "mc-tok")
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::NoMinecraftLicense), "got {err:?}");
    }

    #[tokio::test]
    async fn test_entitlement_missing_one_item_returns_no_license() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(GET).path("/entitlements/mcstore");
                then.status(200).body(
                    r#"{"items":[{"name":"product_minecraft","signature":"s1"}],"signature":""}"#,
                );
            })
            .await;
        let err = check_entitlement(&http_client(), &server.base_url(), "mc-tok")
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::NoMinecraftLicense), "got {err:?}");
    }

    #[tokio::test]
    async fn test_profile_success_and_hex_validation() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/minecraft/profile")
                    .header("authorization", "Bearer mc-tok");
                then.status(200).body(
                    r#"{"id":"c6bf819300004000800000000000abcd","name":"PlayerOne"}"#,
                );
            })
            .await;
        let got = fetch_profile(&http_client(), &server.base_url(), "mc-tok")
            .await
            .unwrap();
        assert_eq!(got.id, "c6bf819300004000800000000000abcd");
        assert_eq!(got.name, "PlayerOne");
    }

    #[tokio::test]
    async fn test_profile_malformed_id_rejected() {
        let server = MockServer::start_async().await;
        let _m = server
            .mock_async(|when, then| {
                when.method(GET).path("/minecraft/profile");
                then.status(200).body(r#"{"id":"short","name":"X"}"#);
            })
            .await;
        let err = fetch_profile(&http_client(), &server.base_url(), "mc-tok")
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::MalformedResponse(_)), "got {err:?}");
    }
}
