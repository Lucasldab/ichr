//! CurseForge for Studios REST API v1 client (sub-submodule).
//! See `.planning/phases/09-curseforge-integration/09-RESEARCH.md` §Endpoint Reference.

pub mod api_key;
pub mod client;
pub mod error;
pub mod filter;
pub mod installer;
pub mod service;
pub mod types;
pub mod url;

pub use client::CurseForgeClient;
pub use error::CurseForgeError;
pub use service::CurseForgeService;
