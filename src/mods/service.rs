//! Stub created by 08-01; populated by 08-06 (ModrinthService facade).

use crate::mods::error::ModrinthError;

#[derive(Debug, Default)]
pub struct ModrinthService;

impl ModrinthService {
    pub fn new() -> Result<Self, ModrinthError> { Ok(Self) }
}
