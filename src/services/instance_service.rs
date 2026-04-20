//! Instance CRUD: create, list, rename, set_group, delete, clone.

use std::path::{Path, PathBuf};

use crate::domain::InstanceManifest;
use crate::error::AppError;
use crate::instance::{
    list_instance_manifests, read_instance_manifest, slugify, unique_slug,
    write_instance_manifest,
};
use crate::persistence::paths::AppPaths;

/// Validation: max length for a user-provided instance name.
pub const MAX_INSTANCE_NAME_LEN: usize = 128;

/// Subdirectories of `.minecraft/` that ARE copied during a clone.
const CLONE_INCLUDE_SUBDIRS: &[&str] = &["mods", "config"];

/// Create a new instance on disk. Generates a unique slug, builds the
/// `.minecraft` subtree, and writes `instance.json`.
///
/// Rejects names that are empty after trimming or exceed `MAX_INSTANCE_NAME_LEN`.
/// Does NOT perform the Mojang download — that is the install orchestrator's job.
pub async fn create_instance(
    paths: &AppPaths,
    display_name: &str,
    mc_version_id: &str,
) -> Result<InstanceManifest, AppError> {
    let trimmed = display_name.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInstanceName {
            reason: "name cannot be empty".into(),
        });
    }
    if trimmed.chars().count() > MAX_INSTANCE_NAME_LEN {
        return Err(AppError::InvalidInstanceName {
            reason: format!("name exceeds {MAX_INSTANCE_NAME_LEN} characters"),
        });
    }
    if mc_version_id.trim().is_empty() {
        return Err(AppError::InvalidInstanceName {
            reason: "mc_version_id cannot be empty".into(),
        });
    }
    let base = slugify(trimmed);
    let slug = unique_slug(&base, &paths.instances_dir()).await;
    let manifest = InstanceManifest::new(
        trimmed.to_string(),
        slug,
        mc_version_id.to_string(),
    );
    write_instance_manifest(paths, &manifest).await?;
    Ok(manifest)
}

/// List all instances found under `instances_dir`, sorted per 02-04 store.rs.
pub async fn list_instances(paths: &AppPaths) -> Result<Vec<InstanceManifest>, AppError> {
    list_instance_manifests(paths).await
}

/// Update the `display_name` of an existing instance. The slug and filesystem
/// directory are NOT renamed — per locked decision ("rename is display-only").
pub async fn rename_instance(
    paths: &AppPaths,
    slug: &str,
    new_display_name: &str,
) -> Result<InstanceManifest, AppError> {
    let trimmed = new_display_name.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInstanceName {
            reason: "name cannot be empty".into(),
        });
    }
    if trimmed.chars().count() > MAX_INSTANCE_NAME_LEN {
        return Err(AppError::InvalidInstanceName {
            reason: format!("name exceeds {MAX_INSTANCE_NAME_LEN} characters"),
        });
    }
    let mut m = read_or_not_found(paths, slug).await?;
    m.display_name = trimmed.to_string();
    write_instance_manifest(paths, &m).await?;
    Ok(m)
}

/// Set or clear the `group` tag for an instance. `Some(_)` sets the tag;
/// `None` clears it.
pub async fn set_group(
    paths: &AppPaths,
    slug: &str,
    group: Option<String>,
) -> Result<InstanceManifest, AppError> {
    let mut m = read_or_not_found(paths, slug).await?;
    m.group = group.map(|g| g.trim().to_string()).filter(|s| !s.is_empty());
    write_instance_manifest(paths, &m).await?;
    Ok(m)
}

/// Remove the instance directory (including natives, saves, etc.).
/// Returns `AppError::InstanceNotFound` if the slug does not exist.
pub async fn delete_instance(paths: &AppPaths, slug: &str) -> Result<(), AppError> {
    let dir = paths.instance_dir(slug);
    if !tokio::fs::try_exists(&dir).await? {
        return Err(AppError::InstanceNotFound { slug: slug.into() });
    }
    tokio::fs::remove_dir_all(&dir).await?;
    Ok(())
}

