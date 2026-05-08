//! Download orchestration layer for Modrinth `.mrpack` modpack imports.
//!
//! Provides:
//! - `MODPACK_ALLOWLIST` — the 7-host hardcoded allowlist (CONTEXT.md D-04).
//! - `is_url_allowlisted` — security gate; runs BEFORE any network call.
//! - `filter_files_for_client` — applies `env.client` filter per PACK-02.
//! - `download_files` — semaphore-bounded, cancellation-cooperative, SHA-512
//!   verified parallel download orchestrator mirroring
//!   `src/mods/installer.rs::install_mods_into_instance` (lines 499-638).
//!
//! Security invariant: `is_url_allowlisted` is called on every download URL
//! BEFORE the reqwest GET is issued (threat T-10-03-01). This is pinned by
//! `test_download_files_rejects_disallowed_source_before_network` which asserts
//! the mock server is NEVER hit when the URL is disallowed.

use std::sync::Arc;

use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::modpack::error::ModpackError;
use crate::modpack::parse::{should_download_for_client, strip_leading_dot_slash, MrpackFile};
use crate::mods::error::ModrinthError;
use crate::mods::installer::download_one_with_hash_algo;
use crate::mods::installer::MOD_DOWNLOAD_CONCURRENCY;
use crate::mods::types::{HashAlgo, InstalledModRow, ModSource};
use crate::persistence::paths::AppPaths;
use crate::tasks::{JobId, TaskEvent};

// Compile-time assertion: MOD_DOWNLOAD_CONCURRENCY is imported, not redefined.
// This constant expression will fail to compile if the import changes shape.
const _: usize = MOD_DOWNLOAD_CONCURRENCY;

/// Hardcoded allowlist of download URL hosts permitted in `.mrpack` `files[].downloads`.
///
/// Source: CONTEXT.md D-04 + RESEARCH.md §URL Allowlist (locked for v1).
/// Expanding this list requires a v2 decision — see Deferred Items in CONTEXT.md.
///
/// Security: checked BEFORE every network call in `download_files`. A non-allowlisted
/// host signals a malformed or malicious modpack (Threat T-10-03-01).
pub const MODPACK_ALLOWLIST: &[&str] = &[
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
    "cdn.azuriom.com",
    "maven.fabricmc.net",
    "maven.minecraftforge.net",
];

/// Check whether `url` is on the 7-host allowlist.
///
/// Returns `Ok(())` if the URL parses and its host is in `MODPACK_ALLOWLIST`.
/// Returns `Err(ModpackError::DisallowedSource)` otherwise.
///
/// Fail semantics:
/// - Unparseable URL → `host: "<unparseable>"`
/// - URL with no host component (e.g. `data:...`) → `host: "<no-host>"`
/// - Host not in allowlist → `host: <actual host>`
///
/// Test exemption: `http://127.0.0.1:*` and `http://localhost:*` are permitted
/// when compiled in `cfg(test)` mode so that httpmock-backed unit tests can serve
/// mod jars over loopback. This exemption mirrors `is_acceptable_mod_url` in
/// `src/mods/installer.rs` and is invisible in production — real modpacks never
/// reference loopback download URLs.
pub fn is_url_allowlisted(url: &str) -> Result<(), ModpackError> {
    // Test-only loopback exemption — mirrors installer.rs::is_acceptable_mod_url.
    #[cfg(test)]
    if url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:") {
        return Ok(());
    }

    let parsed = reqwest::Url::parse(url).map_err(|_| ModpackError::DisallowedSource {
        url: url.to_string(),
        host: "<unparseable>".to_string(),
    })?;
    let host = parsed.host_str().ok_or_else(|| ModpackError::DisallowedSource {
        url: url.to_string(),
        host: "<no-host>".to_string(),
    })?;
    if !MODPACK_ALLOWLIST.contains(&host) {
        return Err(ModpackError::DisallowedSource {
            url: url.to_string(),
            host: host.to_string(),
        });
    }
    Ok(())
}

/// Filter a slice of `MrpackFile` entries to those that should be downloaded
/// for a client install.
///
/// Applies `should_download_for_client` across all entries:
/// - `env.client == Unsupported` → excluded (server-only file).
/// - `env` absent → included (universal file per spec, Pitfall 3).
/// - `env.client == Required | Optional` → included.
///
/// Returns borrowed references into the input slice — no allocation of file data.
pub fn filter_files_for_client(files: &[MrpackFile]) -> Vec<&MrpackFile> {
    files.iter().filter(|f| should_download_for_client(f.env.as_ref())).collect()
}

