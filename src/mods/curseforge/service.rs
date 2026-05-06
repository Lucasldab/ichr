//! Stub created by 09-01; populated by 09-05 (CurseForgeService facade).

use crate::mods::curseforge::error::CurseForgeError;

#[derive(Debug, Default)]
pub struct CurseForgeService;

impl CurseForgeService {
    pub fn new() -> Result<Self, CurseForgeError> {
        Ok(Self)
    }

    pub fn api_key_present(&self) -> bool {
        false
    }
}
