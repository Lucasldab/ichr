//! End-to-end instance lifecycle tests. No network, no Mojang downloads.
//! All state lives under a fresh tempdir per test.

use mineltui::error::AppError;
use mineltui::persistence::AppPaths;
use mineltui::services::{
    clone_instance, create_instance, delete_instance, list_instances, rename_instance, set_group,
};
use tempfile::tempdir;

fn make_paths() -> (tempfile::TempDir, AppPaths) {
    let td = tempdir().unwrap();
    let paths = AppPaths::with_roots(
        td.path().join("data"),
        td.path().join("config"),
        td.path().join("cache"),
    );
    (td, paths)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_full_lifecycle() {
    let (_td, paths) = make_paths();

    let a1 = create_instance(&paths, "Alpha", "1.21.4").await.unwrap();
    assert_eq!(a1.slug, "alpha");
    assert!(paths.instance_manifest("alpha").exists());
    assert!(paths.instance_minecraft_dir("alpha").join("mods").exists());

    let a2 = create_instance(&paths, "Alpha", "1.21.4").await.unwrap();
    assert_eq!(a2.slug, "alpha-2");

    let list = list_instances(&paths).await.unwrap();
    assert_eq!(list.len(), 2);

    let renamed = rename_instance(&paths, "alpha", "Alpha Renamed")
        .await
        .unwrap();
    assert_eq!(renamed.display_name, "Alpha Renamed");
    assert_eq!(renamed.slug, "alpha");
    assert!(paths.instance_dir("alpha").exists());

    let grouped = set_group(&paths, "alpha", Some("vanilla".into()))
        .await
        .unwrap();
    assert_eq!(grouped.group.as_deref(), Some("vanilla"));

    let mod_path = paths
        .instance_minecraft_dir("alpha")
        .join("mods")
        .join("fakemod.jar");
    tokio::fs::write(&mod_path, b"mod contents").await.unwrap();
    let cloned = clone_instance(&paths, "alpha", "Alpha Clone")
        .await
        .unwrap();
    assert_eq!(cloned.slug, "alpha-clone");
    let cloned_mod = paths
        .instance_minecraft_dir("alpha-clone")
        .join("mods")
        .join("fakemod.jar");
    assert_eq!(tokio::fs::read(&cloned_mod).await.unwrap(), b"mod contents");

    delete_instance(&paths, "alpha").await.unwrap();
    assert!(!paths.instance_dir("alpha").exists());
    let err = delete_instance(&paths, "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::InstanceNotFound { .. }));

    let list = list_instances(&paths).await.unwrap();
    assert_eq!(list.len(), 2);
    let slugs: Vec<&str> = list.iter().map(|m| m.slug.as_str()).collect();
    assert!(slugs.contains(&"alpha-2"));
    assert!(slugs.contains(&"alpha-clone"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_lifecycle_rejects_invalid_names() {
    let (_td, paths) = make_paths();
    assert!(matches!(
        create_instance(&paths, "", "1.21.4").await,
        Err(AppError::InvalidInstanceName { .. })
    ));
    assert!(matches!(
        create_instance(&paths, "   ", "1.21.4").await,
        Err(AppError::InvalidInstanceName { .. })
    ));
    let long = "x".repeat(129);
    assert!(matches!(
        create_instance(&paths, &long, "1.21.4").await,
        Err(AppError::InvalidInstanceName { .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_lifecycle_clone_on_missing_source() {
    let (_td, paths) = make_paths();
    let err = clone_instance(&paths, "nope", "X").await.unwrap_err();
    assert!(matches!(err, AppError::InstanceNotFound { .. }));
}
