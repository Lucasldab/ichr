//! Streaming mod-file download + SHA-512 verify + parallel install pipeline.
//!
//! Mirrors `src/loader/service.rs::install_loader_impl` (Step 2 lines 305-377)
//! and `download_one_library` (lines 498-574), with these deltas per
//! 08-RESEARCH.md §Pattern 6:
//! - sha2::Sha512 (not sha1) -- Modrinth's primary file hash
//! - bytes_stream() per-chunk hashing -> progress emission per chunk
//! - MOD_DOWNLOAD_CONCURRENCY = 6 (vs. LIB_CONCURRENCY = 8) -- Modrinth CDN
//!   is shared infrastructure; conservative under the 300 req/min anonymous cap
//! - MAX_MOD_FILE_BYTES = 256 MB cap (Security Domain V12 -- DoS defense)
//! - tmp -> ledger upsert -> rename atomicity (Pitfall 8 + anti-pattern lines 869-870)
//!
//! ASSUMPTION A1 -- verify in human checkpoint.
//! ASSUMPTION A5 (256 MB cap) -- verify in human checkpoint.

use std::sync::Arc;

use futures::StreamExt;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::mods::error::ModrinthError;
use crate::mods::filter::{is_safe_mod_filename, pick_primary_file};
use crate::mods::ledger::{per_instance_lock, upsert_mod};
use crate::mods::modrinth::client::MAX_MOD_FILE_BYTES;
use crate::mods::types::{
    DepKind, HashAlgo, InstalledModRow, ModSource, ModrinthFile, ModrinthVersion, ResolvedDep,
};
use crate::persistence::paths::AppPaths;
use crate::tasks::{JobId, TaskEvent};

/// Conservative cap on parallel mod-file downloads within a single install job.
/// Below Phase 2 LIB_CONCURRENCY (8) because Modrinth CDN is shared infrastructure
/// and 6 leaves headroom under the 300 req/min anonymous cap.
pub const MOD_DOWNLOAD_CONCURRENCY: usize = 6;

/// Stream a mod-file body, hash with SHA-512 on the fly, write the verified
/// bytes to `dest_tmp`. The caller is responsible for the final
/// `dest_tmp -> final.jar` rename, performed AFTER the ledger upsert per
/// the Pitfall 8 atomicity protocol.
///
/// `completed_files` is the count of files already finished BEFORE this one
/// starts; `total_files` is the total install plan size. Used purely for
/// per-chunk progress emission.
///
/// Returns the verified file size in bytes.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(
    name = "mods::download_and_verify",
    skip_all,
    fields(filename = %file.filename, size = file.size)
)]
pub async fn download_and_verify(
    http: &reqwest::Client,
    file: &ModrinthFile,
    dest_tmp: &std::path::Path,
    progress_tx: &mpsc::Sender<TaskEvent>,
    job_id: JobId,
    token: &CancellationToken,
    completed_files: usize,
    total_files: usize,
) -> Result<u64, ModrinthError> {
    // V5 input validation -- filename gate BEFORE building dest path / network call.
    if !is_safe_mod_filename(&file.filename) {
        return Err(ModrinthError::Io(std::io::Error::other(format!(
            "unsafe filename: {}",
            file.filename
        ))));
    }
    // V8 -- HTTPS only. Loopback (127.0.0.1, localhost) is permitted via
    // http:// to support mock servers in tests; Modrinth's CDN is always
    // HTTPS in production, so this exemption has no effect on real traffic.
    if !is_acceptable_mod_url(&file.url) {
        return Err(ModrinthError::FileNotDownloadable {
            project_slug: format!("{} (non-https URL: {})", file.filename, file.url),
        });
    }
    // V12 -- early reject advertised-oversize files BEFORE issuing GET.
    if file.size > MAX_MOD_FILE_BYTES {
        return Err(ModrinthError::FileNotDownloadable {
            project_slug: format!(
                "{} ({}B exceeds cap {}B)",
                file.filename, file.size, MAX_MOD_FILE_BYTES
            ),
        });
    }
    if token.is_cancelled() {
        return Err(ModrinthError::Cancelled);
    }

    let resp = http
        .get(&file.url)
        .send()
        .await
        .map_err(|e| ModrinthError::Http(format!("GET {}: {e}", file.url)))?
        .error_for_status()
        .map_err(|e| ModrinthError::Http(format!("status {}: {e}", file.url)))?;

    let mut hasher = Sha512::new();
    let mut buf: Vec<u8> = Vec::with_capacity(file.size as usize);
    let mut bytes_done: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        if token.is_cancelled() {
            return Err(ModrinthError::Cancelled);
        }
        let chunk = chunk
            .map_err(|e| ModrinthError::Http(format!("body {}: {e}", file.url)))?;
        bytes_done += chunk.len() as u64;
        // Defense in depth -- abort if mid-stream bytes exceed cap (tampered Content-Length).
        if bytes_done > MAX_MOD_FILE_BYTES {
            return Err(ModrinthError::FileNotDownloadable {
                project_slug: format!(
                    "{} (mid-stream bytes {}B exceeded cap {}B)",
                    file.filename, bytes_done, MAX_MOD_FILE_BYTES
                ),
            });
        }
        hasher.update(&chunk);
        buf.extend_from_slice(&chunk);

        // Per-chunk progress emit. Each file gets `100 / total_files` percentage points;
        // the chunk contributes a proportional share of that.
        let intra_pct: u64 = if file.size == 0 {
            100
        } else {
            (bytes_done.saturating_mul(100)) / file.size.max(1)
        };
        let total = total_files.max(1) as u64;
        let already_done_pct: u64 = (completed_files as u64 * 100) / total;
        let this_share_pct: u64 = intra_pct / total;
        let pct = (already_done_pct + this_share_pct).min(99) as u8;
        // Best-effort emit; ignore mpsc closed (TUI may have backed off).
        let _ = progress_tx
            .send(TaskEvent::Progress {
                id: job_id,
                pct,
                msg: format!(
                    "Downloading mod {}/{}: {}",
                    completed_files + 1,
                    total_files,
                    file.filename
                ),
            })
            .await;
    }

    // Verify (case-insensitive -- Pitfall 3).
    // Note: sha2 0.11 returns Array<u8, _> which does NOT implement LowerHex
    // (Phase 05 decision: use iter().fold() instead of {:x}). See
    // src/java/adoptium.rs::sha256_hex for the canonical pattern.
    let got = sha512_hex(hasher.finalize().as_slice());
    if !got.eq_ignore_ascii_case(&file.hashes.sha512) {
        return Err(ModrinthError::Sha512Mismatch {
            url: file.url.clone(),
            expected: file.hashes.sha512.clone(),
            got,
        });
    }

    // Write the verified bytes to `dest_tmp`. We do NOT call
    // `mojang::cache::atomic_write` here because that helper writes to
    // `dest.with_extension("tmp")` and renames -- when the caller-supplied
    // `dest_tmp` already ends in `.tmp` (our convention), `with_extension`
    // returns the same path and the rename is a no-op. Caller controls the
    // tmp -> final.jar rename after the ledger upsert (Pitfall 8 protocol).
    if let Some(parent) = dest_tmp.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            ModrinthError::Io(std::io::Error::other(format!(
                "create_dir_all {}: {e}",
                parent.display()
            )))
        })?;
    }
    tokio::fs::write(dest_tmp, &buf).await.map_err(|e| {
        ModrinthError::Io(std::io::Error::other(format!(
            "write {}: {e}",
            dest_tmp.display()
        )))
    })?;
    Ok(buf.len() as u64)
}

