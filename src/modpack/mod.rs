//! Modrinth `.mrpack` modpack import module.
//!
//! Sub-modules:
//! - `error`    — `ModpackError` typed error enum.
//! - `parse`    — `MrpackFile` and related serde types + helper functions.
//! - `download` — allowlist gate + env filter + parallel SHA-512 download orchestrator.
//!
//! Plans 10-04 (`overrides.rs`) and 10-05 (`service.rs`) will add further sub-modules.

pub mod download;
pub mod error;
pub mod parse;

pub use download::{
    download_files, filter_files_for_client, is_url_allowlisted, MODPACK_ALLOWLIST,
};
pub use error::ModpackError;