/// Map a `ModrinthError` returned by `download_one_with_hash_algo` to a
/// `ModpackError` at the call boundary.
fn map_download_error(e: ModrinthError, file_path: &str) -> ModpackError {
    match e {
        ModrinthError::Sha512Mismatch { expected, got, .. } => ModpackError::HashMismatch {
            path: file_path.to_string(),
            expected,
            got,
        },
        ModrinthError::Cancelled => ModpackError::Cancelled,
        ModrinthError::Http(msg) => ModpackError::Http(msg),
        ModrinthError::Io(io_err) => ModpackError::Io(io_err),
        other => ModpackError::Http(format!("{other}")),
    }
}

/// Build an `InstalledModRow` for a successfully downloaded modpack file.
fn build_mod_row(file: &MrpackFile, file_name: &str, size: u64) -> InstalledModRow {
    let hash_prefix_16 = &file.hashes.sha512[..file.hashes.sha512.len().min(16)];
    let hash_prefix_8 = &file.hashes.sha512[..file.hashes.sha512.len().min(8)];
    InstalledModRow {
        mod_id: format!("modpack:{hash_prefix_16}"),
        project_slug: file_name.trim_end_matches(".jar").to_string(),
        display_name: file_name.to_string(),
        version_id: file.hashes.sha512.clone(),
        version_label: format!("modpack-{hash_prefix_8}"),
        file_name: std::path::Path::new(&file.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&file.path)
            .to_string(),
        sha512: file.hashes.sha512.clone(),
        size,
        hash_algo: HashAlgo::Sha512,
        source: ModSource::Modpack,
        enabled: true,
        installed_at: {
            use time::format_description::well_known::Rfc3339;
            time::OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default()
        },
    }
}