/// True iff the URL is acceptable for a mod-file download.
///
/// Production rule: must be `https://` -- Modrinth's CDN is always HTTPS,
/// so a non-HTTPS URL in an API response indicates tampering (V8 defense
/// in depth).
///
/// Test exemption: `http://127.0.0.1:*` and `http://localhost:*` are
/// permitted so that httpmock-backed unit tests can serve mod jars over
/// loopback. This exemption is invisible in production: real Modrinth
/// responses never contain loopback URLs.
fn is_acceptable_mod_url(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:")
}

/// SHA-512 lowercase hex (128 chars) of a digest output.
///
/// `sha2` 0.11 returns `Array<u8, _>` which does not implement `LowerHex`
/// (see Phase 05 `sha256_hex` decision). We hand-format via `iter().fold()`
/// matching `src/java/adoptium.rs::sha256_hex`.
fn sha512_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(128), |mut s, b| {
        use std::fmt::Write;
        write!(s, "{b:02x}").unwrap();
        s
    })
}

/// SHA-1 lowercase hex (40 chars) of a digest output. Mirrors `sha512_hex`
/// shape (sha2 0.11 Array does not implement LowerHex; iter().fold()).
/// Per Phase 9 09-RESEARCH.md §Pattern 5 line 622 (CurseForge default hash).
pub fn sha1_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(40), |mut s, b| {
        use std::fmt::Write;
        write!(s, "{b:02x}").unwrap();
        s
    })
}

/// SHA-256 lowercase hex (64 chars). Same pattern; rare in CurseForge but
/// supported as a fallback for files carrying algo=3 instead of algo=1.
pub fn sha256_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write;
        write!(s, "{b:02x}").unwrap();
        s
    })
}

