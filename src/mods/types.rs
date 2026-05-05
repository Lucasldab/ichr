//! Modrinth domain types: wire shapes, UI state shapes, and ledger schema.
//!
//! Stub introduced by 08-01 Task 1 to satisfy `pub use types::ModSource;`
//! in `mods/mod.rs`; Task 3 of this same plan replaces this with the full
//! type set (wire types, UI-SPEC types, Ledger).

use serde::{Deserialize, Serialize};

/// Source of an installed mod — supports forward-compat with Phase 9
/// (CurseForge), Phase 10 (modpack), and any future "manual drop" detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModSource {
    Modrinth,
    CurseForge,
    Manual,
    Modpack,
}