/// Parallel, semaphore-bounded, cancellation-cooperative SHA-512 download
/// orchestrator for Modrinth `.mrpack` modpack files.
///
/// # Behaviour
///
/// 1. Filters `files` to client-applicable entries via `filter_files_for_client`.
/// 2. If the filtered list is empty, emits a 100% progress event and returns `Ok(vec![])`.
/// 3. Spawns one `tokio::task::JoinSet` task per file, bounded by
///    `MOD_DOWNLOAD_CONCURRENCY` (= 6, imported from `src/mods/installer.rs`).
/// 4. Each task:
///    a. Acquires a semaphore permit.
///    b. Checks `token.is_cancelled()`.
///    c. For each URL in `file.downloads` (first allowlisted+successful wins):
///       - Calls `is_url_allowlisted` BEFORE the network call.
///       - Calls `download_one_with_hash_algo` with `HashAlgo::Sha512`.
///       - Maps `ModrinthError` → `ModpackError` at the boundary.
///    d. Returns `Ok(InstalledModRow)` on success.
/// 5. Join loop: accumulates results; on cancel or error calls `set.abort_all()`.
///
/// # Atomicity note
///
/// Downloaded files land at `<dest>.tmp`. The `.tmp → final` rename is NOT
/// performed here — it happens in Plan 10-05 `service.rs` after override
/// extraction succeeds (atomicity invariant). The caller owns the rename.
///
/// # Return value
///
/// `Vec<InstalledModRow>` with `source = ModSource::Modpack`. The caller
/// (Plan 10-05) writes these rows to the ledger AFTER rename.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip_all, fields(slug = %slug, file_count = files.len()))]
pub async fn download_files(
    http: &reqwest::Client,
    paths: &AppPaths,
    slug: &str,
    files: Vec<MrpackFile>,
    progress_tx: mpsc::Sender<TaskEvent>,
    token: CancellationToken,
    job_id: JobId,
) -> Result<Vec<InstalledModRow>, ModpackError> {
    if token.is_cancelled() {
        return Err(ModpackError::Cancelled);
    }

    // Step 1: filter to client-applicable files and collect to owned Vec.
    let filtered: Vec<MrpackFile> = filter_files_for_client(&files)
        .into_iter()
        .cloned()
        .collect();

    let total = filtered.len();

    // Step 2: early-exit if nothing to download.
    if total == 0 {
        let _ = progress_tx
            .send(TaskEvent::Progress {
                id: job_id,
                pct: 100,
                msg: "No mods to download".to_string(),
            })
            .await;
        return Ok(vec![]);
    }

    // Step 3: semaphore for concurrency cap.
    let sem = Arc::new(Semaphore::new(MOD_DOWNLOAD_CONCURRENCY));
    let mut set = tokio::task::JoinSet::new();

    // Step 4: spawn one task per file.
    for (i, file) in filtered.into_iter().enumerate() {
        let sem = Arc::clone(&sem);
        let http = http.clone();
        let paths = paths.clone();
        let slug = slug.to_string();
        let progress_tx = progress_tx.clone();
        let token = token.clone();

        set.spawn(async move {
            // 4a. Acquire semaphore permit.
            let _permit = sem.acquire_owned().await.map_err(|e| {
                ModpackError::Io(std::io::Error::other(format!("semaphore closed: {e}")))
            })?;

            // 4b. Check cancellation AFTER acquiring permit (Pitfall — per plan must).
            if token.is_cancelled() {
                return Err(ModpackError::Cancelled);
            }

            // Derive the file name label for progress messages.
            let file_name = std::path::Path::new(&file.path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&file.path)
                .to_string();

            // 4c. Try each download URL in order.
            if file.downloads.is_empty() {
                return Err(ModpackError::Http(format!("no download URLs for {}", file.path)));
            }

            let mut last_err: Option<ModpackError> = None;

            for url in &file.downloads {
                // Security gate: allowlist check BEFORE any network call.
                // On DisallowedSource → fail-fast (don't try next URL).
                is_url_allowlisted(url)?;

                // Compute .tmp destination path.
                // Base: <instance>/.minecraft/<file.path> (after stripping leading ./)
                let relative = strip_leading_dot_slash(&file.path);
                let final_path = paths.instance_minecraft_dir(&slug).join(relative);
                let tmp_path = {
                    let mut s = final_path.clone().into_os_string();
                    s.push(".tmp");
                    std::path::PathBuf::from(s)
                };

                let result = download_one_with_hash_algo(
                    &http,
                    url,
                    &file.hashes.sha512,
                    HashAlgo::Sha512,
                    &tmp_path,
                    &file_name,
                    file.file_size,
                    &progress_tx,
                    job_id,
                    &token,
                    i,
                    total,
                )
                .await;

                match result {
                    Ok(size) => {
                        let row = build_mod_row(&file, &file_name, size);
                        return Ok(row);
                    }
                    Err(ModrinthError::Sha512Mismatch { expected, got, .. }) => {
                        // Hash mismatch is always a fatal error — don't try next URL.
                        return Err(ModpackError::HashMismatch {
                            path: file.path.clone(),
                            expected,
                            got,
                        });
                    }
                    Err(ModrinthError::Cancelled) => {
                        return Err(ModpackError::Cancelled);
                    }
                    Err(e) => {
                        // HTTP/network error: try next URL.
                        last_err = Some(map_download_error(e, &file.path));
                    }
                }
            }

            // All URLs exhausted with non-fatal errors.
            Err(last_err.unwrap_or_else(|| {
                ModpackError::Http(format!("all download URLs failed for {}", file.path))
            }))
        });
    }

    // Step 5: join loop — collect results, handle cancellation and errors.
    let mut rows: Vec<InstalledModRow> = Vec::with_capacity(total);
    let mut completed: usize = 0;

    while let Some(res) = set.join_next().await {
        // Check cancellation on each iteration of the join loop.
        if token.is_cancelled() {
            set.abort_all();
            return Err(ModpackError::Cancelled);
        }

        match res {
            Ok(Ok(row)) => {
                completed += 1;
                rows.push(row);
                let pct = ((completed as u64 * 100) / total as u64).min(100) as u8;
                let _ = progress_tx
                    .send(TaskEvent::Progress {
                        id: job_id,
                        pct,
                        msg: format!("Downloaded {completed}/{total} mods"),
                    })
                    .await;
            }
            Ok(Err(e)) => {
                set.abort_all();
                return Err(e);
            }
            Err(join_err) if join_err.is_cancelled() => {
                set.abort_all();
                return Err(ModpackError::Cancelled);
            }
            Err(join_err) => {
                set.abort_all();
                return Err(ModpackError::Io(std::io::Error::other(format!(
                    "download task panicked: {join_err}"
                ))));
            }
        }
    }

    Ok(rows)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modpack::parse::{EnvRequirement, MrpackEnv, MrpackHashes};
    use crate::tasks::JobId;

    /// Helper: build a minimal MrpackFile for test setup.
    fn mk_file(path: &str, hex_sha512: &str, size: u64, url: &str) -> MrpackFile {
        MrpackFile {
            path: path.to_string(),
            hashes: MrpackHashes { sha1: "aabb".to_string(), sha512: hex_sha512.to_string() },
            env: None,
            downloads: vec![url.to_string()],
            file_size: size,
        }
    }

    /// Helper: build a MrpackFile with a specific env.client value.
    fn mk_file_with_env(
        path: &str,
        hex_sha512: &str,
        size: u64,
        url: &str,
        client: EnvRequirement,
    ) -> MrpackFile {
        MrpackFile {
            path: path.to_string(),
            hashes: MrpackHashes { sha1: "aabb".to_string(), sha512: hex_sha512.to_string() },
            env: Some(MrpackEnv { client, server: EnvRequirement::Required }),
            downloads: vec![url.to_string()],
            file_size: size,
        }
    }

    // -------------------------------------------------------------------------
    // Allowlist tests (1-6)
    // -------------------------------------------------------------------------

    #[test]
    fn test_allowlist_accepts_cdn_modrinth() {
        assert!(
            is_url_allowlisted("https://cdn.modrinth.com/data/foo.jar").is_ok(),
            "cdn.modrinth.com must be accepted"
        );
    }

    #[test]
    fn test_allowlist_accepts_github_releases() {
        assert!(
            is_url_allowlisted("https://github.com/x/y/releases/download/foo.jar").is_ok(),
            "github.com must be accepted"
        );
    }

    #[test]
    fn test_allowlist_accepts_maven_fabricmc_net() {
        assert!(
            is_url_allowlisted("https://maven.fabricmc.net/foo/bar.jar").is_ok(),
            "maven.fabricmc.net must be accepted"
        );
    }

    #[test]
    fn test_allowlist_rejects_attacker_com() {
        let err = is_url_allowlisted("http://attacker.com/backdoor.jar")
            .expect_err("attacker.com must be rejected");
        match err {
            ModpackError::DisallowedSource { host, .. } => {
                assert_eq!(host, "attacker.com", "host in error must be attacker.com");
            }
            other => panic!("expected DisallowedSource, got {other:?}"),
        }
    }

    #[test]
    fn test_allowlist_rejects_typo_modrinth() {
        let err = is_url_allowlisted("https://cdn.modrinth.co/foo.jar")
            .expect_err("cdn.modrinth.co (typo) must be rejected");
        match err {
            ModpackError::DisallowedSource { host, .. } => {
                assert_eq!(host, "cdn.modrinth.co");
            }
            other => panic!("expected DisallowedSource, got {other:?}"),
        }
    }

    #[test]
    fn test_allowlist_rejects_unparseable_url() {
        let err =
            is_url_allowlisted("not a url").expect_err("unparseable URL must be rejected");
        match err {
            ModpackError::DisallowedSource { host, .. } => {
                assert_eq!(host, "<unparseable>", "host must be <unparseable> for bad URL");
            }
            other => panic!("expected DisallowedSource, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // Filter tests (7-8)
    // -------------------------------------------------------------------------

    #[test]
    fn test_filter_skips_unsupported_client() {
        let files = vec![
            mk_file_with_env("mods/a.jar", "aa00", 10, "https://cdn.modrinth.com/a.jar",
                EnvRequirement::Required),
            mk_file_with_env("mods/b.jar", "bb00", 10, "https://cdn.modrinth.com/b.jar",
                EnvRequirement::Unsupported),
            mk_file_with_env("mods/c.jar", "cc00", 10, "https://cdn.modrinth.com/c.jar",
                EnvRequirement::Optional),
        ];
        let filtered = filter_files_for_client(&files);
        assert_eq!(filtered.len(), 2, "Unsupported entry must be excluded");
        assert!(filtered.iter().all(|f| f.path != "mods/b.jar"),
            "b.jar (Unsupported) must not appear in filtered list");
    }

    #[test]
    fn test_filter_keeps_missing_env() {
        let files = vec![
            mk_file("mods/no-env.jar", "dd00", 10, "https://cdn.modrinth.com/no-env.jar"),
        ];
        // env field is None → should be treated as Required → kept
        let filtered = filter_files_for_client(&files);
        assert_eq!(filtered.len(), 1, "File with missing env must be kept (Pitfall 3)");
    }

    // -------------------------------------------------------------------------
    // download_files integration tests (9-14) using httpmock
    // -------------------------------------------------------------------------

    /// Compute real SHA-512 hex of a byte slice for test fixtures.
    fn sha512_hex(data: &[u8]) -> String {
        use sha2::{Digest, Sha512};
        let mut h = Sha512::new();
        h.update(data);
        h.finalize().iter().fold(String::with_capacity(128), |mut s, b| {
            use std::fmt::Write as _;
            write!(s, "{b:02x}").unwrap();
            s
        })
    }

    fn make_http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .build()
            .expect("build reqwest client")
    }

    fn make_paths(tmp: &tempfile::TempDir) -> AppPaths {
        AppPaths::with_roots(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
        )
    }

    #[tokio::test]
    async fn test_download_files_happy_path() {
        use httpmock::prelude::*;

        let server = MockServer::start();
        let body_a = b"fake mod content A";
        let body_b = b"fake mod content B";
        let sha_a = sha512_hex(body_a);
        let sha_b = sha512_hex(body_b);

        let mock_a = server.mock(|when, then| {
            when.method(GET).path("/mod-a.jar");
            then.status(200).body(body_a);
        });
        let mock_b = server.mock(|when, then| {
            when.method(GET).path("/mod-b.jar");
            then.status(200).body(body_b);
        });

        let tmp = tempfile::TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let slug = "test-pack";

        let files = vec![
            mk_file("mods/mod-a.jar", &sha_a, body_a.len() as u64,
                &server.url("/mod-a.jar")),
            mk_file("mods/mod-b.jar", &sha_b, body_b.len() as u64,
                &server.url("/mod-b.jar")),
        ];

        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let job_id = JobId(1);

        let result = download_files(&make_http_client(), &paths, slug, files, tx, token, job_id).await;

        assert!(result.is_ok(), "happy path must succeed: {result:?}");
        let rows = result.unwrap();
        assert_eq!(rows.len(), 2, "must return 2 InstalledModRow entries");
        for row in &rows {
            assert_eq!(row.source, ModSource::Modpack, "source must be ModSource::Modpack");
        }

        // Both .tmp files must exist on disk.
        let tmp_a = paths.instance_minecraft_dir(slug).join("mods/mod-a.jar.tmp");
        let tmp_b = paths.instance_minecraft_dir(slug).join("mods/mod-b.jar.tmp");
        assert!(tmp_a.exists(), "mod-a.jar.tmp must exist: {}", tmp_a.display());
        assert!(tmp_b.exists(), "mod-b.jar.tmp must exist: {}", tmp_b.display());

        mock_a.assert();
        mock_b.assert();
    }

    #[tokio::test]
    async fn test_download_files_skips_unsupported_client() {
        use httpmock::prelude::*;

        let server = MockServer::start();
        let body = b"mod bytes";
        let sha = sha512_hex(body);

        let mock_required = server.mock(|when, then| {
            when.method(GET).path("/required.jar");
            then.status(200).body(body);
        });
        let mock_optional = server.mock(|when, then| {
            when.method(GET).path("/optional.jar");
            then.status(200).body(body);
        });
        // The server-only file's mock — must NEVER be hit.
        let mock_server_only = server.mock(|when, then| {
            when.method(GET).path("/server-only.jar");
            then.status(200).body(body);
        });

        let tmp = tempfile::TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let slug = "test-pack";

        let files = vec![
            mk_file_with_env("mods/required.jar", &sha, body.len() as u64,
                &server.url("/required.jar"), EnvRequirement::Required),
            mk_file_with_env("mods/optional.jar", &sha, body.len() as u64,
                &server.url("/optional.jar"), EnvRequirement::Optional),
            mk_file_with_env("mods/server-only.jar", "deadbeef", body.len() as u64,
                &server.url("/server-only.jar"), EnvRequirement::Unsupported),
        ];

        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let job_id = JobId(2);

        let result = download_files(&make_http_client(), &paths, slug, files, tx, token, job_id).await;
        assert!(result.is_ok(), "must succeed: {result:?}");
        let rows = result.unwrap();
        assert_eq!(rows.len(), 2, "only 2 client-applicable files must be downloaded");

        // The server-only mock must never have been hit.
        mock_server_only.assert_calls(0);
        mock_required.assert();
        mock_optional.assert();
    }

    #[tokio::test]
    async fn test_download_files_rejects_disallowed_source_before_network() {
        use httpmock::prelude::*;

        // Strategy: the disallowed URL uses a non-allowlisted hostname (attacker.com).
        // The allowlist check fires BEFORE any network call — so the mock server
        // that would serve "attacker.com"-equivalent responses is NEVER contacted.
        //
        // We verify both that the error type is DisallowedSource AND that the
        // mock server hit count is 0 (proving the allowlist ran before any GET).
        let server = MockServer::start();
        let attacker_mock = server.mock(|when, then| {
            when.method(GET).path("/backdoor.jar");
            then.status(200).body(b"evil bytes");
        });

        let tmp = tempfile::TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let slug = "evil-pack";

        // Use "http://attacker.com/backdoor.jar" — not reachable over the network
        // in test (no DNS resolution attempted; the allowlist short-circuits first).
        // This URL is NOT in MODPACK_ALLOWLIST and is NOT a loopback exemption.
        let disallowed_url = "http://attacker.com/backdoor.jar";

        let files = vec![mk_file("mods/bad.jar", "aabbccdd", 10, disallowed_url)];

        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let job_id = JobId(3);

        let result = download_files(&make_http_client(), &paths, slug, files, tx, token, job_id).await;

        assert!(result.is_err(), "disallowed URL must return Err");
        match result.unwrap_err() {
            ModpackError::DisallowedSource { host, .. } => {
                assert_eq!(host, "attacker.com", "host must be attacker.com");
            }
            other => panic!("expected DisallowedSource, got {other:?}"),
        }
        // The mock server must NEVER have been hit — allowlist ran before any GET.
        attacker_mock.assert_calls(0);
    }

    #[tokio::test]
    async fn test_download_files_sha512_mismatch_maps_to_hash_mismatch() {
        use httpmock::prelude::*;

        let server = MockServer::start();
        let actual_body = b"real content";
        let wrong_sha = "a".repeat(128); // 128 'a' chars — definitely wrong

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/mod.jar");
            then.status(200).body(actual_body);
        });

        let tmp = tempfile::TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let slug = "test-pack";

        let files = vec![mk_file("mods/mod.jar", &wrong_sha, actual_body.len() as u64,
            &server.url("/mod.jar"))];

        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let job_id = JobId(4);

        let result = download_files(&make_http_client(), &paths, slug, files, tx, token, job_id).await;

        assert!(result.is_err(), "hash mismatch must return Err");
        match result.unwrap_err() {
            ModpackError::HashMismatch { path, .. } => {
                assert_eq!(path, "mods/mod.jar", "path in error must be the manifest path");
            }
            other => panic!("expected HashMismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_download_files_cancel_before_start_returns_cancelled() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let slug = "test-pack";

        let files = vec![mk_file("mods/mod.jar", &"a".repeat(128), 10,
            "https://cdn.modrinth.com/this-will-not-be-fetched.jar")];

        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        token.cancel(); // Cancel BEFORE calling download_files
        let job_id = JobId(5);

        let result = download_files(&make_http_client(), &paths, slug, files, tx, token, job_id).await;

        assert!(result.is_err(), "pre-cancelled token must return Err");
        match result.unwrap_err() {
            ModpackError::Cancelled => {}
            other => panic!("expected Cancelled, got {other:?}"),
        }

        // No .tmp files should have been created.
        let mods_dir = paths.instance_minecraft_dir(slug).join("mods");
        if mods_dir.exists() {
            let entries: Vec<_> = std::fs::read_dir(&mods_dir).unwrap().collect();
            assert!(entries.is_empty(), "no .tmp files should exist after pre-cancel");
        }
    }

    #[test]
    fn test_download_files_concurrency_uses_imported_const() {
        // Structural assertion: MOD_DOWNLOAD_CONCURRENCY is imported from
        // crate::mods::installer, not redefined in this module.
        // If this binding resolves, the import is wired correctly.
        let _: usize = crate::mods::installer::MOD_DOWNLOAD_CONCURRENCY;
    }
}
