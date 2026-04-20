//! Persistence layer: the only code that touches platform path APIs or disk I/O.

pub mod paths;

pub use paths::AppPaths;