/// Clone an existing instance. Copies `.minecraft/mods/`, `.minecraft/config/`,
/// and a reset copy of `instance.json`. Does NOT copy `.minecraft/saves/`,
/// `.minecraft/resourcepacks/`, `.minecraft/shaderpacks/`, or `natives/`.
/// Timestamps and play time are reset on the new manifest.
pub async fn clone_instance(
    paths: &AppPaths,
    source_slug: &str,
    new_display_name: &str,
) -> Result<InstanceManifest, AppError> {
    let trimmed = new_display_name.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInstanceName {
            reason: "name cannot be empty".into(),
        });
    }
    if trimmed.chars().count() > MAX_INSTANCE_NAME_LEN {
        return Err(AppError::InvalidInstanceName {
            reason: format!("name exceeds {MAX_INSTANCE_NAME_LEN} characters"),
        });
    }
    let source = read_or_not_found(paths, source_slug).await?;
    let base = slugify(trimmed);
    let new_slug = unique_slug(&base, &paths.instances_dir()).await;

    // Build fresh manifest (timestamps + play time reset per locked clone policy).
    let mut new_manifest = InstanceManifest::new(
        trimmed.to_string(),
        new_slug.clone(),
        source.mc_version_id.clone(),
    );
    // Group is inherited from source; play state is not.
    new_manifest.group = source.group.clone();

    // Copy mods/ and config/ from source's .minecraft into new instance's .minecraft.
    // write_instance_manifest will create the .minecraft subtree for the new slug.
    let src_mc = paths.instance_minecraft_dir(source_slug);
    let dst_mc = paths.instance_minecraft_dir(&new_slug);
    for sub in CLONE_INCLUDE_SUBDIRS {
        let src_sub = src_mc.join(sub);
        let dst_sub = dst_mc.join(sub);
        if tokio::fs::try_exists(&src_sub).await? {
            copy_tree(&src_sub, &dst_sub).await?;
        }
    }

    // Write manifest last so a failure mid-copy does not leave a stale entry
    // that list_instances would try to show.
    write_instance_manifest(paths, &new_manifest).await?;
    Ok(new_manifest)
}

// ---- helpers ----------------------------------------------------------------

async fn read_or_not_found(
    paths: &AppPaths,
    slug: &str,
) -> Result<InstanceManifest, AppError> {
    if !tokio::fs::try_exists(&paths.instance_manifest(slug)).await? {
        return Err(AppError::InstanceNotFound { slug: slug.into() });
    }
    read_instance_manifest(paths, slug).await
}

/// Recursively copy `src` into `dst`. Creates parent directories as needed.
/// Symlinks are skipped (logged at debug level) to prevent escape outside the
/// source tree (T-2-05-01).
async fn copy_tree(src: &Path, dst: &Path) -> Result<(), AppError> {
    // BFS to avoid async recursion (which requires BoxFuture + pin).
    let mut queue: Vec<(PathBuf, PathBuf)> = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((src_dir, dst_dir)) = queue.pop() {
        tokio::fs::create_dir_all(&dst_dir).await?;
        let mut entries = tokio::fs::read_dir(&src_dir).await?;
        while let Some(e) = entries.next_entry().await? {
            let ft = e.file_type().await?;
            let src_path = e.path();
            let dst_path = dst_dir.join(e.file_name());
            if ft.is_dir() {
                queue.push((src_path, dst_path));
            } else if ft.is_file() {
                tokio::fs::copy(&src_path, &dst_path).await?;
            } else {
                // Symlinks: skip for v1 to avoid security complications.
                tracing::debug!(
                    path = %src_path.display(),
                    "skipping non-file entry during clone"
                );
            }
        }
    }
    Ok(())
}