/// Stream a file with hash verification driven by `HashAlgo`. Sibling to
/// `download_and_verify` (which is hardcoded SHA-512 for the Modrinth path).
/// Per Phase 9 09-PATTERNS.md §"Generalization delta" line 657 + 09-RESEARCH.md
/// §"Per-Instance Ledger Reuse" line 312.
///
/// Preserves the Phase 8 invariants from `download_and_verify`:
///   - 256MB cap (pre- and mid-stream)
///   - safe-filename allowlist via `is_safe_mod_filename`
///   - HTTPS-only URL acceptability (loopback exemption for httpmock tests)
///   - per-chunk progress emit
///   - cancellation check before send and per-chunk
///   - case-insensitive hash compare (Pitfall 3)
///
/// On hash mismatch returns `ModrinthError::Sha512Mismatch` (kept under that
/// variant for ModrinthError stability; the Phase 9 service-layer maps to
/// `CurseForgeError::ShaMismatch { algo, ... }` at the boundary).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(
    name = "mods::download_one_with_hash_algo",
    skip_all,
    fields(url = %url, file_label = %file_label, hash_algo = ?hash_algo)
)]
pub async fn download_one_with_hash_algo(
    http: &reqwest::Client,
    url: &str,
    expected_hash: &str,
    hash_algo: HashAlgo,
    dest_tmp: &std::path::Path,
    file_label: &str,
    file_size_hint: u64,
    progress_tx: &mpsc::Sender<TaskEvent>,
    job_id: JobId,
    token: &CancellationToken,
    completed_files: usize,
    total_files: usize,
) -> Result<u64, ModrinthError> {
    if !is_safe_mod_filename(file_label) {
        return Err(ModrinthError::Io(std::io::Error::other(format!(
            "unsafe filename: {file_label}"
        ))));
    }
    if !is_acceptable_mod_url(url) {
        return Err(ModrinthError::FileNotDownloadable {
            project_slug: format!("{file_label} (non-https URL: {url})"),
        });
    }
    if file_size_hint > MAX_MOD_FILE_BYTES {
        return Err(ModrinthError::FileNotDownloadable {
            project_slug: format!(
                "{file_label} ({file_size_hint}B exceeds cap {MAX_MOD_FILE_BYTES}B)"
            ),
        });
    }
    if token.is_cancelled() {
        return Err(ModrinthError::Cancelled);
    }

    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| ModrinthError::Http(format!("GET {url}: {e}")))?
        .error_for_status()
        .map_err(|e| ModrinthError::Http(format!("status {url}: {e}")))?;

    let mut buf: Vec<u8> = Vec::with_capacity(file_size_hint as usize);
    let mut bytes_done: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        if token.is_cancelled() {
            return Err(ModrinthError::Cancelled);
        }
        let chunk = chunk
            .map_err(|e| ModrinthError::Http(format!("body {url}: {e}")))?;
        bytes_done += chunk.len() as u64;
        if bytes_done > MAX_MOD_FILE_BYTES {
            return Err(ModrinthError::FileNotDownloadable {
                project_slug: format!(
                    "{file_label} (mid-stream bytes {bytes_done}B exceeded cap {MAX_MOD_FILE_BYTES}B)"
                ),
            });
        }
        buf.extend_from_slice(&chunk);

        let intra_pct: u64 = if file_size_hint == 0 {
            100
        } else {
            (bytes_done.saturating_mul(100)) / file_size_hint.max(1)
        };
        let total = total_files.max(1) as u64;
        let already_done_pct: u64 = (completed_files as u64 * 100) / total;
        let this_share_pct: u64 = intra_pct / total;
        let pct = (already_done_pct + this_share_pct).min(99) as u8;
        let _ = progress_tx
            .send(TaskEvent::Progress {
                id: job_id,
                pct,
                msg: format!(
                    "Downloading mod {}/{}: {file_label}",
                    completed_files + 1,
                    total_files
                ),
            })
            .await;
    }

    // Verify: branch on HashAlgo. Case-insensitive compare per Pitfall 3.
    //
    // Important: `sha1` 0.10 uses `digest` 0.10 while `sha2` 0.11 uses `digest`
    // 0.11 (CLAUDE.md compatibility note: "they coexist but you cannot unify
    // the digest trait across them. Call them independently"). The module-level
    // `use sha2::{Digest, Sha512}` puts `sha2::Digest` (digest 0.11) in scope —
    // sha2 hashers (Sha512, Sha256) work directly. For Sha1 we scope-import
    // `sha1::Digest` inside the arm so its method resolution wins locally.
    let got = match hash_algo {
        HashAlgo::Sha512 => {
            let mut h = Sha512::new();
            h.update(&buf);
            sha512_hex(h.finalize().as_slice())
        }
        HashAlgo::Sha1 => {
            use sha1::Digest as _;
            let mut h = Sha1::new();
            h.update(&buf);
            sha1_hex(h.finalize().as_slice())
        }
        HashAlgo::Sha256 => {
            use sha2::Sha256;
            let mut h = Sha256::new();
            h.update(&buf);
            sha256_hex(h.finalize().as_slice())
        }
    };
    if !got.eq_ignore_ascii_case(expected_hash) {
        return Err(ModrinthError::Sha512Mismatch {
            url: url.to_string(),
            expected: expected_hash.to_string(),
            got,
        });
    }

    if let Some(parent) = dest_tmp.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            ModrinthError::Io(std::io::Error::other(format!(
                "create_dir_all {}: {e}",
                parent.display()
            )))
        })?;
    }
    tokio::fs::write(dest_tmp, &buf).await.map_err(|e| {
        ModrinthError::Io(std::io::Error::other(format!(
            "write {}: {e}",
            dest_tmp.display()
        )))
    })?;
    Ok(buf.len() as u64)
}

/// One element of the install plan -- a row to write into the ledger and the
/// file to fetch.
#[derive(Debug, Clone)]
pub struct InstallStep {
    pub row: InstalledModRow,
    pub file: ModrinthFile,
}

