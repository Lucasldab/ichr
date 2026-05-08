//! Override-file extractor for `.mrpack` modpacks.
//!
//! Implements `apply_overrides`, which copies files from the `overrides/` and
//! `client-overrides/` directories inside a `.mrpack` ZIP into the instance's
//! `.minecraft/` directory. Files in `client-overrides/` are written *after*
//! `overrides/` so client-specific files win on path collision (CONTEXT.md D-05).
//!
//! # Security controls
//!
//! - **Path traversal:** Every entry name has its prefix stripped before being
//!   passed to [`crate::util::safe_zip::safe_extract_path`], which rejects any
//!   name containing `..`, absolute roots, or Windows drive prefixes. Entries
//!   that fail the check are skipped silently (consistent with
//!   `install::natives_extract` semantics).
//! - **Symlink entries:** Entries that the `zip` crate identifies as symlinks are
//!   skipped with a `tracing::warn!` log (Pitfall 4). The JVM would resolve them
//!   at launch and could read files outside `.minecraft/`.
//! - **Unix permissions:** After each write, permissions are normalized to
//!   `0o644` on Unix platforms (Pitfall 9), overriding any mode bits stored in
//!   the archive metadata.
//!
//! # Cancellation
//!
//! `tokio::task::spawn_blocking` cannot be interrupted once started.
//! [`CancellationToken::is_cancelled`] is therefore checked **before** entering
//! `spawn_blocking`. If the token fires during extraction the extraction runs to
//! completion (typically < 1 s for override archives); the cancel is honored at
//! the next `await` point after `spawn_blocking` returns.

use std::path::Path;

use tokio_util::sync::CancellationToken;

use crate::modpack::error::ModpackError;
use crate::util::safe_zip::safe_extract_path;

/// Apply `overrides/` and `client-overrides/` from a `.mrpack` ZIP into
/// `dest_minecraft_dir`.
///
/// Extraction order (CONTEXT.md D-05, LOCKED):
///  1. All entries whose name starts with `overrides/` are extracted first.
///  2. All entries whose name starts with `client-overrides/` are extracted
///     second, overwriting any file written in step 1 at the same relative path.
///
/// `server-overrides/` entries are silently ignored (mineltui is a client
/// launcher; server-side overrides are out of scope for v1).
///
/// # Returns
///
/// `Ok(n)` where `n` is the total number of files extracted across both passes
/// (informational; useful for progress display in Plan 10-05).
///
/// # Errors
///
/// Returns [`ModpackError::Cancelled`] if `token` is already cancelled before
/// the blocking extraction begins. Returns [`ModpackError::Zip`] or
/// [`ModpackError::Io`] on archive or filesystem errors.
#[tracing::instrument(skip_all, fields(
    mrpack = %mrpack_path.display(),
    dest = %dest_minecraft_dir.display()
))]
pub async fn apply_overrides(
    mrpack_path: &Path,
    dest_minecraft_dir: &Path,
    token: &CancellationToken,
) -> Result<usize, ModpackError> {
    // Pre-flight cancel check — spawn_blocking cannot be interrupted once started.
    if token.is_cancelled() {
        return Err(ModpackError::Cancelled);
    }

    tokio::fs::create_dir_all(dest_minecraft_dir).await?;

    let mrpack_path = mrpack_path.to_owned();
    let dest = dest_minecraft_dir.to_owned();

    let count = tokio::task::spawn_blocking(move || -> Result<usize, ModpackError> {
        let file = std::fs::File::open(&mrpack_path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let mut total = 0usize;

        // Pass 1: overrides/
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_string();
            if !name.starts_with("overrides/") {
                continue;
            }
            let stripped = name["overrides/".len()..].to_string();
            total += extract_one_entry(&mut entry, &stripped, &name, &dest)?;
        }

        // Pass 2: client-overrides/ (last-write-wins over pass 1)
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_string();
            if !name.starts_with("client-overrides/") {
                continue;
            }
            let stripped = name["client-overrides/".len()..].to_string();
            total += extract_one_entry(&mut entry, &stripped, &name, &dest)?;
        }

        Ok(total)
    })
    .await
    .map_err(|e| ModpackError::Io(std::io::Error::other(format!("spawn_blocking join: {e}"))))??;

    Ok(count)
}

