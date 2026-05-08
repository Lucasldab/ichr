//! Modpack import module — Modrinth `.mrpack` v1 format support.
//!
//! Entry point: `crate::modpack::service::ModpackService::import_mrpack`.
//! Public surface re-exported below for downstream plans (10-03, 10-04, 10-05).

pub mod error;
pub mod parse;

pub use error::ModpackError;
pub use parse::{
    detect_loader, parse_index, should_download_for_client, strip_leading_dot_slash,
    EnvRequirement, MrpackEnv, MrpackFile, MrpackHashes, MrpackIndex,
};
