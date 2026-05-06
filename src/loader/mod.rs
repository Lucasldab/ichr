//! Fabric, Quilt, Forge, NeoForge modloader install — meta fetch + (for
//! Forge/NeoForge) installer subprocess + library harvest + version JSON
//! write. See `.planning/phases/06-fabric-and-quilt-modloaders/06-RESEARCH.md`
//! and `.planning/phases/07-forge-and-neoforge-modloaders/07-RESEARCH.md`.

pub mod error;
pub mod fabric;
pub mod forge_meta;
pub mod forgewrapper;
pub mod harvest;
pub mod installer_subprocess;
pub mod maven;
pub mod maven_metadata;
pub mod neoforge_meta;
pub mod quilt;
pub mod service;
pub mod staging;
pub mod types;

pub use error::LoaderError;
pub use service::LoaderService;
pub use types::{LoaderInfo, LoaderLibrary, LoaderType, LoaderVersionEntry};
