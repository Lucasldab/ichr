//! Modrinth integration — search, dependency resolution, mod install/uninstall,
//! per-instance mod ledger.
//! See `.planning/phases/08-modrinth-integration/08-RESEARCH.md`.

pub mod dep_resolve;
pub mod error;
pub mod filter;
pub mod installer;
pub mod ledger;
pub mod modrinth;
pub mod service;
pub mod types;

pub use error::ModrinthError;
pub use service::ModrinthService;
pub use types::ModSource;
