//! Service layer: orchestrates domain + persistence + HTTP clients.

pub mod instance_service;

pub use instance_service::{
    clone_instance, create_instance, delete_instance, list_instances, rename_instance, set_group,
};
