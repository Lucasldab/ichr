//! Install orchestrators. The `version_installer` composes services + task
//! system to produce a ready-to-launch install on disk.

pub mod natives_extract;
pub mod version_installer;

pub use version_installer::install_version;
