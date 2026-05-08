//! Microsoft authentication + Minecraft services auth chain.
//!
//! Module layout (see .planning/phases/04-microsoft-authentication/04-RESEARCH.md
//! §"Recommended Project Structure"):
//!   - device_code.rs  MSA OAuth device-code request + polling
//!   - xbox.rs         Xbox Live + XSTS POST calls
//!   - xsts_errors.rs  Pure XErr code -> user string mapping
//!   - mc_services.rs  Minecraft login + entitlement + profile fetch
//!   - chain.rs        Orchestrates the full auth chain + token refresh
//!   - store.rs        keyring-primary + AES-256-GCM file fallback
//!   - service.rs      High-level AccountService facade for the TUI

pub mod chain;
pub mod device_code;
pub mod mc_services;
pub mod service;
pub mod store;
pub mod xbox;
pub mod xsts_errors;

pub use crate::domain::account::{Account, AccountKind, StorageBackend};

/// Authentication context passed to the launcher.
/// Replaces the bare `username: &str` parameter in `launch_instance`.
///
/// The Offline variant preserves the Phase 3 offline-launch behavior;
/// the Msa variant triggers `chain::ensure_valid_mc_token(account_id)`
/// at launch time to produce a valid `MsaTokens` snapshot.
#[derive(Debug, Clone)]
pub enum AuthContext {
    /// Offline mode -- arbitrary username, no MS account.
    Offline { username: String },
    /// MSA online mode -- account_id used to load tokens from store.
    Msa { account_id: String },
}

/// MC tokens resolved at launch time (after refresh if needed).
/// Consumed by `launcher::service::launch_instance` to populate
/// the `SubstitutionContext` for the MSA path.
#[derive(Debug, Clone)]
pub struct MsaTokens {
    /// For SubstitutionContext.auth_access_token (the --accessToken arg).
    pub mc_access_token: String,
    /// For SubstitutionContext.auth_uuid (hyphenated UUID).
    pub mc_uuid: String,
    /// For SubstitutionContext.auth_player_name.
    pub mc_player_name: String,
    /// For SubstitutionContext.auth_xuid (the XSTS uhs -- same as user_hash).
    pub xuid: String,
    /// For SubstitutionContext.auth_xbox_user_hash (the XSTS uhs).
    pub user_hash: String,
    /// Always "msa" for this variant. Offline uses "legacy".
    pub user_type: String,
}

/// Typed auth error. Convertible to `AppError` via `#[from]`.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Device code request failed: {0}")]
    DeviceCodeRequest(String),

    #[error("Device code polling failed: {0}")]
    DeviceCodeFailed(String),

    #[error("Device code expired -- please try again")]
    DeviceCodeExpired,

    #[error("User cancelled authentication")]
    UserCancelled,

    #[error("Xbox Live authentication failed: {0}")]
    XboxLive(String),

    #[error("Xbox authentication error: {message} (XErr: {xerr})")]
    XstsDenied { xerr: u64, message: String },

    #[error("Minecraft token exchange failed: {0}")]
    McLogin(String),

    #[error("This account does not own Minecraft Java Edition. Purchase at minecraft.net or activate Game Pass.")]
    NoMinecraftLicense,

    #[error("Minecraft profile fetch failed: {0}")]
    ProfileFetch(String),

    #[error("Token refresh failed -- please re-authenticate")]
    RefreshFailed,

    #[error("Keyring unavailable: {0}")]
    KeyringUnavailable(String),

    #[error("Account not found: {0}")]
    AccountNotFound(String),

    #[error("Credential storage I/O error: {0}")]
    StorageIo(String),

    #[error("Credential decryption failed")]
    DecryptFailed,

    #[error("HTTP error during auth: {0}")]
    Http(String),

    #[error("Malformed response from {0}")]
    MalformedResponse(String),
}
