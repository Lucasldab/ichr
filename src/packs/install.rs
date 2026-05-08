//! Drop-from-path pack install entry point.
//!
//! Per CONTEXT.md D-LOCK drop-from-path:
//! - Validation order: file_name → ext → is_safe_pack_filename → size cap →
//!   collision check → cancel gate → create_dir_all → cancel-aware copy →
//!   SHA-1 hash → upsert_pack
//! - Copy via `tokio::fs::read` (full file into memory, up to 500 MB cap) with
//!   cancel-aware `tokio::select!` wrapper.
//! - SHA-1 ledger entry (no integrity verification -- user-supplied file).
//! - On cancel mid-copy: delete partial file + return `PackError::Cancelled`.
//!
//! Pitfall 4: `Path::file_name()` is called BEFORE any `.join()` -- the raw
//! input string is never used in path construction.
//!
//! Pitfall 2: `is_safe_pack_filename` is used (not `is_safe_mod_filename`).
//!
//! Pitfall 3: `create_dir_all` is called before write (old instances without
//! the `.minecraft/resourcepacks` or `.minecraft/shaderpacks` subdirectory).

use std::path::{Path, PathBuf};

use sha1::Digest as _;
use sha1::Sha1;
use tokio_util::sync::CancellationToken;

use crate::mods::filter::{is_safe_pack_filename, MAX_PACK_FILE_BYTES};
use crate::mods::ledger::upsert_pack;
use crate::mods::types::{HashAlgo, InstalledModRow, ModSource};
use crate::packs::error::PackError;
use crate::packs::kind::PackKind;
use crate::persistence::paths::AppPaths;

/// Result of a successful drop-from-path install.
#[derive(Debug, Clone)]
pub struct DropPackOutcome {
    /// The ledger row that was upserted.
    pub row: InstalledModRow,
    /// Destination path on disk (absolute).
    pub dest: PathBuf,
}

