//! Modpack import module — Modrinth `.mrpack` v1 format support.
//!
//! Entry point: `crate::modpack::service::ModpackService::import_mrpack`.
//! Public surface re-exported below for downstream plans (10-03, 10-04, 10-05).

pub mod error;
pub mod parse;

pub use error::ModpackError;