/// Extract a single zip entry into `dest_root`, after its directory prefix has
/// already been stripped into `stripped_name`.
///
/// Returns `1` if a file was written, `0` if the entry was skipped (directory
/// stub, symlink, or path-traversal rejection).
///
/// The `full_entry_name` parameter is the original name from the archive (with
/// prefix) and is used only for diagnostic log messages.
fn extract_one_entry<R: std::io::Read>(
    entry: &mut zip::read::ZipFile<'_, R>,
    stripped_name: &str,
    full_entry_name: &str,
    dest_root: &Path,
) -> Result<usize, ModpackError> {
    // Skip directory stubs (trailing slash or empty after prefix-strip).
    if stripped_name.is_empty() || stripped_name.ends_with('/') {
        return Ok(0);
    }

    // Skip symlink entries — never create or follow archive-declared symlinks
    // (Pitfall 4). The zip crate 8.5.1 exposes `is_symlink()` directly.
    if entry.is_symlink() {
        tracing::warn!(
            entry = %full_entry_name,
            "skipping symlink entry in modpack archive"
        );
        return Ok(0);
    }

    // Path-traversal guard: reject any name with `..`, absolute prefix, etc.
    let Some(safe_path) = safe_extract_path(stripped_name, dest_root) else {
        tracing::warn!(
            entry = %full_entry_name,
            "skipping path-traversal entry in modpack archive"
        );
        return Ok(0);
    };

    // Ensure parent directories exist.
    if let Some(parent) = safe_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write the file.
    let mut out = std::fs::File::create(&safe_path)?;
    std::io::copy(entry, &mut out)?;
    drop(out);

    // Normalize Unix permissions to 0o644 regardless of archive metadata
    // (Pitfall 9 — defense vs. malicious chmod bits in archive).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&safe_path, std::fs::Permissions::from_mode(0o644))?;
    }

    Ok(1)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;
    use zip::write::SimpleFileOptions;

    use super::apply_overrides;

    // ── fixture builder ───────────────────────────────────────────────────────

    /// Build a synthetic `.mrpack` zip at `path` from a list of
    /// `(entry_name, content)` pairs.
    fn build_test_mrpack(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, content) in entries {
            writer.start_file(*name, opts).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }

    /// Build an mrpack with a specific unix_permissions on one entry.
    fn build_mrpack_with_perms(
        path: &std::path::Path,
        entry_name: &str,
        content: &[u8],
        mode: u32,
    ) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(mode);
        writer.start_file(entry_name, opts).unwrap();
        writer.write_all(content).unwrap();
        writer.finish().unwrap();
    }

    /// Build an mrpack containing a symlink entry (using the zip crate's
    /// `add_symlink` API).
    #[cfg(unix)]
    fn build_mrpack_with_symlink(
        path: &std::path::Path,
        regular_name: &str,
        regular_content: &[u8],
        symlink_name: &str,
        symlink_target: &str,
    ) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        // Regular file
        writer.start_file(regular_name, opts).unwrap();
        writer.write_all(regular_content).unwrap();
        // Symlink entry
        writer
            .add_symlink(symlink_name, symlink_target, opts)
            .unwrap();
        writer.finish().unwrap();
    }

    // ── test 1: happy path ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_apply_overrides_happy_path() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        build_test_mrpack(&mrpack, &[("overrides/config/foo.toml", b"ok")]);

        let token = CancellationToken::new();
        let count = apply_overrides(&mrpack, &dest, &token).await.unwrap();

        assert_eq!(count, 1);
        let contents = std::fs::read_to_string(dest.join("config/foo.toml")).unwrap();
        assert_eq!(contents, "ok");
    }

    // ── test 2: client-overrides last-write-wins ──────────────────────────────

    #[tokio::test]
    async fn test_client_overrides_overwrites_overrides() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        build_test_mrpack(
            &mrpack,
            &[
                ("overrides/options.txt", b"from overrides"),
                ("client-overrides/options.txt", b"from client-overrides"),
            ],
        );

        let token = CancellationToken::new();
        let count = apply_overrides(&mrpack, &dest, &token).await.unwrap();

        assert_eq!(count, 2);
        let contents = std::fs::read_to_string(dest.join("options.txt")).unwrap();
        assert_eq!(contents, "from client-overrides");
    }

    // ── test 3: overrides first, then client-overrides (positive order check) ─

    #[tokio::test]
    async fn test_overrides_first_pass_writes_then_client_overrides_second_pass() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        build_test_mrpack(
            &mrpack,
            &[
                ("overrides/options.txt", b"from overrides"),
                ("client-overrides/options.txt", b"from client-overrides"),
            ],
        );

        let token = CancellationToken::new();
        apply_overrides(&mrpack, &dest, &token).await.unwrap();

        // Client-overrides ran AFTER overrides, so its content wins.
        let contents = std::fs::read_to_string(dest.join("options.txt")).unwrap();
        assert_eq!(
            contents, "from client-overrides",
            "client-overrides must run after overrides and overwrite the file"
        );
    }

    // ── test 4: server-overrides silently skipped ─────────────────────────────

    #[tokio::test]
    async fn test_server_overrides_silently_skipped() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        build_test_mrpack(&mrpack, &[("server-overrides/world/level.dat", b"binary")]);

        let token = CancellationToken::new();
        let count = apply_overrides(&mrpack, &dest, &token).await.unwrap();

        assert_eq!(count, 0);
        assert!(!dest.join("world/level.dat").exists());
    }

    // ── test 5: path traversal in overrides/ silently skipped ─────────────────

    #[tokio::test]
    async fn test_path_traversal_entry_silently_skipped() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        // The raw entry name includes the overrides/ prefix followed by a
        // traversal sequence.
        build_test_mrpack(&mrpack, &[("overrides/../../../etc/passwd", b"root:x:0:0")]);

        let token = CancellationToken::new();
        let count = apply_overrides(&mrpack, &dest, &token).await.unwrap();

        // Nothing should have been extracted (dest may not even exist yet).
        assert_eq!(count, 0);
        // The traversal target must not have been written.
        assert!(!std::path::Path::new("/etc/passwd_mineltui_test").exists());
    }

    // ── test 6: path traversal in client-overrides/ silently skipped ──────────

    #[tokio::test]
    async fn test_path_traversal_with_client_override_prefix() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        build_test_mrpack(&mrpack, &[("client-overrides/../../etc/shadow", b"x")]);

        let token = CancellationToken::new();
        let count = apply_overrides(&mrpack, &dest, &token).await.unwrap();

        assert_eq!(count, 0);
    }

    // ── test 7: directory entries skipped from count ──────────────────────────

    #[tokio::test]
    async fn test_directory_entries_skipped() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        build_test_mrpack(
            &mrpack,
            &[
                ("overrides/config/", b""), // directory stub
                ("overrides/config/real.toml", b"ok"),
            ],
        );

        let token = CancellationToken::new();
        let count = apply_overrides(&mrpack, &dest, &token).await.unwrap();

        assert_eq!(count, 1, "directory stub must not be counted");
        assert!(dest.join("config/real.toml").exists());
    }

    // ── test 8: files outside known prefixes are ignored ──────────────────────

    #[tokio::test]
    async fn test_apply_overrides_ignores_files_outside_known_prefixes() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        build_test_mrpack(
            &mrpack,
            &[
                ("modrinth.index.json", b"{}"),
                ("overrides/foo.txt", b"x"),
                ("random-other-prefix/bar.txt", b"y"),
            ],
        );

        let token = CancellationToken::new();
        let count = apply_overrides(&mrpack, &dest, &token).await.unwrap();

        assert_eq!(count, 1);
        assert!(dest.join("foo.txt").exists());
        assert!(!dest.join("modrinth.index.json").exists());
        assert!(!dest.join("bar.txt").exists());
    }

    // ── test 9: pre-cancel returns Cancelled ──────────────────────────────────

    #[tokio::test]
    async fn test_pre_cancel_returns_cancelled() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        build_test_mrpack(&mrpack, &[("overrides/foo.txt", b"x")]);

        let token = CancellationToken::new();
        token.cancel();

        let result = apply_overrides(&mrpack, &dest, &token).await;
        assert!(
            matches!(result, Err(crate::modpack::error::ModpackError::Cancelled)),
            "expected Cancelled, got: {result:?}"
        );
        // dest should not have been created (we bail before create_dir_all).
        // (On some platforms create_dir_all is a no-op on an existing dir, so
        // only assert no *files* were written — not that the dir is absent.)
        if dest.exists() {
            assert_eq!(
                std::fs::read_dir(&dest).unwrap().count(),
                0,
                "no files should be extracted after pre-cancel"
            );
        }
    }

    // ── test 10: Unix perms normalized to 0o644 ───────────────────────────────

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_perms_normalized_to_0o644() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        // Build an mrpack where the entry has 0o777 Unix mode.
        build_mrpack_with_perms(&mrpack, "overrides/foo.txt", b"data", 0o777);

        let token = CancellationToken::new();
        let count = apply_overrides(&mrpack, &dest, &token).await.unwrap();

        assert_eq!(count, 1);
        let meta = std::fs::metadata(dest.join("foo.txt")).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o644,
            "permissions must be normalized to 0o644, got 0o{mode:o}"
        );
    }

    // ── test 11: symlink entry silently skipped ───────────────────────────────

    #[cfg(unix)]
    #[tokio::test]
    async fn test_symlink_entry_silently_skipped() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        build_mrpack_with_symlink(
            &mrpack,
            "overrides/regular.txt",
            b"real content",
            "overrides/link.txt",
            "/etc/passwd",
        );

        let token = CancellationToken::new();
        let count = apply_overrides(&mrpack, &dest, &token).await.unwrap();

        // Only the regular file should be extracted; symlink must be skipped.
        assert_eq!(count, 1, "symlink must not be counted");
        assert!(dest.join("regular.txt").exists());
        assert!(!dest.join("link.txt").exists());
    }

    // ── test 12: idempotent double apply ──────────────────────────────────────

    #[tokio::test]
    async fn test_idempotent_double_apply() {
        let tmp = TempDir::new().unwrap();
        let mrpack = tmp.path().join("test.mrpack");
        let dest = tmp.path().join("minecraft");

        build_test_mrpack(
            &mrpack,
            &[
                ("overrides/config/mod.toml", b"setting=1"),
                ("client-overrides/options.txt", b"gamma=2.0"),
            ],
        );

        let token = CancellationToken::new();
        let count1 = apply_overrides(&mrpack, &dest, &token).await.unwrap();
        let count2 = apply_overrides(&mrpack, &dest, &token).await.unwrap();

        assert_eq!(count1, count2, "idempotent: same file count on second call");

        let contents_mod = std::fs::read_to_string(dest.join("config/mod.toml")).unwrap();
        let contents_opt = std::fs::read_to_string(dest.join("options.txt")).unwrap();
        assert_eq!(contents_mod, "setting=1");
        assert_eq!(contents_opt, "gamma=2.0");
    }
}
