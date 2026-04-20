//! Account domain types. Placeholder; Phase 4 fills in token fields.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    Microsoft,
    Offline,
}

#[derive(Debug, Clone)]
pub struct Account {
    pub kind: AccountKind,
    pub username: String,
}
