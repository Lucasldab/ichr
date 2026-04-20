//! Instance manifest disk I/O. Pure serde + atomic writes.

use crate::domain::InstanceManifest;
use crate::error::AppError;
use crate::mojang::cache::atomic_write;
use crate::persistence::paths::AppPaths;

/// Read `instance.json` for the given slug.
///
/// Returns `AppError::Io` on missing file, `AppError::InstanceSerde` on parse error.
pub async fn read_instance_manifest(
    paths: &AppPaths,
    slug: &str,
) -> Result<InstanceManifest, AppError> {
    let path = paths.instance_manifest(slug);
    let bytes = tokio::fs::read(&path).await?;
    serde_json::from_slice::<InstanceManifest>(&bytes)
        .map_err(|e| AppError::InstanceSerde(format!("{path:?}: {e}")))
}

/// Write `manifest` to its canonical path atomically (tmp → rename).
///
/// Creates the instance directory and `.minecraft` subdirectory tree if
/// they don't exist. The `.minecraft` subdirs (mods, config, saves,
/// resourcepacks, shaderpacks) are created to make the instance ready for
/// launch without a separate init step.
pub async fn write_instance_manifest(
    paths: &AppPaths,
    manifest: &InstanceManifest,
) -> Result<(), AppError> {
    let path = paths.instance_manifest(&manifest.slug);
    // Ensure canonical .minecraft subdirectory tree exists.
    let mc = paths.instance_minecraft_dir(&manifest.slug);
    for sub in ["mods", "config", "saves", "resourcepacks", "shaderpacks"] {
        tokio::fs::create_dir_all(mc.join(sub)).await?;
    }
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| AppError::InstanceSerde(format!("serialize: {e}")))?;
    atomic_write(&path, &bytes).await
}

/// Scan `instances_dir` and return all successfully-parsed manifests.
///
/// Sort order: `last_played_at` DESC (most recent first; `None` last),
/// tiebreaker by `display_name` ASC.
///
/// Invalid or unreadable `instance.json` files are logged and skipped — NOT fatal.
pub async fn list_instance_manifests(paths: &AppPaths) -> Result<Vec<InstanceManifest>, AppError> {
    let instances_dir = paths.instances_dir();
    if !tokio::fs::try_exists(&instances_dir).await? {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut entries = tokio::fs::read_dir(&instances_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let slug = entry.file_name();
        let Some(slug_str) = slug.to_str() else {
            continue;
        };
        match read_instance_manifest(paths, slug_str).await {
            Ok(m) => out.push(m),
            Err(e) => {
                tracing::warn!(slug = %slug_str, error = %e, "skipping unreadable instance");
            }
        }
    }
    // Sort: last_played_at DESC (None last), then display_name ASC.
    // Comparator receives (a, b); returning Less means a comes before b.
    out.sort_by(|a, b| {
        match (a.last_played_at.as_deref(), b.last_played_at.as_deref()) {
            (Some(at), Some(bt)) => bt.cmp(at).then_with(|| a.display_name.cmp(&b.display_name)),
            (Some(_), None) => std::cmp::Ordering::Less,   // a has date, b doesn't → a first
            (None, Some(_)) => std::cmp::Ordering::Greater, // a has no date, b does → b first
            (None, None) => a.display_name.cmp(&b.display_name),
        }
    });
    Ok(out)
}