/// Build the ordered install plan from the resolved dep graph.
/// Includes the root version + every required-and-not-already-satisfied dep.
/// Returns `Err(FileNotDownloadable)` if any candidate has no primary file.
pub fn build_install_plan(
    root_version: &ModrinthVersion,
    root_project_slug: &str,
    root_project_title: &str,
    deps: &[ResolvedDep],
    now_iso8601: impl Fn() -> String,
) -> Result<Vec<InstallStep>, ModrinthError> {
    let mut steps: Vec<InstallStep> = Vec::new();

    // Root first.
    let root_file = pick_primary_file(&root_version.files)
        .ok_or_else(|| ModrinthError::FileNotDownloadable {
            project_slug: root_project_slug.to_string(),
        })?
        .clone();
    steps.push(InstallStep {
        row: InstalledModRow {
            mod_id: root_version.project_id.clone(),
            project_slug: root_project_slug.to_string(),
            display_name: root_project_title.to_string(),
            version_id: root_version.id.clone(),
            version_label: root_version.version_number.clone(),
            file_name: root_file.filename.clone(),
            sha512: root_file.hashes.sha512.clone(),
            size: root_file.size,
            hash_algo: HashAlgo::Sha512,
            source: ModSource::Modrinth,
            enabled: true,
            installed_at: now_iso8601(),
        },
        file: root_file,
    });

    // Then each new-download required dep.
    for d in deps {
        if !matches!(d.kind, DepKind::Required) || !d.is_new_download {
            continue;
        }
        let Some(ver) = &d.version else { continue };
        let f = pick_primary_file(&ver.files)
            .ok_or_else(|| ModrinthError::FileNotDownloadable {
                project_slug: d.project_id.clone(),
            })?
            .clone();
        steps.push(InstallStep {
            row: InstalledModRow {
                mod_id: d.project_id.clone(),
                // Best-effort: use project_id as slug when the resolver did not
                // populate a friendly title/slug. The 08-04 resolver leaves
                // project_title empty for non-root entries; the UI can re-fetch
                // the title via /v2/project on demand if needed.
                project_slug: d.project_id.clone(),
                display_name: if d.project_title.is_empty() {
                    d.project_id.clone()
                } else {
                    d.project_title.clone()
                },
                version_id: ver.id.clone(),
                version_label: ver.version_number.clone(),
                file_name: f.filename.clone(),
                sha512: f.hashes.sha512.clone(),
                size: f.size,
                hash_algo: HashAlgo::Sha512,
                source: ModSource::Modrinth,
                enabled: true,
                installed_at: now_iso8601(),
            },
            file: f,
        });
    }

    Ok(steps)
}

/// Parallel install pipeline.
///
/// For each step in `plan`:
///   1. Acquire a permit from the bounded `Semaphore` (cap = `MOD_DOWNLOAD_CONCURRENCY`).
///   2. Cancel check.
///   3. Download to `<file>.tmp` with sha512 verify.
///   4. `upsert_mod` (acquires the per-instance ledger lock internally).
///   5. Rename `.tmp` -> final `.jar` (atomic on same FS).
///   6. Drop permit.
///
/// On any failure or cancellation: `set.abort_all()`, attempt to clean up partial
/// `.tmp` files on the failing task's path (best-effort), return the error.
#[tracing::instrument(
    name = "mods::install_mods_into_instance",
    skip_all,
    fields(slug = %slug, plan_size = plan.len(), job_id = job_id.0)
)]
pub async fn install_mods_into_instance(
    http: reqwest::Client,
    paths: AppPaths,
    slug: String,
    plan: Vec<InstallStep>,
    progress_tx: mpsc::Sender<TaskEvent>,
    token: CancellationToken,
    job_id: JobId,
) -> Result<(), ModrinthError> {
    if token.is_cancelled() {
        return Err(ModrinthError::Cancelled);
    }
    let total = plan.len();
    if total == 0 {
        // No mods to install (caller is expected to pass at least the root step).
        let _ = progress_tx
            .send(TaskEvent::Progress {
                id: job_id,
                pct: 100,
                msg: "No mods to install".into(),
            })
            .await;
        return Ok(());
    }

    let sem = Arc::new(Semaphore::new(MOD_DOWNLOAD_CONCURRENCY));
    let mut set = tokio::task::JoinSet::new();

    for (i, step) in plan.iter().cloned().enumerate() {
        let sem = Arc::clone(&sem);
        let http = http.clone();
        let paths = paths.clone();
        let slug = slug.clone();
        let progress_tx = progress_tx.clone();
        let token = token.clone();

        set.spawn(async move {
            let _permit = sem
                .acquire_owned()
                .await
                .map_err(|e| {
                    ModrinthError::Io(std::io::Error::other(format!(
                        "semaphore closed: {e}"
                    )))
                })?;
            if token.is_cancelled() {
                return Err(ModrinthError::Cancelled);
            }

            let final_path = paths.instance_mod_file(&slug, &step.file.filename);
            let tmp_path = {
                let mut s = final_path.clone().into_os_string();
                s.push(".tmp");
                std::path::PathBuf::from(s)
            };

            // 1. Download to .tmp with sha512 verify.
            let dl = download_and_verify(
                &http,
                &step.file,
                &tmp_path,
                &progress_tx,
                job_id,
                &token,
                i,
                total,
            )
            .await;
            if let Err(e) = dl {
                // Best-effort cleanup of any partial .tmp on this task's path.
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(e);
            }

            // 2. Upsert ledger row. `upsert_mod` acquires the per-instance lock
            //    internally (Pitfall 8); ledger row lands BEFORE the rename so a
            //    rename failure leaves the .tmp on disk + the ledger consistent
            //    with prior state when followed by post-error cleanup at retry.
            if let Err(e) = upsert_mod(&paths, &slug, step.row.clone()).await {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(e);
            }

            // 3. Rename .tmp -> final.jar (atomic on same FS).
            if let Err(e) = tokio::fs::rename(&tmp_path, &final_path).await {
                // Best-effort cleanup of the orphan .tmp (sync; not in async ctx
                // for the error variant).
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(ModrinthError::Io(std::io::Error::other(format!(
                    "rename {} -> {}: {e}",
                    tmp_path.display(),
                    final_path.display(),
                ))));
            }

            Ok::<usize, ModrinthError>(i)
        });
    }

    let mut completed: usize = 0;
    while let Some(res) = set.join_next().await {
        if token.is_cancelled() {
            set.abort_all();
            return Err(ModrinthError::Cancelled);
        }
        match res {
            Ok(Ok(_idx)) => {
                completed += 1;
                let pct = ((completed as u64 * 100) / total as u64) as u8;
                let _ = progress_tx
                    .send(TaskEvent::Progress {
                        id: job_id,
                        pct: pct.min(100),
                        msg: format!("Installed {completed}/{total} mods"),
                    })
                    .await;
            }
            Ok(Err(e)) => {
                set.abort_all();
                return Err(e);
            }
            Err(join_err) if join_err.is_cancelled() => {
                set.abort_all();
                return Err(ModrinthError::Cancelled);
            }
            Err(join_err) => {
                set.abort_all();
                return Err(ModrinthError::Io(std::io::Error::other(format!(
                    "install task panicked: {join_err}"
                ))));
            }
        }
    }

    // Touch the per-instance lock map (no-op; ensures the entry exists for the slug).
    // The lock is created lazily by ledger ops so this is purely cosmetic.
    let _ = per_instance_lock(&slug);

    Ok(())
}

