//! Stub created by 09-01; populated by 09-03 (CurseForgeClient HTTP).

use crate::mods::curseforge::error::CurseForgeError;

#[derive(Debug, Clone, Default)]
pub struct CurseForgeClient;

impl CurseForgeClient {
    pub fn new(_api_key: &str) -> Result<Self, CurseForgeError> {
        Ok(Self)
    }
}
