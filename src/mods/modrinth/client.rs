//! Stub created by 08-01; populated by 08-03 (ModrinthClient HTTP).

use crate::mods::error::ModrinthError;

#[derive(Debug, Clone, Default)]
pub struct ModrinthClient;

impl ModrinthClient {
    pub fn new() -> Result<Self, ModrinthError> { Ok(Self) }
}