// ============================================================================
// === Tests                                                                ===
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::types::ModrinthHashes;
    use httpmock::prelude::*;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn mk_file(name: &str, size: u64, sha512: &str, url: String) -> ModrinthFile {
        ModrinthFile {
            url,
            filename: name.to_string(),
            primary: true,
            size,
            hashes: ModrinthHashes {
                sha1: "".into(),
                sha512: sha512.to_string(),
            },
        }
    }

    /// Spawn a background drain so the channel never fills during tests.
    fn drain(mut rx: mpsc::Receiver<TaskEvent>) {
        tokio::spawn(async move {
            while rx.recv().await.is_some() {}
        });
    }

    #[tokio::test]
    async fn test_sha512_verify_passes_on_correct_bytes() {
        let body = b"hello world";
        let mut h = Sha512::new();
        h.update(body);
        let expected = sha512_hex(h.finalize().as_slice());

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/x.jar");
            then.status(200).body(body);
        });

        let td = TempDir::new().unwrap();
        let dest = td.path().join("x.jar.tmp");
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let (tx, rx) = mpsc::channel(8);
        drain(rx);
        let token = CancellationToken::new();
        let f = mk_file(
            "x.jar",
            body.len() as u64,
            &expected,
            format!("{}/x.jar", server.base_url()),
        );
        let n = download_and_verify(&http, &f, &dest, &tx, JobId(0), &token, 0, 1)
            .await
            .unwrap();
        assert_eq!(n, body.len() as u64);
        assert!(dest.exists());
    }

    #[tokio::test]
    async fn test_sha512_verify_fails_on_wrong_hash() {
        let body = b"hello world";
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/x.jar");
            then.status(200).body(body);
        });
        let td = TempDir::new().unwrap();
        let dest = td.path().join("x.jar.tmp");
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let (tx, rx) = mpsc::channel(8);
        drain(rx);
        let token = CancellationToken::new();
        let f = mk_file(
            "x.jar",
            body.len() as u64,
            "deadbeef",
            format!("{}/x.jar", server.base_url()),
        );
        let r = download_and_verify(&http, &f, &dest, &tx, JobId(0), &token, 0, 1).await;
        match r {
            Err(ModrinthError::Sha512Mismatch { expected, got, .. }) => {
                assert_eq!(expected, "deadbeef");
                assert!(!got.is_empty());
            }
            other => panic!("expected Sha512Mismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_sha512_compare_is_case_insensitive() {
        // Pitfall 3 -- Modrinth returns lowercase hex; verify must not depend on case.
        let body = b"abc";
        let mut h = Sha512::new();
        h.update(body);
        let lower = sha512_hex(h.finalize().as_slice());
        let upper = lower.to_uppercase();

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/x.jar");
            then.status(200).body(body);
        });

        let td = TempDir::new().unwrap();
        let dest = td.path().join("x.jar.tmp");
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let (tx, rx) = mpsc::channel(8);
        drain(rx);
        let token = CancellationToken::new();
        let f = mk_file(
            "x.jar",
            body.len() as u64,
            &upper,
            format!("{}/x.jar", server.base_url()),
        );
        // Should succeed despite case mismatch.
        assert!(
            download_and_verify(&http, &f, &dest, &tx, JobId(0), &token, 0, 1)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_max_mod_file_bytes_rejects_advertised_oversize() {
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let td = TempDir::new().unwrap();
        let dest = td.path().join("big.jar.tmp");
        let (tx, rx) = mpsc::channel(8);
        drain(rx);
        let token = CancellationToken::new();
        let f = mk_file(
            "big.jar",
            MAX_MOD_FILE_BYTES + 1,
            "0",
            "https://example.invalid/big.jar".into(),
        );
        let r = download_and_verify(&http, &f, &dest, &tx, JobId(0), &token, 0, 1).await;
        assert!(matches!(r, Err(ModrinthError::FileNotDownloadable { .. })), "got {r:?}");
    }

    #[tokio::test]
    async fn test_rejects_unsafe_filename() {
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let td = TempDir::new().unwrap();
        let dest = td.path().join("x.tmp");
        let (tx, rx) = mpsc::channel(8);
        drain(rx);
        let token = CancellationToken::new();
        let f = mk_file(
            "../etc/passwd.jar",
            10,
            "0",
            "https://example.invalid/x.jar".into(),
        );
        let r = download_and_verify(&http, &f, &dest, &tx, JobId(0), &token, 0, 1).await;
        assert!(matches!(r, Err(ModrinthError::Io(_))), "got {r:?}");
    }

    #[tokio::test]
    async fn test_rejects_non_https_url() {
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let td = TempDir::new().unwrap();
        let dest = td.path().join("x.tmp");
        let (tx, rx) = mpsc::channel(8);
        drain(rx);
        let token = CancellationToken::new();
        let f = mk_file(
            "x.jar",
            10,
            "0",
            "http://insecure.example.com/x.jar".into(),
        );
        let r = download_and_verify(&http, &f, &dest, &tx, JobId(0), &token, 0, 1).await;
        assert!(matches!(r, Err(ModrinthError::FileNotDownloadable { .. })), "got {r:?}");
    }

    #[tokio::test]
    async fn test_cancelled_before_send_returns_cancelled() {
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let td = TempDir::new().unwrap();
        let dest = td.path().join("x.jar.tmp");
        let (tx, rx) = mpsc::channel(8);
        drain(rx);
        let token = CancellationToken::new();
        token.cancel();
        let f = mk_file("x.jar", 10, "0", "https://example.invalid/x.jar".into());
        let r = download_and_verify(&http, &f, &dest, &tx, JobId(0), &token, 0, 1).await;
        assert!(matches!(r, Err(ModrinthError::Cancelled)), "got {r:?}");
    }

    #[tokio::test]
    async fn test_build_install_plan_root_only() {
        let v = ModrinthVersion {
            id: "v1".into(),
            project_id: "rootp".into(),
            name: "Root 1.0".into(),
            version_number: "1.0".into(),
            version_type: "release".into(),
            game_versions: vec!["1.20.4".into()],
            loaders: vec!["fabric".into()],
            downloads: 0,
            date_published: "2026-01-01T00:00:00Z".into(),
            dependencies: vec![],
            files: vec![ModrinthFile {
                url: "https://cdn.modrinth.com/root.jar".into(),
                filename: "root.jar".into(),
                primary: true,
                size: 100,
                hashes: ModrinthHashes {
                    sha1: "a".into(),
                    sha512: "b".into(),
                },
            }],
        };
        let plan =
            build_install_plan(&v, "rootslug", "Root Title", &[], || "now".into()).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].row.mod_id, "rootp");
        assert_eq!(plan[0].row.project_slug, "rootslug");
        assert_eq!(plan[0].row.display_name, "Root Title");
        assert_eq!(plan[0].file.filename, "root.jar");
    }

    #[tokio::test]
    async fn test_build_install_plan_skips_non_required_and_already_satisfied() {
        let root = ModrinthVersion {
            id: "v1".into(),
            project_id: "rootp".into(),
            name: "Root".into(),
            version_number: "1.0".into(),
            version_type: "release".into(),
            game_versions: vec!["1.20.4".into()],
            loaders: vec!["fabric".into()],
            downloads: 0,
            date_published: "2026-01-01T00:00:00Z".into(),
            dependencies: vec![],
            files: vec![ModrinthFile {
                url: "https://cdn.modrinth.com/root.jar".into(),
                filename: "root.jar".into(),
                primary: true,
                size: 100,
                hashes: ModrinthHashes {
                    sha1: "a".into(),
                    sha512: "b".into(),
                },
            }],
        };
        let dep_v = ModrinthVersion {
            id: "depv".into(),
            project_id: "depp".into(),
            name: "Dep".into(),
            version_number: "0.5".into(),
            version_type: "release".into(),
            game_versions: vec!["1.20.4".into()],
            loaders: vec!["fabric".into()],
            downloads: 0,
            date_published: "2026-01-01T00:00:00Z".into(),
            dependencies: vec![],
            files: vec![ModrinthFile {
                url: "https://cdn.modrinth.com/dep.jar".into(),
                filename: "dep.jar".into(),
                primary: true,
                size: 50,
                hashes: ModrinthHashes {
                    sha1: "a".into(),
                    sha512: "b".into(),
                },
            }],
        };
        let deps = vec![
            // Optional -- skip.
            ResolvedDep {
                kind: DepKind::Optional,
                project_id: "opt".into(),
                project_title: "Opt".into(),
                version: None,
                already_satisfied: false,
                is_new_download: false,
            },
            // Required already satisfied -- skip.
            ResolvedDep {
                kind: DepKind::Required,
                project_id: "old".into(),
                project_title: "Old".into(),
                version: None,
                already_satisfied: true,
                is_new_download: false,
            },
            // Required new download -- include.
            ResolvedDep {
                kind: DepKind::Required,
                project_id: "depp".into(),
                project_title: "Dep Title".into(),
                version: Some(dep_v.clone()),
                already_satisfied: false,
                is_new_download: true,
            },
        ];
        let plan = build_install_plan(&root, "rootslug", "Root", &deps, || "now".into()).unwrap();
        assert_eq!(plan.len(), 2, "root + 1 new dep");
        assert_eq!(plan[0].row.mod_id, "rootp");
        assert_eq!(plan[1].row.mod_id, "depp");
        assert_eq!(plan[1].row.display_name, "Dep Title");
    }

    #[tokio::test]
    async fn test_install_mods_into_instance_root_only_writes_jar_and_ledger() {
        // End-to-end: one root step, one mod jar served, ledger row appears,
        // file appears at instance_mod_file path.
        let body = b"jar bytes here";
        let mut h = Sha512::new();
        h.update(body);
        let sha = sha512_hex(h.finalize().as_slice());

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/sodium.jar");
            then.status(200).body(body);
        });

        let td = TempDir::new().unwrap();
        let paths = AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        );
        // Pre-create the mods dir (mirrors instance creation).
        tokio::fs::create_dir_all(paths.instance_minecraft_dir("inst1").join("mods"))
            .await
            .unwrap();

        let file = ModrinthFile {
            url: format!("{}/sodium.jar", server.base_url()),
            filename: "sodium.jar".into(),
            primary: true,
            size: body.len() as u64,
            hashes: ModrinthHashes {
                sha1: "".into(),
                sha512: sha.clone(),
            },
        };
        let row = InstalledModRow {
            mod_id: "AANobbMI".into(),
            project_slug: "sodium".into(),
            display_name: "Sodium".into(),
            version_id: "v1".into(),
            version_label: "0.5.8".into(),
            file_name: "sodium.jar".into(),
            sha512: sha,
            size: body.len() as u64,
            hash_algo: HashAlgo::Sha512,
            source: ModSource::Modrinth,
            enabled: true,
            installed_at: "2026-01-01T00:00:00Z".into(),
        };
        let plan = vec![InstallStep {
            row: row.clone(),
            file,
        }];

        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let (tx, rx) = mpsc::channel(16);
        drain(rx);
        let token = CancellationToken::new();
        install_mods_into_instance(
            http,
            paths.clone(),
            "inst1".into(),
            plan,
            tx,
            token,
            JobId(7),
        )
        .await
        .unwrap();

        // Final jar present; .tmp gone.
        let final_path = paths.instance_mod_file("inst1", "sodium.jar");
        assert!(final_path.exists(), "final jar must exist");
        let mut tmp = final_path.clone().into_os_string();
        tmp.push(".tmp");
        assert!(
            !std::path::PathBuf::from(tmp).exists(),
            ".tmp must be renamed away"
        );

        // Ledger has the row.
        let l = crate::mods::ledger::read_ledger(&paths, "inst1").await.unwrap();
        assert_eq!(l.mods.len(), 1);
        assert_eq!(l.mods[0].mod_id, "AANobbMI");
        assert!(l.mods[0].enabled);
    }

    #[tokio::test]
    async fn test_install_mods_into_instance_propagates_sha_mismatch() {
        let body = b"jar bytes here";
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/x.jar");
            then.status(200).body(body);
        });
        let td = TempDir::new().unwrap();
        let paths = AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        );
        tokio::fs::create_dir_all(paths.instance_minecraft_dir("inst").join("mods"))
            .await
            .unwrap();

        let file = ModrinthFile {
            url: format!("{}/x.jar", server.base_url()),
            filename: "x.jar".into(),
            primary: true,
            size: body.len() as u64,
            hashes: ModrinthHashes {
                sha1: "".into(),
                // Wrong hash on purpose.
                sha512: "deadbeef".into(),
            },
        };
        let row = InstalledModRow {
            mod_id: "P1".into(),
            project_slug: "p".into(),
            display_name: "P".into(),
            version_id: "v".into(),
            version_label: "0.0".into(),
            file_name: "x.jar".into(),
            sha512: "deadbeef".into(),
            size: body.len() as u64,
            hash_algo: HashAlgo::Sha512,
            source: ModSource::Modrinth,
            enabled: true,
            installed_at: "2026-01-01T00:00:00Z".into(),
        };
        let plan = vec![InstallStep { row, file }];
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let (tx, rx) = mpsc::channel(8);
        drain(rx);
        let token = CancellationToken::new();
        let r = install_mods_into_instance(
            http,
            paths.clone(),
            "inst".into(),
            plan,
            tx,
            token,
            JobId(1),
        )
        .await;
        assert!(matches!(r, Err(ModrinthError::Sha512Mismatch { .. })), "got {r:?}");

        // Final jar must NOT exist; ledger must be empty.
        let final_path = paths.instance_mod_file("inst", "x.jar");
        assert!(!final_path.exists(), "no final jar on hash mismatch");
        let l = crate::mods::ledger::read_ledger(&paths, "inst").await.unwrap();
        assert!(l.mods.is_empty(), "no ledger row on hash mismatch");
    }

    // --- download_one_with_hash_algo (Phase 9 generic helper) ---------------

    #[tokio::test]
    async fn test_sha1_verify_passes_with_correct_hash() {
        use httpmock::prelude::*;
        use sha1::{Digest, Sha1};
        let server = MockServer::start();
        let body = b"hello sha1 world".to_vec();
        let mut h = Sha1::new();
        h.update(&body);
        let expected = sha1_hex(h.finalize().as_slice());
        server.mock(|when, then| {
            when.method(GET).path("/x.jar");
            then.status(200).body(body.clone());
        });

        let td = TempDir::new().unwrap();
        let dest = td.path().join("x.jar.tmp");
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let (tx, rx) = mpsc::channel(64);
        drain(rx);
        let token = CancellationToken::new();

        let n = download_one_with_hash_algo(
            &http,
            &format!("{}/x.jar", server.base_url()),
            &expected,
            HashAlgo::Sha1,
            &dest,
            "x.jar",
            body.len() as u64,
            &tx,
            JobId(0),
            &token,
            0,
            1,
        )
        .await
        .unwrap();
        assert_eq!(n, body.len() as u64);
        assert!(dest.exists());
    }

    #[tokio::test]
    async fn test_sha1_verify_fails_with_wrong_hash() {
        use httpmock::prelude::*;
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/x.jar");
            then.status(200).body(b"actual body");
        });
        let td = TempDir::new().unwrap();
        let dest = td.path().join("x.jar.tmp");
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let (tx, rx) = mpsc::channel(64);
        drain(rx);
        let token = CancellationToken::new();

        let r = download_one_with_hash_algo(
            &http,
            &format!("{}/x.jar", server.base_url()),
            "deadbeef",
            HashAlgo::Sha1,
            &dest,
            "x.jar",
            11,
            &tx,
            JobId(0),
            &token,
            0,
            1,
        )
        .await;
        assert!(
            matches!(r, Err(ModrinthError::Sha512Mismatch { .. })),
            "expected Sha512Mismatch (used as generic mismatch variant), got {r:?}"
        );
    }

    #[tokio::test]
    async fn test_sha1_verify_case_insensitive_uppercase_expected() {
        // Pitfall 3: CurseForge sometimes returns hashes in UPPERCASE.
        use httpmock::prelude::*;
        use sha1::{Digest, Sha1};
        let server = MockServer::start();
        let body = b"case-insensitive test".to_vec();
        let mut h = Sha1::new();
        h.update(&body);
        let expected_lower = sha1_hex(h.finalize().as_slice());
        let expected_upper = expected_lower.to_uppercase();
        server.mock(|when, then| {
            when.method(GET).path("/x.jar");
            then.status(200).body(body.clone());
        });
        let td = TempDir::new().unwrap();
        let dest = td.path().join("x.jar.tmp");
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let (tx, rx) = mpsc::channel(64);
        drain(rx);
        let token = CancellationToken::new();

        let n = download_one_with_hash_algo(
            &http,
            &format!("{}/x.jar", server.base_url()),
            &expected_upper,
            HashAlgo::Sha1,
            &dest,
            "x.jar",
            body.len() as u64,
            &tx,
            JobId(0),
            &token,
            0,
            1,
        )
        .await
        .unwrap();
        assert_eq!(n, body.len() as u64);
    }

    #[tokio::test]
    async fn test_sha256_verify_passes() {
        use httpmock::prelude::*;
        use sha2::{Digest, Sha256};
        let server = MockServer::start();
        let body = b"sha256 round trip".to_vec();
        let mut h = Sha256::new();
        h.update(&body);
        let expected = sha256_hex(h.finalize().as_slice());
        server.mock(|when, then| {
            when.method(GET).path("/x.jar");
            then.status(200).body(body.clone());
        });
        let td = TempDir::new().unwrap();
        let dest = td.path().join("x.jar.tmp");
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let (tx, rx) = mpsc::channel(64);
        drain(rx);
        let token = CancellationToken::new();

        let n = download_one_with_hash_algo(
            &http,
            &format!("{}/x.jar", server.base_url()),
            &expected,
            HashAlgo::Sha256,
            &dest,
            "x.jar",
            body.len() as u64,
            &tx,
            JobId(0),
            &token,
            0,
            1,
        )
        .await
        .unwrap();
        assert_eq!(n, body.len() as u64);
    }

    #[tokio::test]
    async fn test_install_mods_into_instance_pre_cancelled_returns_cancelled() {
        let td = TempDir::new().unwrap();
        let paths = AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        );
        let plan = vec![InstallStep {
            row: InstalledModRow {
                mod_id: "p".into(),
                project_slug: "p".into(),
                display_name: "p".into(),
                version_id: "v".into(),
                version_label: "0".into(),
                file_name: "p.jar".into(),
                sha512: "0".into(),
                size: 1,
                hash_algo: HashAlgo::Sha512,
                source: ModSource::Modrinth,
                enabled: true,
                installed_at: "now".into(),
            },
            file: ModrinthFile {
                url: "https://example.invalid/p.jar".into(),
                filename: "p.jar".into(),
                primary: true,
                size: 1,
                hashes: ModrinthHashes {
                    sha1: "".into(),
                    sha512: "0".into(),
                },
            },
        }];
        let http = reqwest::Client::builder().user_agent("test").build().unwrap();
        let (tx, rx) = mpsc::channel(8);
        drain(rx);
        let token = CancellationToken::new();
        token.cancel();
        let r = install_mods_into_instance(
            http,
            paths,
            "inst".into(),
            plan,
            tx,
            token,
            JobId(1),
        )
        .await;
        assert!(matches!(r, Err(ModrinthError::Cancelled)), "got {r:?}");
    }
}