/// Install a local pack file into `<instance>/.minecraft/{resourcepacks,shaderpacks}/`.
///
/// # Validation order (D-LOCK)
///
/// 1. `source_path.file_name()` extraction (path-traversal guard -- Pitfall 4)
/// 2. Lowercase-suffix `.zip` check (case-insensitive -- Windows NTFS)
/// 3. `is_safe_pack_filename` allowlist → `PackError::UnsafeFilename`
/// 4. `tokio::fs::metadata` → `PackError::NotFound` on absent or non-file;
///    `PackError::FileTooLarge` if `size > MAX_PACK_FILE_BYTES`
/// 5. Collision check via `tokio::fs::try_exists` on `dest_path` →
///    `PackError::FilenameCollision` (D-LOCK refuse policy)
/// 6. Cancel check
/// 7. `tokio::fs::create_dir_all(dest_dir)` -- defensive (Pitfall 3)
/// 8. Cancel-aware `tokio::select!` read + write
/// 9. Build `InstalledModRow` with `source=ModSource::Local`
/// 10. `upsert_pack` ledger row
#[tracing::instrument(
    name = "packs::drop_pack_from_path",
    skip_all,
    fields(
        source = %source_path.display(),
        slug = %slug,
        kind = ?kind,
    )
)]
pub async fn drop_pack_from_path(
    paths: &AppPaths,
    slug: &str,
    kind: PackKind,
    source_path: &Path,
    token: &CancellationToken,
) -> Result<DropPackOutcome, PackError> {
    // ── (1) file_name extraction (path-traversal guard -- Pitfall 4) ──────────
    // Path::file_name() is called BEFORE any .join(); the raw source_path
    // string is NEVER concatenated into the dest path.
    let dest_filename_os = source_path.file_name().ok_or_else(|| PackError::NotFound {
        path: source_path.display().to_string(),
    })?;
    let dest_filename = dest_filename_os.to_string_lossy().into_owned();

    // ── (2) Extension check (case-insensitive -- Windows NTFS) ────────────────
    if !dest_filename.to_ascii_lowercase().ends_with(".zip") {
        return Err(PackError::NotAZip {
            path: dest_filename,
        });
    }

    // ── (3) is_safe_pack_filename allowlist ───────────────────────────────────
    if !is_safe_pack_filename(&dest_filename) {
        return Err(PackError::UnsafeFilename {
            filename: dest_filename,
        });
    }

    // ── (4) metadata + size cap ───────────────────────────────────────────────
    let meta = tokio::fs::metadata(source_path)
        .await
        .map_err(|_| PackError::NotFound {
            path: source_path.display().to_string(),
        })?;
    if !meta.is_file() {
        return Err(PackError::NotFound {
            path: source_path.display().to_string(),
        });
    }
    if meta.len() > MAX_PACK_FILE_BYTES {
        return Err(PackError::FileTooLarge {
            bytes: meta.len(),
            cap: MAX_PACK_FILE_BYTES,
        });
    }

    // ── (5) Collision check (D-LOCK refuse -- 1:1 ledger/disk invariant) ──────
    let dest_dir = paths.instance_packs_dir(slug, kind);
    let dest_path = dest_dir.join(&dest_filename);
    if tokio::fs::try_exists(&dest_path).await.unwrap_or(false) {
        return Err(PackError::FilenameCollision);
    }

    // ── (6) Cancel check before any side-effect ───────────────────────────────
    if token.is_cancelled() {
        return Err(PackError::Cancelled);
    }

    // ── (7) Defensive create_dir_all (Pitfall 3 -- old instances) ─────────────
    tokio::fs::create_dir_all(&dest_dir).await.map_err(|e| {
        PackError::Io(std::io::Error::other(format!(
            "create_dir_all {}: {e}",
            dest_dir.display()
        )))
    })?;

    // ── (8) Cancel-aware read + write ─────────────────────────────────────────
    // We read the entire source file into memory (up to 500 MB at v1 scale;
    // average pack is <50 MB). This enables clean cancel-via-select semantics.
    // Per 11-03-PLAN.md action: "read full source bytes (tokio::fs::read)".
    let source_bytes_fut = tokio::fs::read(source_path);

    let bytes = tokio::select! {
        biased;
        _ = token.cancelled() => {
            return Err(PackError::Cancelled);
        }
        res = source_bytes_fut => {
            res.map_err(|e| PackError::Io(std::io::Error::other(
                format!("read source {}: {e}", source_path.display())
            )))?
        }
    };

    if token.is_cancelled() {
        return Err(PackError::Cancelled);
    }

    // ── Hash the source bytes (already in memory) ────────────────────────────
    // SHA-1 stored in `sha512` field (Phase 9 carve-out: field name is
    // historical; `hash_algo: HashAlgo::Sha1` is the discriminator).
    let mut h = Sha1::new();
    h.update(&bytes);
    let sha1 = sha1_hex(h.finalize().as_slice());

    // ── Write to dest with cancel-aware select ───────────────────────────────
    let write_fut = tokio::fs::write(&dest_path, &bytes);
    let write_result = tokio::select! {
        biased;
        _ = token.cancelled() => {
            // Best-effort cleanup: remove any partial write.
            let _ = tokio::fs::remove_file(&dest_path).await;
            return Err(PackError::Cancelled);
        }
        res = write_fut => res,
    };
    write_result.map_err(|e| {
        PackError::Io(std::io::Error::other(format!(
            "write dest {}: {e}",
            dest_path.display()
        )))
    })?;

    // ── (9) Build ledger row ──────────────────────────────────────────────────
    // mod_id convention for local packs: "local:{first-16-chars-of-sha1-hex}"
    // mirrors the "manual:{sha512_short}" convention for manual mods.
    let mod_id = format!("local:{}", &sha1[..16]);
    // project_slug = filename without ".zip" suffix (case-insensitive strip).
    let slug_part = strip_zip_suffix_case_insensitive(&dest_filename).to_string();

    let row = InstalledModRow {
        mod_id: mod_id.clone(),
        project_slug: slug_part,
        display_name: dest_filename.clone(),
        version_id: String::new(),
        version_label: "local".to_string(),
        file_name: dest_filename.clone(),
        sha512: sha1.clone(), // Phase 9 carve-out: field name historical
        size: meta.len(),
        hash_algo: HashAlgo::Sha1,
        source: ModSource::Local,
        enabled: true,
        installed_at: now_rfc3339(),
        kind: kind.into_installed_item_kind(),
    };

    // ── (10) upsert_pack -- ledger lock acquired internally ───────────────────
    upsert_pack(paths, slug, row.clone())
        .await
        .map_err(PackError::Modrinth)?;

    Ok(DropPackOutcome {
        row,
        dest: dest_path,
    })
}

