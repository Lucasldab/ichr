//! Modrinth `.mrpack` modpack import module.
//!
//! Sub-modules:
//! - `error`    — `ModpackError` typed error enum (11 variants; Plan 10-02).
//! - `parse`    — `MrpackIndex`/`MrpackFile`/`MrpackHashes`/`MrpackEnv` serde types
//!                + `parse_index` validator + `detect_loader` + `should_download_for_client`
//!                + `strip_leading_dot_slash` (Plan 10-02).
//! - `download` — `MODPACK_ALLOWLIST` (7 hosts) + `is_url_allowlisted` (allowlist gate
//!                BEFORE network call) + `filter_files_for_client` (env.client honor) +
//!                `download_files` parallel SHA-512 orchestrator (Plan 10-03; reuses
//!                `MOD_DOWNLOAD_CONCURRENCY` and `download_one_with_hash_algo` from
//!                `crate::mods::installer`).
//!
//! Plans 10-04 (`overrides.rs`) and 10-05 (`service.rs`) will add further sub-modules.

pub mod download;
pub mod error;
pub mod parse;

pub use download::{
    download_files, filter_files_for_client, is_url_allowlisted, MODPACK_ALLOWLIST,
};
pub use error::ModpackError;
pub use parse::{
    detect_loader, parse_index, should_download_for_client, strip_leading_dot_slash,
    EnvRequirement, MrpackEnv, MrpackFile, MrpackHashes, MrpackIndex,
};
