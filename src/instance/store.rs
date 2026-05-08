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

/// Record that a launch is about to begin: set `last_played_at` to now.
/// `total_play_time_ms` is not touched -- that is updated on exit.
pub async fn mark_launch_started(
    paths: &AppPaths,
    slug: &str,
) -> Result<InstanceManifest, AppError> {
    if !tokio::fs::try_exists(&paths.instance_manifest(slug)).await? {
        return Err(AppError::InstanceNotFound {
            slug: slug.to_string(),
        });
    }
    let mut m = read_instance_manifest(paths, slug).await?;
    m.last_played_at = Some(crate::domain::instance::now_iso8601_utc());
    write_instance_manifest(paths, &m).await?;
    Ok(m)
}

/// Record a completed launch: add `additional_ms` to `total_play_time_ms`
/// (saturating) and refresh `last_played_at`.
pub async fn update_play_time(
    paths: &AppPaths,
    slug: &str,
    additional_ms: u64,
) -> Result<InstanceManifest, AppError> {
    if !tokio::fs::try_exists(&paths.instance_manifest(slug)).await? {
        return Err(AppError::InstanceNotFound {
            slug: slug.to_string(),
        });
    }
    let mut m = read_instance_manifest(paths, slug).await?;
    m.last_played_at = Some(crate::domain::instance::now_iso8601_utc());
    m.total_play_time_ms = m.total_play_time_ms.saturating_add(additional_ms);
    write_instance_manifest(paths, &m).await?;
    Ok(m)
}

/// Scan `instances_dir` and return all successfully-parsed manifests.
///
/// Sort order: `last_played_at` DESC (most recent first; `None` last),
/// tiebreaker by `display_name` ASC.
///
/// Invalid or unreadable `instance.json` files are logged and skipped -- NOT fatal.
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
            (Some(_), None) => std::cmp::Ordering::Less, // a has date, b doesn't → a first
            (None, Some(_)) => std::cmp::Ordering::Greater, // a has no date, b does → b first
            (None, None) => a.display_name.cmp(&b.display_name),
        }
    });
    Ok(out)
}

/// Atomically set or clear the `java_override` field on an instance manifest.
///
/// `None` clears the override (field omitted from serialized JSON per serde rules).
#[tracing::instrument(skip_all, fields(slug))]
pub async fn set_java_override(
    paths: &AppPaths,
    slug: &str,
    override_id: Option<crate::java::types::JavaRuntimeId>,
) -> Result<InstanceManifest, AppError> {
    let mut m = read_instance_manifest(paths, slug).await?;
    m.java_override = override_id;
    write_instance_manifest(paths, &m).await?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::InstanceManifest;
    use tempfile::TempDir;

    fn paths_in(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    #[tokio::test]
    async fn test_update_play_time_saturating() {
        let td = TempDir::new().unwrap();
        let paths = paths_in(&td);
        let mut m = InstanceManifest::new("x".into(), "x".into(), "1.21.4".into());
        m.total_play_time_ms = u64::MAX - 1;
        write_instance_manifest(&paths, &m).await.unwrap();
        let updated = update_play_time(&paths, "x", 1000).await.unwrap();
        assert_eq!(
            updated.total_play_time_ms,
            u64::MAX,
            "play_time_ms must saturate at u64::MAX"
        );
        assert!(
            updated.last_played_at.is_some(),
            "last_played_at must be set"
        );
    }

    #[tokio::test]
    async fn test_update_play_time_missing_slug() {
        let td = TempDir::new().unwrap();
        let paths = paths_in(&td);
        let result = update_play_time(&paths, "nope", 100).await;
        assert!(
            matches!(result, Err(AppError::InstanceNotFound { .. })),
            "missing slug must return InstanceNotFound; got {result:?}"
        );
    }

    #[tokio::test]
    async fn test_mark_launch_started_sets_last_played_at() {
        let td = TempDir::new().unwrap();
        let paths = paths_in(&td);
        let m = InstanceManifest::new("y".into(), "y".into(), "1.21.4".into());
        write_instance_manifest(&paths, &m).await.unwrap();
        let updated = mark_launch_started(&paths, "y").await.unwrap();
        assert!(
            updated.last_played_at.is_some(),
            "mark_launch_started must set last_played_at"
        );
        // play_time_ms should remain unchanged (0)
        assert_eq!(
            updated.total_play_time_ms, 0,
            "mark_launch_started must not change total_play_time_ms"
        );
    }

    #[tokio::test]
    async fn test_mark_launch_started_missing_slug() {
        let td = TempDir::new().unwrap();
        let paths = paths_in(&td);
        let result = mark_launch_started(&paths, "nonexistent").await;
        assert!(
            matches!(result, Err(AppError::InstanceNotFound { .. })),
            "missing slug must return InstanceNotFound; got {result:?}"
        );
    }
}
