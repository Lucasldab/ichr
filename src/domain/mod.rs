//! Pure domain types. No I/O, no async. Safe to import from anywhere.

pub mod account;
pub mod instance;
pub mod platform;
pub mod version;

pub use account::{Account, AccountKind};
pub use instance::{Instance, InstanceId, ModloaderKind};
pub use platform::{Arch, OsName};
pub use version::{McVersion, VersionType};