/// Strip the `.zip` suffix case-insensitively from a filename.
///
/// Returns the slice up to (but not including) the last 4 chars when those
/// chars match `.zip` case-insensitively; otherwise returns the full string.
fn strip_zip_suffix_case_insensitive(s: &str) -> &str {
    if s.len() >= 4 && s[s.len() - 4..].eq_ignore_ascii_case(".zip") {
        &s[..s.len() - 4]
    } else {
        s
    }
}

/// Format SHA-1 digest as lowercase hex (40 chars).
fn sha1_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(40), |mut s, b| {
        use std::fmt::Write;
        write!(s, "{b:02x}").unwrap();
        s
    })
}

/// Current time as an RFC3339 string (UTC).
///
/// Uses the `time` crate (added Phase 2; `time 0.3` with `std` + `formatting`
/// features). Mirrors `src/domain/instance::now_iso8601_utc()`.
fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let dt = time::OffsetDateTime::from_unix_timestamp(secs as i64)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    dt.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| format!("{secs}"))
}

// ============================================================================
// === Tests                                                               ===
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    use tempfile::TempDir;

    use crate::mods::types::{InstalledItemKind, ModSource};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn test_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    fn fresh_token() -> CancellationToken {
        CancellationToken::new()
    }

    /// Write `content` bytes to `<dir>/<name>` and return the full path.
    fn write_file(dir: &std::path::Path, name: &str, content: &[u8]) -> PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(content).unwrap();
        p
    }

    // ── Task 1 -- Validation gate tests ───────────────────────────────────────

    #[tokio::test]
    async fn test_drop_rejects_nonexistent_path() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let nonexistent = td.path().join("does_not_exist.zip");
        let res = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &nonexistent,
            &fresh_token(),
        )
        .await;
        match res {
            Err(PackError::NotFound { path }) => {
                assert!(path.contains("does_not_exist.zip"), "path missing: {path}");
            }
            other => panic!("expected NotFound, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_drop_rejects_non_zip_extension() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src = write_file(td.path(), "pack.tar.gz", b"not a zip");
        let res = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src,
            &fresh_token(),
        )
        .await;
        assert!(
            matches!(res, Err(PackError::NotAZip { .. })),
            "expected NotAZip, got: {res:?}"
        );
    }

    #[tokio::test]
    async fn test_drop_accepts_uppercase_zip_extension() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let src = write_file(src_td.path(), "Pack.ZIP", b"fake zip bytes");
        let res = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src,
            &fresh_token(),
        )
        .await;
        // Should succeed (not fail with NotAZip or UnsafeFilename).
        // The result may be Ok or fail on something else (not NotAZip).
        assert!(
            !matches!(res, Err(PackError::NotAZip { .. })),
            "uppercase .ZIP should be accepted, got: {res:?}"
        );
        assert!(
            !matches!(res, Err(PackError::UnsafeFilename { .. })),
            "uppercase .ZIP should not trigger UnsafeFilename: {res:?}"
        );
    }

    #[tokio::test]
    async fn test_drop_rejects_size_over_cap() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let src_path = src_td.path().join("big.zip");
        {
            // Create a sparse file larger than the cap using set_len.
            let f = std::fs::File::create(&src_path).unwrap();
            f.set_len(MAX_PACK_FILE_BYTES + 1).unwrap();
        }
        let res = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src_path,
            &fresh_token(),
        )
        .await;
        match res {
            Err(PackError::FileTooLarge { bytes, cap }) => {
                assert_eq!(bytes, MAX_PACK_FILE_BYTES + 1);
                assert_eq!(cap, MAX_PACK_FILE_BYTES);
            }
            other => panic!("expected FileTooLarge, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_drop_rejects_unsafe_filename() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        // .hidden.zip starts with '.' -- is_safe_pack_filename rejects it.
        let src = write_file(src_td.path(), ".hidden.zip", b"data");
        let res = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src,
            &fresh_token(),
        )
        .await;
        assert!(
            matches!(res, Err(PackError::UnsafeFilename { .. })),
            "expected UnsafeFilename, got: {res:?}"
        );
    }

    #[tokio::test]
    async fn test_drop_rejects_path_traversal_in_filename() {
        // Path::file_name() returns None for paths whose last component is "..".
        // Constructing such a path: e.g. "/foo/../" has file_name() == None.
        // We test that such an input is rejected with NotFound.
        let path_with_trailing_dotdot = Path::new("/tmp/foo/..");
        assert!(
            path_with_trailing_dotdot.file_name().is_none(),
            "sanity: Path::file_name() should be None for /tmp/foo/.."
        );

        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let res = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            path_with_trailing_dotdot,
            &fresh_token(),
        )
        .await;
        assert!(
            matches!(res, Err(PackError::NotFound { .. })),
            "expected NotFound for path traversal input, got: {res:?}"
        );
    }

    #[tokio::test]
    async fn test_drop_dest_dir_created_defensively() {
        // Source is a valid .zip file; instance dir exists but NOT
        // .minecraft/resourcepacks. drop_pack_from_path should create it.
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let src = write_file(src_td.path(), "Faithful.zip", b"fake zip");

        // Confirm that the resourcepacks dir does NOT exist yet.
        let packs_dir = paths.instance_packs_dir("my-instance", PackKind::Resource);
        assert!(!packs_dir.exists(), "packs dir should not pre-exist");

        // Should succeed (creates the dir and writes the file).
        let res = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src,
            &fresh_token(),
        )
        .await;
        assert!(res.is_ok(), "expected Ok, got: {res:?}");
        assert!(packs_dir.exists(), "packs dir should have been created");
    }

    // ── Task 2 -- Happy path / collision / cancel tests ─────────────────────

    #[tokio::test]
    async fn test_drop_happy_path_resource_pack() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let content = b"fake resource pack zip content";
        let src = write_file(src_td.path(), "Faithful32x.zip", content);

        let outcome = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src,
            &fresh_token(),
        )
        .await
        .expect("drop should succeed");

        // Dest file present with correct bytes.
        let dest_bytes = std::fs::read(&outcome.dest).unwrap();
        assert_eq!(dest_bytes, content);

        // Check ledger row fields.
        let row = &outcome.row;
        assert_eq!(row.kind, InstalledItemKind::ResourcePack);
        assert_eq!(row.source, ModSource::Local);
        assert_eq!(row.hash_algo, HashAlgo::Sha1);
        assert_eq!(row.file_name, "Faithful32x.zip");
        assert!(row.mod_id.starts_with("local:"), "mod_id: {}", row.mod_id);
        assert!(row.enabled);
        assert_eq!(row.version_label, "local");

        // sha512 field holds SHA-1 hex.
        use sha1::Digest as _;
        let expected_sha1 = sha1_hex(sha1::Sha1::digest(content).as_slice());
        assert_eq!(row.sha512, expected_sha1);
    }

    #[tokio::test]
    async fn test_drop_happy_path_shader_pack() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let src = write_file(src_td.path(), "BSL_Shaders.zip", b"shader pack data");

        let outcome = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Shader,
            &src,
            &fresh_token(),
        )
        .await
        .expect("drop should succeed for shader");

        assert_eq!(outcome.row.kind, InstalledItemKind::Shader);
        assert!(
            outcome.dest.to_string_lossy().contains("shaderpacks"),
            "dest should be in shaderpacks, got: {}",
            outcome.dest.display()
        );
    }

    #[tokio::test]
    async fn test_drop_collision_after_first_install() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let src = write_file(src_td.path(), "Faithful32x.zip", b"content");

        // First install succeeds.
        let first = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src,
            &fresh_token(),
        )
        .await;
        assert!(first.is_ok(), "first install should succeed");

        // Second install with same filename → FilenameCollision.
        let second = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src,
            &fresh_token(),
        )
        .await;
        assert!(
            matches!(second, Err(PackError::FilenameCollision)),
            "second install should fail with FilenameCollision, got: {second:?}"
        );
    }

    #[tokio::test]
    async fn test_drop_cancel_before_copy() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let src = write_file(src_td.path(), "Faithful.zip", b"content");

        let token = CancellationToken::new();
        token.cancel(); // Pre-cancel.

        let res =
            drop_pack_from_path(&paths, "my-instance", PackKind::Resource, &src, &token).await;
        assert!(
            matches!(res, Err(PackError::Cancelled)),
            "expected Cancelled, got: {res:?}"
        );
        // Dest file must NOT exist.
        let dest_path = paths.instance_pack_file("my-instance", PackKind::Resource, "Faithful.zip");
        assert!(
            !dest_path.exists(),
            "dest should not exist after pre-cancel"
        );
    }

    #[tokio::test]
    async fn test_drop_cancel_mid_copy_removes_partial_file() {
        // Write a 10 MB source file so the tokio::select! has a chance to see
        // cancellation during the write future.
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let big_content = vec![0u8; 10 * 1024 * 1024]; // 10 MB
        let src = write_file(src_td.path(), "BigPack.zip", &big_content);

        let token = CancellationToken::new();
        let token_clone = token.clone();

        let paths_clone = paths.clone();
        let handle = tokio::spawn(async move {
            drop_pack_from_path(
                &paths_clone,
                "my-instance",
                PackKind::Resource,
                &src,
                &token_clone,
            )
            .await
        });

        // Cancel after a brief delay to allow the task to start.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        token.cancel();

        let res = handle.await.unwrap();
        // Either Cancelled (if the select caught it) or Ok (if copy completed
        // before cancel fired) -- both are valid. What matters is that if the
        // result is Cancelled, the partial dest file must not exist.
        if matches!(res, Err(PackError::Cancelled)) {
            let dest_path =
                paths.instance_pack_file("my-instance", PackKind::Resource, "BigPack.zip");
            assert!(
                !tokio::fs::try_exists(&dest_path).await.unwrap_or(true),
                "partial file should be cleaned up after cancel"
            );
        }
        // If Ok -- copy completed before cancel; that's fine too.
    }

    #[tokio::test]
    async fn test_drop_local_mod_id_starts_with_local_prefix() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let src = write_file(src_td.path(), "TestPack.zip", b"test data");

        let outcome = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src,
            &fresh_token(),
        )
        .await
        .unwrap();

        // mod_id must match ^local:[0-9a-f]{16}$
        let mod_id = &outcome.row.mod_id;
        assert!(mod_id.starts_with("local:"), "mod_id: {mod_id}");
        let hex_part = &mod_id["local:".len()..];
        assert_eq!(hex_part.len(), 16, "expected 16 hex chars, got: {hex_part}");
        assert!(
            hex_part.chars().all(|c| c.is_ascii_hexdigit()),
            "hex part should be lowercase hex: {hex_part}"
        );
    }

    #[tokio::test]
    async fn test_drop_local_source_field() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let src = write_file(src_td.path(), "AnyPack.zip", b"bytes");

        let outcome = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src,
            &fresh_token(),
        )
        .await
        .unwrap();

        assert_eq!(outcome.row.source, ModSource::Local);
    }

    #[tokio::test]
    async fn test_drop_writes_safe_pack_filename_with_space() {
        // D-LOCK: packs allow spaces (e.g. "Faithful 32x.zip").
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let src_td = TempDir::new().unwrap();
        let src = write_file(src_td.path(), "Faithful 32x.zip", b"pack data");

        let res = drop_pack_from_path(
            &paths,
            "my-instance",
            PackKind::Resource,
            &src,
            &fresh_token(),
        )
        .await;
        assert!(res.is_ok(), "filename with space should succeed: {res:?}");
        let outcome = res.unwrap();
        assert_eq!(outcome.row.file_name, "Faithful 32x.zip");
    }

    // ── strip_zip_suffix_case_insensitive ────────────────────────────────────

    #[test]
    fn test_strip_zip_suffix_lowercase() {
        assert_eq!(
            strip_zip_suffix_case_insensitive("Faithful.zip"),
            "Faithful"
        );
    }

    #[test]
    fn test_strip_zip_suffix_uppercase() {
        assert_eq!(strip_zip_suffix_case_insensitive("Pack.ZIP"), "Pack");
    }

    #[test]
    fn test_strip_zip_suffix_no_zip() {
        assert_eq!(
            strip_zip_suffix_case_insensitive("file.tar.gz"),
            "file.tar.gz"
        );
    }
}
