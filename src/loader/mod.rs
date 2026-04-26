//! Fabric and Quilt modloader install — meta fetch, library download, version JSON write.
//! See `.planning/phases/06-fabric-and-quilt-modloaders/06-RESEARCH.md`.

pub mod error;
pub mod fabric;
pub mod maven;
pub mod quilt;
pub mod service;
pub mod types;

pub use error::LoaderError;
pub use service::LoaderService;
pub use types::{LoaderInfo, LoaderLibrary, LoaderType, LoaderVersionEntry};
