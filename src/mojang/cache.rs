//! Disk cache primitives for Mojang protocol responses.
//!
//! Design: every cache write goes through `atomic_write` (tmp → rename).
//! TTL checks use filesystem mtime.

use std::path::Path;
use std::time::Duration;

use sha1::{Digest, Sha1};

use crate::error::AppError;

/// TTL for `version_manifest_v2.json` cache (1h). See 02-RESEARCH.md §Caching.
pub const MANIFEST_CACHE_TTL: Duration = Duration::from_secs(3600);

/// Write `bytes` to `dest` atomically: write to `dest.tmp`, then rename.
/// Creates parent directories as needed. Overwrites any existing file.
pub async fn atomic_write(dest: &Path, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = dest.with_extension("tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, dest).await?;
    Ok(())
}

/// SHA1 (lowercase hex, 40 chars) of `bytes`.
pub fn sha1_hex_of_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

/// SHA1 of `path`'s contents, lowercase hex.
pub async fn sha1_hex_of_file(path: &Path) -> Result<String, AppError> {
    let bytes = tokio::fs::read(path).await?;
    Ok(sha1_hex_of_bytes(&bytes))
}

/// True iff `path` exists AND its contents match `expected` (case-insensitive).
/// Missing file returns Ok(false), not Err.
pub async fn verify_sha1(path: &Path, expected: &str) -> Result<bool, AppError> {
    if !tokio::fs::try_exists(path).await? {
        return Ok(false);
    }
    let got = sha1_hex_of_file(path).await?;
    Ok(got.eq_ignore_ascii_case(expected))
}

/// True iff `path` exists and its mtime is within `ttl` of now.
pub async fn cache_is_fresh(path: &Path, ttl: Duration) -> Result<bool, AppError> {
    let meta = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(AppError::Io(e)),
    };
    let modified = meta.modified().map_err(AppError::Io)?;
    Ok(modified.elapsed().unwrap_or_default() < ttl)
}
