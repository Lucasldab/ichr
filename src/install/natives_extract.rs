//! Extract a classifier-style natives JAR (pre-1.19 libraries) into the
//! per-instance natives directory. Applies `extract.exclude` entries and
//! guards against ZIP path-traversal.
//!
//! Uses `spawn_blocking` — the `zip` crate is synchronous CPU work.

use std::path::Path;

use crate::error::AppError;
use crate::util::safe_zip::safe_extract_path;

/// Extract the contents of `jar_path` into `dest_dir`. Paths are canonicalized
/// and any entry that would escape `dest_dir` (via `..` or absolute paths)
/// is skipped. Entries whose name starts with any `exclude` prefix are skipped.
pub async fn extract_native_jar(
    jar_path: &Path,
    dest_dir: &Path,
    exclude: &[String],
) -> Result<(), AppError> {
    tokio::fs::create_dir_all(dest_dir).await?;
    let jar_path = jar_path.to_owned();
    let dest_dir = dest_dir.to_owned();
    let exclude = exclude.to_vec();

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let file = std::fs::File::open(&jar_path)?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| AppError::Http(format!("zip open {jar_path:?}: {e}")))?;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| AppError::Http(format!("zip entry {i}: {e}")))?;
            let name = entry.name().to_string();
            // Skip directory stubs (trailing slash).
            if name.ends_with('/') {
                continue;
            }
            // Skip excluded prefixes (e.g. "META-INF/").
            if exclude.iter().any(|ex| name.starts_with(ex.as_str())) {
                continue;
            }
            // Reject path-traversal entries.
            let Some(safe_path) = safe_extract_path(&name, &dest_dir) else {
                continue;
            };
            if let Some(parent) = safe_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&safe_path)?;
            std::io::copy(&mut entry, &mut out)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::Http(format!("spawn_blocking join: {e}")))??;

    Ok(())
}

