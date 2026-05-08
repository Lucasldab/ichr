//! Integration tests for services::instance_service.
//! Each test builds its own AppPaths with_roots pointing at a tempdir.

use ichr::error::AppError;
use ichr::persistence::AppPaths;
use ichr::services::{
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

// ---------------------------------------------------------------------------
// create_instance
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_create_instance_basic() {
    let (_td, paths) = make_paths();
    let m = create_instance(&paths, "My First", "1.21.4").await.unwrap();
    assert_eq!(m.slug, "my-first");
    assert_eq!(m.display_name, "My First");
    assert_eq!(m.mc_version_id, "1.21.4");
    assert!(paths.instance_dir("my-first").exists());
    assert!(paths.instance_manifest("my-first").exists());
    assert!(paths
        .instance_minecraft_dir("my-first")
        .join("mods")
        .exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_create_instance_collision_appends_suffix() {
    let (_td, paths) = make_paths();
    create_instance(&paths, "Foo", "1.21.4").await.unwrap();
    let m = create_instance(&paths, "Foo", "1.21.4").await.unwrap();
    assert_eq!(m.slug, "foo-2");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_create_instance_rejects_empty_name() {
    let (_td, paths) = make_paths();
    let err = create_instance(&paths, "", "1.21.4").await.unwrap_err();
    assert!(
        matches!(err, AppError::InvalidInstanceName { .. }),
        "expected InvalidInstanceName, got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_create_instance_rejects_whitespace_only() {
    let (_td, paths) = make_paths();
    let err = create_instance(&paths, "   ", "1.21.4").await.unwrap_err();
    assert!(
        matches!(err, AppError::InvalidInstanceName { .. }),
        "expected InvalidInstanceName, got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_create_instance_rejects_name_longer_than_128() {
    let (_td, paths) = make_paths();
    let long_name: String = "a".repeat(129);
    let err = create_instance(&paths, &long_name, "1.21.4")
        .await
        .unwrap_err();
    assert!(
        matches!(err, AppError::InvalidInstanceName { .. }),
        "expected InvalidInstanceName, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// list_instances
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_list_instances_empty() {
    let (_td, paths) = make_paths();
    let list = list_instances(&paths).await.unwrap();
    assert_eq!(list, vec![]);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_instances_returns_created() {
    let (_td, paths) = make_paths();
    create_instance(&paths, "Alpha", "1.21.4").await.unwrap();
    create_instance(&paths, "Beta", "1.21.4").await.unwrap();
    create_instance(&paths, "Gamma", "1.21.4").await.unwrap();
    let list = list_instances(&paths).await.unwrap();
    assert_eq!(list.len(), 3);
}

// ---------------------------------------------------------------------------
// rename_instance
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_rename_instance_updates_display_name_only() {
    let (_td, paths) = make_paths();
    create_instance(&paths, "Old", "1.21.4").await.unwrap();
    rename_instance(&paths, "old", "New").await.unwrap();
    // Directory slug unchanged
    assert!(paths.instance_dir("old").exists());
    // Display name updated
    let m = ichr::instance::read_instance_manifest(&paths, "old")
        .await
        .unwrap();
    assert_eq!(m.display_name, "New");
    assert_eq!(m.slug, "old");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rename_instance_rejects_missing_instance() {
    let (_td, paths) = make_paths();
    let err = rename_instance(&paths, "nope", "X").await.unwrap_err();
    assert!(
        matches!(err, AppError::InstanceNotFound { .. }),
        "expected InstanceNotFound, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// set_group
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_set_group_updates_group_field() {
    let (_td, paths) = make_paths();
    let m = create_instance(&paths, "Grouped", "1.21.4").await.unwrap();
    set_group(&paths, &m.slug, Some("favorites".to_string()))
        .await
        .unwrap();
    let updated = ichr::instance::read_instance_manifest(&paths, &m.slug)
        .await
        .unwrap();
    assert_eq!(updated.group, Some("favorites".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_set_group_none_clears_group() {
    let (_td, paths) = make_paths();
    let m = create_instance(&paths, "Clearable", "1.21.4")
        .await
        .unwrap();
    set_group(&paths, &m.slug, Some("favorites".to_string()))
        .await
        .unwrap();
    set_group(&paths, &m.slug, None).await.unwrap();
    let updated = ichr::instance::read_instance_manifest(&paths, &m.slug)
        .await
        .unwrap();
    assert_eq!(updated.group, None);
}

// ---------------------------------------------------------------------------
// delete_instance
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_instance_removes_directory() {
    let (_td, paths) = make_paths();
    let m = create_instance(&paths, "ToDelete", "1.21.4").await.unwrap();
    assert!(paths.instance_dir(&m.slug).exists());
    delete_instance(&paths, &m.slug).await.unwrap();
    assert!(!paths.instance_dir(&m.slug).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_instance_missing_returns_not_found() {
    let (_td, paths) = make_paths();
    let err = delete_instance(&paths, "nope").await.unwrap_err();
    assert!(
        matches!(err, AppError::InstanceNotFound { .. }),
        "expected InstanceNotFound, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// clone_instance
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_copies_mods_and_config_not_saves() {
    let (_td, paths) = make_paths();
    // Create source instance
    create_instance(&paths, "A", "1.21.4").await.unwrap();
    let src_mc = paths.instance_minecraft_dir("a");

    // Populate mods, config, saves, and natives
    tokio::fs::write(src_mc.join("mods").join("foo.jar"), b"mod-bytes")
        .await
        .unwrap();
    tokio::fs::write(src_mc.join("config").join("bar.toml"), b"config-bytes")
        .await
        .unwrap();
    tokio::fs::create_dir_all(src_mc.join("saves").join("world"))
        .await
        .unwrap();
    tokio::fs::write(
        src_mc.join("saves").join("world").join("level.dat"),
        b"save-bytes",
    )
    .await
    .unwrap();
    let natives_dir = paths.instance_dir("a").join("natives");
    tokio::fs::create_dir_all(&natives_dir).await.unwrap();
    tokio::fs::write(natives_dir.join("libfoo.so"), b"native-bytes")
        .await
        .unwrap();

    // Clone
    let cloned = clone_instance(&paths, "a", "A Copy").await.unwrap();
    assert_eq!(cloned.slug, "a-copy");

    let dst_mc = paths.instance_minecraft_dir("a-copy");

    // mods and config MUST be copied with same content
    let mods_bytes = tokio::fs::read(dst_mc.join("mods").join("foo.jar"))
        .await
        .unwrap();
    assert_eq!(mods_bytes, b"mod-bytes");
    let config_bytes = tokio::fs::read(dst_mc.join("config").join("bar.toml"))
        .await
        .unwrap();
    assert_eq!(config_bytes, b"config-bytes");

    // saves and natives MUST NOT be copied
    assert!(
        !dst_mc
            .join("saves")
            .join("world")
            .join("level.dat")
            .exists(),
        "saves should not be cloned"
    );
    assert!(
        !paths
            .instance_dir("a-copy")
            .join("natives")
            .join("libfoo.so")
            .exists(),
        "natives should not be cloned"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_resets_timestamps_and_play_time() {
    let (_td, paths) = make_paths();
    create_instance(&paths, "Src", "1.21.4").await.unwrap();

    // Manually update the source manifest to have play state
    let mut m = ichr::instance::read_instance_manifest(&paths, "src")
        .await
        .unwrap();
    m.last_played_at = Some("2026-01-01T00:00:00Z".to_string());
    m.total_play_time_ms = 99999;
    ichr::instance::write_instance_manifest(&paths, &m)
        .await
        .unwrap();

    let cloned = clone_instance(&paths, "src", "Src Copy").await.unwrap();
    assert_eq!(
        cloned.last_played_at, None,
        "last_played_at should be reset"
    );
    assert_eq!(
        cloned.total_play_time_ms, 0,
        "total_play_time_ms should be reset"
    );
    // created_at on the clone must not carry over the source's last_played_at
    assert_ne!(
        cloned.created_at, "2026-01-01T00:00:00Z",
        "created_at should be a fresh timestamp, not the source's last_played_at value"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_new_name_collision_appends_suffix() {
    let (_td, paths) = make_paths();
    create_instance(&paths, "A", "1.21.4").await.unwrap();
    // First clone produces "a-copy"
    clone_instance(&paths, "a", "A Copy").await.unwrap();
    // Second clone with same name produces "a-copy-2"
    let second = clone_instance(&paths, "a", "A Copy").await.unwrap();
    assert_eq!(second.slug, "a-copy-2");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_rejects_empty_new_name() {
    let (_td, paths) = make_paths();
    create_instance(&paths, "A", "1.21.4").await.unwrap();
    let err = clone_instance(&paths, "a", "").await.unwrap_err();
    assert!(
        matches!(err, AppError::InvalidInstanceName { .. }),
        "expected InvalidInstanceName, got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_missing_source_returns_not_found() {
    let (_td, paths) = make_paths();
    let err = clone_instance(&paths, "nope", "X").await.unwrap_err();
    assert!(
        matches!(err, AppError::InstanceNotFound { .. }),
        "expected InstanceNotFound, got {err:?}"
    );
}
