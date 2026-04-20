//! Instance CRUD primitives.
//!
//! - `slug` — name → slug conversion + collision resolution
//! - `store` — read / write / list `instance.json` on disk
//!
//! Full create/clone/delete orchestration lives in the `services` layer
//! (plan 02-05) and depends on this module.

pub mod slug;
pub mod store;

pub use slug::{slugify, unique_slug};
pub use store::{list_instance_manifests, read_instance_manifest, write_instance_manifest};
