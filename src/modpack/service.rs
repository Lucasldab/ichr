//! `ModpackService` -- orchestration façade for Modrinth `.mrpack` import.
//!
//! Composes the four sibling modules (parse, download, overrides, plus the
//! existing LoaderService and JavaService from prior phases) into a single
//! 7-step atomic import sequence consumed by Plan 10-06's TUI `Effect` arm.
//!
//! # Atomicity invariant
//!
//! `instance.json` is written LAST (Step 8).  Any failure or cancellation in
//! Steps 3-7 triggers `tokio::fs::remove_dir_all(&instance_dir)` before the
//! error is returned -- leaving the filesystem as if the import never started.
//! This is the Phase 8/9 atomicity invariant applied to modpack imports.
//!
//! # 7-step sequence
//!
//! ```text
//! Step 1  Read + parse modrinth.index.json from the .mrpack zip (in-memory; no disk write)
//! Step 2  slugify + unique_slug + create_instance (instance dir on disk after this)
//! Step 3  download mods via download_files (SHA-512 verified; .tmp files on disk)
//! Step 4  upsert ledger rows BEFORE rename (Pitfall 8 invariant)
//! Step 5  rename each .tmp → final.jar
//! Step 6  apply_overrides: overrides/ then client-overrides/
//! Step 7  install modloader via LoaderService::install_loader (if declared)
//! Step 8  re-read + write_instance_manifest LAST (explicit atomicity gate)
//! ```

use std::path::Path;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::domain::InstanceManifest;
use crate::install::install_version;
use crate::instance::store::{read_instance_manifest, write_instance_manifest};
use crate::java::service::JavaService;
use crate::loader::error::LoaderError;
use crate::loader::service::LoaderService;
use crate::modpack::download::download_files;
use crate::modpack::error::ModpackError;
use crate::modpack::overrides::apply_overrides;
use crate::modpack::parse::{detect_loader, parse_index, MrpackIndex};
use crate::mods::error::ModrinthError;
use crate::mods::ledger::upsert_mod;
use crate::mojang::client::MojangClient;
use crate::persistence::paths::AppPaths;
use crate::services::instance_service::create_instance;
use crate::tasks::{JobId, TaskEvent};

// ============================================================================
// Service struct
// ============================================================================

/// Façade for `.mrpack` modpack imports.
///
/// Holds a single `reqwest::Client` with a 30-second timeout and the project
/// user-agent -- mirrors `ModrinthService`/`CurseForgeService` field-by-field
/// (PATTERNS.md §1).
#[derive(Debug)]
pub struct ModpackService {
    http: reqwest::Client,
}

impl ModpackService {
    /// Build a production `ModpackService` with a 30-second HTTP timeout.
    #[tracing::instrument(skip_all)]
    pub fn new() -> Result<Self, ModpackError> {
        let http = reqwest::Client::builder()
            .user_agent(crate::mojang::client::USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ModpackError::Io(std::io::Error::other(e.to_string())))?;
        Ok(Self { http })
    }

    /// Test constructor -- accepts a pre-built client.
    ///
    /// Used by unit tests with `httpmock` to inject a loopback-exempt client
    /// without relying on environment variables.
    pub fn with_client(http: reqwest::Client) -> Self {
        Self { http }
    }

    // ========================================================================
    // Public import entry point
    // ========================================================================

    /// Import a `.mrpack` archive as a new isolated instance.
    ///
    /// Executes the 7-step atomic sequence:
    ///
    /// 1. Read + parse `modrinth.index.json` (in-memory; no instance dir yet).
    /// 2. Create instance via `create_instance` (instance dir on disk after this).
    /// 3-7. Download mods, upsert ledger, rename, apply overrides, install loader.
    /// 8.  Write instance manifest LAST (atomicity gate).
    ///
    /// On any failure or cancellation in Steps 3-7, the partially-created
    /// instance directory is removed via `tokio::fs::remove_dir_all` before
    /// the error is propagated -- leaving no half-installed state on disk.
    ///
    /// A pre-cancel (before Step 2) returns `Err(Cancelled)` without creating
    /// any instance directory.
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip_all, fields(path = %mrpack_path.display()))]
    pub async fn import_mrpack(
        &self,
        paths: &AppPaths,
        mrpack_path: &Path,
        mojang_client: &MojangClient,
        loader_service: &LoaderService,
        java_service: &JavaService,
        progress_tx: mpsc::Sender<TaskEvent>,
        token: CancellationToken,
        job_id: JobId,
    ) -> Result<InstanceManifest, ModpackError> {
        // ── STEP 1: parse manifest (no disk write yet) ────────────────────────
        if token.is_cancelled() {
            return Err(ModpackError::Cancelled);
        }
        let _ = progress_tx
            .send(TaskEvent::Progress {
                id: job_id,
                pct: 5,
                msg: "Parsing manifest".to_string(),
            })
            .await;

        let (_index_bytes, index) = read_and_parse_mrpack_index(mrpack_path).await?;

        // ── STEP 2: create instance (instance dir on disk after this) ─────────
        if token.is_cancelled() {
            return Err(ModpackError::Cancelled);
        }
        let _ = progress_tx
            .send(TaskEvent::Progress {
                id: job_id,
                pct: 10,
                msg: "Creating instance".to_string(),
            })
            .await;

        let mc_version = index
            .dependencies
            .get("minecraft")
            .ok_or(ModpackError::MissingMinecraftDependency)?
            .clone();

        let manifest_initial = create_instance(paths, &index.name, &mc_version)
            .await
            .map_err(|e| ModpackError::Http(format!("create_instance: {e}")))?;
        let slug = manifest_initial.slug.clone();

        // ── STEPS 2.5-7: cleanup-wrapped inner block ──────────────────────────
        let inner_result = self
            .import_inner(
                paths,
                &slug,
                &mc_version,
                &index,
                mrpack_path,
                mojang_client,
                loader_service,
                java_service,
                &progress_tx,
                &token,
                job_id,
            )
            .await;

        match inner_result {
            Ok(()) => {
                // ── STEP 8: re-read + write manifest LAST (atomicity gate) ────
                let final_manifest = read_instance_manifest(paths, &slug)
                    .await
                    .map_err(|e| ModpackError::Http(format!("read final manifest: {e}")))?;
                write_instance_manifest(paths, &final_manifest)
                    .await
                    .map_err(|e| ModpackError::Http(format!("final manifest write: {e}")))?;
                let _ = progress_tx
                    .send(TaskEvent::Progress {
                        id: job_id,
                        pct: 100,
                        msg: "Done".to_string(),
                    })
                    .await;
                Ok(final_manifest)
            }
            Err(e) => {
                // ATOMICITY INVARIANT: any failure in Steps 3-7 (including
                // Cancelled) removes the entire instance directory.  The
                // absent instance.json signals "instance does not exist".
                let _ = tokio::fs::remove_dir_all(&paths.instance_dir(&slug)).await;
                Err(e)
            }
        }
    }

    // ========================================================================
    // Inner import: Steps 3-7
    // ========================================================================

    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip_all, fields(slug = %slug, mc_version = %mc_version))]
    async fn import_inner(
        &self,
        paths: &AppPaths,
        slug: &str,
        mc_version: &str,
        index: &MrpackIndex,
        mrpack_path: &Path,
        mojang_client: &MojangClient,
        loader_service: &LoaderService,
        java_service: &JavaService,
        progress_tx: &mpsc::Sender<TaskEvent>,
        token: &CancellationToken,
        job_id: JobId,
    ) -> Result<(), ModpackError> {
        // ── STEP 2.5: install vanilla MC (GAP-10-A fix) ───────────────────────
        // LoaderService::install_loader → JavaService::resolve_jre_for_mc_version_install
        // requires the vanilla version JSON on disk. For a fresh modpack import,
        // vanilla MC has never been installed for this version, so we install it here
        // before the loader can be wired up. Idempotent: existing installs are skipped.
        if !paths.version_json(mc_version).exists() {
            if token.is_cancelled() {
                return Err(ModpackError::Cancelled);
            }
            let _ = progress_tx
                .send(TaskEvent::Progress {
                    id: job_id,
                    pct: 12,
                    msg: format!("Installing vanilla MC {mc_version}"),
                })
                .await;

            let cache_path = paths.cache_dir.join("manifest_v2.json");
            let manifest = mojang_client
                .fetch_manifest(&cache_path)
                .await
                .map_err(|e| ModpackError::Http(format!("fetch Mojang manifest: {e}")))?;
            let entry = manifest
                .versions
                .iter()
                .find(|v| v.id == mc_version)
                .ok_or_else(|| {
                    ModpackError::Http(format!(
                        "vanilla MC {mc_version} not found in Mojang manifest \
                         (versions cover release+snapshot; pre-release/alpha excluded)"
                    ))
                })?
                .clone();
            install_version(
                job_id,
                paths,
                mojang_client,
                progress_tx.clone(),
                token.clone(),
                slug,
                &entry,
            )
            .await
            .map_err(|e| match e {
                crate::error::AppError::Cancelled => ModpackError::Cancelled,
                other => ModpackError::Http(format!("install vanilla MC: {other}")),
            })?;
        }

        // ── STEP 3: download mods (.tmp files on disk) ────────────────────────
        let _ = progress_tx
            .send(TaskEvent::Progress {
                id: job_id,
                pct: 15,
                msg: "Downloading mods".to_string(),
            })
            .await;

        // filter_files_for_client is applied inside download_files; pass the
        // full files vec -- download_files handles the env.client filter.
        let rows = download_files(
            &self.http,
            paths,
            slug,
            index.files.clone(),
            progress_tx.clone(),
            token.clone(),
            job_id,
        )
        .await?;

        // ── STEP 4: upsert ledger rows BEFORE rename (Pitfall 8) ─────────────
        for row in &rows {
            upsert_mod(paths, slug, row.clone())
                .await
                .map_err(|e| match e {
                    ModrinthError::Io(io) => ModpackError::Io(io),
                    other => ModpackError::Http(format!("ledger upsert: {other}")),
                })?;
        }

        // ── STEP 5: rename .tmp → final.jar ──────────────────────────────────
        for row in &rows {
            let final_path = paths.instance_mod_file(slug, &row.file_name);
            // download.rs builds the .tmp path as: final_path + ".tmp"
            // (not .with_extension -- the original extension is preserved).
            let tmp_path = {
                let mut s = final_path.clone().into_os_string();
                s.push(".tmp");
                std::path::PathBuf::from(s)
            };
            tokio::fs::rename(&tmp_path, &final_path)
                .await
                .map_err(|e| {
                    ModpackError::Io(std::io::Error::other(format!(
                        "rename {} -> {}: {e}",
                        tmp_path.display(),
                        final_path.display(),
                    )))
                })?;
        }

        let _ = progress_tx
            .send(TaskEvent::Progress {
                id: job_id,
                pct: 70,
                msg: "Mods installed".to_string(),
            })
            .await;

        // ── STEP 6: apply overrides + client-overrides ────────────────────────
        if token.is_cancelled() {
            return Err(ModpackError::Cancelled);
        }
        let _ = progress_tx
            .send(TaskEvent::Progress {
                id: job_id,
                pct: 80,
                msg: "Extracting overrides".to_string(),
            })
            .await;

        apply_overrides(mrpack_path, &paths.instance_minecraft_dir(slug), token).await?;

        // ── STEP 7: install modloader if declared ─────────────────────────────
        if let Some((loader_type, loader_version)) = detect_loader(&index.dependencies) {
            if token.is_cancelled() {
                return Err(ModpackError::Cancelled);
            }
            let _ = progress_tx
                .send(TaskEvent::Progress {
                    id: job_id,
                    pct: 90,
                    msg: format!("Installing modloader ({loader_type:?})"),
                })
                .await;

            let jre_path = java_service
                .resolve_jre_for_mc_version_install(paths, mc_version)
                .await
                .map_err(|e| ModpackError::Http(format!("resolve JRE: {e}")))?;

            loader_service
                .install_loader(
                    paths,
                    slug,
                    mc_version,
                    loader_type,
                    &loader_version,
                    &jre_path,
                    progress_tx.clone(),
                    token.clone(),
                    job_id,
                )
                .await
                .map_err(|e| match e {
                    LoaderError::Cancelled => ModpackError::Cancelled,
                    other => ModpackError::Http(format!("loader install: {other}")),
                })?;
        }

        Ok(())
    }
}

// ============================================================================
// Private helpers
// ============================================================================

/// Open a `.mrpack` zip in a `spawn_blocking` closure, locate
/// `modrinth.index.json`, read its bytes, and parse via `parse_index`.
///
/// Returns `(raw_bytes, MrpackIndex)`.  The raw bytes are returned so the
/// caller retains them for diagnostics if needed.
///
/// Failure before any instance directory is created -- parse errors in Step 1
/// never leave partial state on disk (THREAT T-10-05-03).
async fn read_and_parse_mrpack_index(
    mrpack_path: &Path,
) -> Result<(Vec<u8>, MrpackIndex), ModpackError> {
    let mrpack_path = mrpack_path.to_owned();

    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ModpackError> {
        let file = std::fs::File::open(&mrpack_path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let mut entry = archive
            .by_name("modrinth.index.json")
            .map_err(|_| ModpackError::Zip(zip::result::ZipError::FileNotFound))?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf)?;
        Ok(buf)
    })
    .await
    .map_err(|e| ModpackError::Io(std::io::Error::other(format!("spawn_blocking join: {e}"))))??;

    let json = std::str::from_utf8(&bytes).map_err(|e| {
        ModpackError::ManifestParse(serde_json::Error::io(std::io::Error::other(e)))
    })?;
    let index = parse_index(json)?;

    Ok((bytes, index))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::Path;

    use httpmock::prelude::*;
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use zip::write::SimpleFileOptions;

    use super::ModpackService;
    use crate::modpack::error::ModpackError;
    use crate::mojang::client::MojangClient;
    use crate::persistence::paths::AppPaths;
    use crate::tasks::JobId;

    // ── Fixture builder ───────────────────────────────────────────────────────

    /// Compute SHA-512 hex of a byte slice.
    fn sha512_hex(data: &[u8]) -> String {
        use sha2::{Digest, Sha512};
        let mut h = Sha512::new();
        h.update(data);
        h.finalize()
            .iter()
            .fold(String::with_capacity(128), |mut s, b| {
                use std::fmt::Write as _;
                write!(s, "{b:02x}").unwrap();
                s
            })
    }

    /// Build a minimal `.mrpack` zip at `path`.
    ///
    /// Parameters:
    /// - `mod_url`      -- where the single required mod can be downloaded
    /// - `mod_body`     -- bytes the mock server will return
    /// - `loader_key`   -- e.g. `"fabric-loader"` or `""` for vanilla
    /// - `loader_ver`   -- e.g. `"0.16.9"`
    /// - `override_txt` -- content for `overrides/config/test.txt` (empty → no entry)
    fn build_mrpack(
        path: &Path,
        mod_url: &str,
        mod_body: &[u8],
        loader_key: &str,
        loader_ver: &str,
        override_content: Option<&[u8]>,
        extra_files: &[(&str, &[u8], &str)], // (path, body, url) additional mod files
    ) {
        let sha512 = sha512_hex(mod_body);
        let sha1 = "aabbccdd".to_string();

        let deps_loader = if loader_key.is_empty() {
            String::new()
        } else {
            format!(r#", "{loader_key}": "{loader_ver}""#)
        };

        // Build extra file JSON entries
        let mut file_entries = format!(
            r#"{{
                "path": "mods/required-mod.jar",
                "hashes": {{ "sha1": "{sha1}", "sha512": "{sha512}" }},
                "env": {{ "client": "required", "server": "unsupported" }},
                "downloads": ["{mod_url}"],
                "fileSize": {}
            }}"#,
            mod_body.len()
        );

        for (fpath, fbody, furl) in extra_files {
            let fsha = sha512_hex(fbody);
            file_entries.push_str(&format!(
                r#", {{
                "path": "{fpath}",
                "hashes": {{ "sha1": "ccdd", "sha512": "{fsha}" }},
                "env": {{ "client": "required", "server": "unsupported" }},
                "downloads": ["{furl}"],
                "fileSize": {}
            }}"#,
                fbody.len()
            ));
        }

        let manifest = format!(
            r#"{{
                "formatVersion": 1,
                "game": "minecraft",
                "versionId": "0.1.0",
                "name": "Minimal Test Pack",
                "summary": "Fixture pack",
                "dependencies": {{ "minecraft": "1.20.4"{deps_loader} }},
                "files": [{file_entries}]
            }}"#
        );

        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        writer.start_file("modrinth.index.json", opts).unwrap();
        writer.write_all(manifest.as_bytes()).unwrap();

        if let Some(content) = override_content {
            writer
                .start_file("overrides/config/test.txt", opts)
                .unwrap();
            writer.write_all(content).unwrap();
        }

        writer.finish().unwrap();
    }

    /// Build a `.mrpack` with bad JSON in the manifest.
    fn build_bad_mrpack(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("modrinth.index.json", opts).unwrap();
        writer
            .write_all(br#"{ "formatVersion": 2, "game": "minecraft", "versionId": "1.0", "name": "Bad", "dependencies": { "minecraft": "1.20.4" }, "files": [] }"#)
            .unwrap();
        writer.finish().unwrap();
    }

    fn make_paths(tmp: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
        )
    }

    fn make_http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .build()
            .expect("build reqwest client")
    }

    /// Pre-populate the vanilla MC version JSON so Step 2.5's existence check
    /// skips the live Mojang manifest fetch. Tests that exercise the modpack
    /// import flow without real network must call this in setup, otherwise
    /// Step 2.5 will attempt a live `fetch_manifest` and the test will hang
    /// or fail offline.
    async fn pre_install_vanilla_marker(paths: &AppPaths, mc_version: &str) {
        let json_path = paths.version_json(mc_version);
        if let Some(parent) = json_path.parent() {
            tokio::fs::create_dir_all(parent).await.unwrap();
        }
        tokio::fs::write(&json_path, b"{}").await.unwrap();
    }

    /// Construct a real `MojangClient` for tests. Step 2.5 short-circuits when
    /// the version JSON marker exists, so this client's `fetch_manifest` is
    /// never actually called by the inline tests (the live Mojang URL is not
    /// hit). Tests that omit `pre_install_vanilla_marker` will trigger a real
    /// fetch and may hang offline.
    fn make_mojang_client() -> MojangClient {
        MojangClient::new().expect("build mojang client")
    }

    // ── Test 1: happy path -- vanilla pack (no loader), 1 mod, 1 override ─────

    #[tokio::test]
    async fn test_import_inner_happy_path_vanilla() {
        let server = MockServer::start();
        let mod_body = b"fake mod content for test";
        let sha = {
            use sha2::{Digest, Sha512};
            let mut h = Sha512::new();
            h.update(mod_body);
            h.finalize()
                .iter()
                .fold(String::with_capacity(128), |mut s, b| {
                    use std::fmt::Write as _;
                    write!(s, "{b:02x}").unwrap();
                    s
                })
        };

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/required-mod.jar");
            then.status(200).body(mod_body.as_ref());
        });

        let tmp = TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let mrpack = tmp.path().join("test.mrpack");

        build_mrpack(
            &mrpack,
            &server.url("/required-mod.jar"),
            mod_body,
            "", // vanilla -- no loader
            "",
            Some(b"override content"),
            &[],
        );

        // Drop sha (we only need it for fixture building -- already baked in)
        let _ = sha;

        let svc = ModpackService::with_client(make_http_client());
        let loader_svc = crate::loader::service::LoaderService::new().unwrap();
        let java_svc = crate::java::service::JavaService::new().unwrap();
        let mojang_svc = make_mojang_client();
        pre_install_vanilla_marker(&paths, "1.20.4").await;
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();

        let result = svc
            .import_mrpack(
                &paths,
                &mrpack,
                &mojang_svc,
                &loader_svc,
                &java_svc,
                tx,
                token,
                JobId(1),
            )
            .await;

        assert!(result.is_ok(), "vanilla import must succeed: {result:?}");
        let manifest = result.unwrap();
        assert_eq!(
            manifest.slug, "minimal-test-pack",
            "slug must be derived from pack name"
        );

        // instance.json must exist (atomicity gate was written)
        assert!(
            paths.instance_manifest(&manifest.slug).exists(),
            "instance.json must exist after import"
        );

        // The mod jar must exist (after rename from .tmp)
        let mod_jar = paths.instance_mod_file(&manifest.slug, "required-mod.jar");
        assert!(
            mod_jar.exists(),
            "mod jar must exist: {}",
            mod_jar.display()
        );

        // The override file must exist
        let override_file = paths
            .instance_minecraft_dir(&manifest.slug)
            .join("config/test.txt");
        assert!(
            override_file.exists(),
            "override file must exist: {}",
            override_file.display()
        );
        let contents = std::fs::read_to_string(&override_file).unwrap();
        assert_eq!(contents, "override content");
    }

    // ── Test 2: failure cleans up instance dir (atomicity) ───────────────────

    #[tokio::test]
    async fn test_import_failure_cleans_up_instance_dir() {
        let server = MockServer::start();
        // 500 error on download → the inner steps fail → cleanup must run
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/fail-mod.jar");
            then.status(500).body(b"Internal error");
        });

        let tmp = TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let mrpack = tmp.path().join("test.mrpack");

        build_mrpack(
            &mrpack,
            &server.url("/fail-mod.jar"),
            b"some content",
            "",
            "",
            None,
            &[],
        );

        let svc = ModpackService::with_client(make_http_client());
        let loader_svc = crate::loader::service::LoaderService::new().unwrap();
        let java_svc = crate::java::service::JavaService::new().unwrap();
        let mojang_svc = make_mojang_client();
        pre_install_vanilla_marker(&paths, "1.20.4").await;
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();

        let result = svc
            .import_mrpack(
                &paths,
                &mrpack,
                &mojang_svc,
                &loader_svc,
                &java_svc,
                tx,
                token,
                JobId(2),
            )
            .await;

        assert!(result.is_err(), "failed import must return Err");

        // Atomicity invariant: the instance directory must NOT exist.
        let slug = "minimal-test-pack";
        assert!(
            !paths.instance_dir(slug).exists(),
            "instance dir must be cleaned up after failure: {}",
            paths.instance_dir(slug).display()
        );
        assert!(
            !paths.instance_manifest(slug).exists(),
            "instance.json must not exist after cleanup"
        );
    }

    // ── Test 3: pre-cancel before instance dir -- returns Cancelled, no dir ───

    #[tokio::test]
    async fn test_import_pre_cancel_returns_cancelled_no_instance_dir() {
        let tmp = TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let mrpack = tmp.path().join("test.mrpack");

        // Malformed pack -- doesn't matter; cancel fires first
        build_mrpack(
            &mrpack,
            "https://cdn.modrinth.com/fake.jar",
            b"x",
            "",
            "",
            None,
            &[],
        );

        let svc = ModpackService::with_client(make_http_client());
        let loader_svc = crate::loader::service::LoaderService::new().unwrap();
        let java_svc = crate::java::service::JavaService::new().unwrap();
        let mojang_svc = make_mojang_client();
        pre_install_vanilla_marker(&paths, "1.20.4").await;
        let (tx, _rx) = mpsc::channel(64);

        // Cancel BEFORE calling import_mrpack
        let token = CancellationToken::new();
        token.cancel();

        let result = svc
            .import_mrpack(
                &paths,
                &mrpack,
                &mojang_svc,
                &loader_svc,
                &java_svc,
                tx,
                token,
                JobId(3),
            )
            .await;

        assert!(
            matches!(result, Err(ModpackError::Cancelled)),
            "pre-cancel must return Err(Cancelled): {result:?}"
        );

        // No instance dir should have been created
        let slug = "minimal-test-pack";
        assert!(
            !paths.instance_dir(slug).exists(),
            "pre-cancel must not create instance dir"
        );
    }

    // ── Test 4: mid-download cancel cleans up ─────────────────────────────────

    #[tokio::test]
    async fn test_import_mid_download_cancel_cleans_up() {
        let server = MockServer::start();
        let mod_body = b"x".repeat(1024);

        // Slow mock: respond with delay to give us time to cancel.
        // httpmock doesn't support artificial delays, so we use a 200 response
        // but cancel the token immediately after spawning -- by the time the
        // import_mrpack task calls download_files, the token is already cancelled.
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/slow-mod.jar");
            then.status(200).body(mod_body.clone());
        });

        let tmp = TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let mrpack = tmp.path().join("test.mrpack");

        // Use a real SHA so the mock server would succeed IF not cancelled
        let sha = sha512_hex(&mod_body);
        let _ = sha; // baked into fixture below

        build_mrpack(
            &mrpack,
            &server.url("/slow-mod.jar"),
            &mod_body,
            "",
            "",
            None,
            &[],
        );

        let svc = ModpackService::with_client(make_http_client());
        let loader_svc = crate::loader::service::LoaderService::new().unwrap();
        let java_svc = crate::java::service::JavaService::new().unwrap();
        let mojang_svc = make_mojang_client();
        pre_install_vanilla_marker(&paths, "1.20.4").await;
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        // Spawn import and cancel almost immediately.
        let paths_clone = paths.clone();
        let mrpack_clone = mrpack.clone();
        let handle = tokio::spawn(async move {
            svc.import_mrpack(
                &paths_clone,
                &mrpack_clone,
                &mojang_svc,
                &loader_svc,
                &java_svc,
                tx,
                token_clone,
                JobId(4),
            )
            .await
        });

        // Cancel after a brief yield to let the task start but before it can complete.
        tokio::task::yield_now().await;
        token.cancel();

        let result = handle.await.expect("task must not panic");

        // Either the cancel fired (Err(Cancelled)) or -- if the download completed
        // before the cancel -- it could succeed (Ok). The key invariant is that
        // if it failed, the instance dir is gone.
        match result {
            Err(ModpackError::Cancelled) | Err(_) => {
                let slug = "minimal-test-pack";
                assert!(
                    !paths.instance_dir(slug).exists(),
                    "instance dir must be cleaned up after mid-download cancel"
                );
            }
            Ok(_) => {
                // Download was fast enough to complete before cancel -- acceptable.
            }
        }
    }

    // ── Test 5: malformed manifest rejected before instance dir created ───────

    #[tokio::test]
    async fn test_import_malformed_manifest_rejects_before_instance_dir_created() {
        let tmp = TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let mrpack = tmp.path().join("bad.mrpack");

        // This mrpack has formatVersion: 2 (unsupported)
        build_bad_mrpack(&mrpack);

        let svc = ModpackService::with_client(make_http_client());
        let loader_svc = crate::loader::service::LoaderService::new().unwrap();
        let java_svc = crate::java::service::JavaService::new().unwrap();
        let mojang_svc = make_mojang_client();
        pre_install_vanilla_marker(&paths, "1.20.4").await;
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();

        let result = svc
            .import_mrpack(
                &paths,
                &mrpack,
                &mojang_svc,
                &loader_svc,
                &java_svc,
                tx,
                token,
                JobId(5),
            )
            .await;

        // Must fail with UnsupportedFormat
        match result {
            Err(ModpackError::UnsupportedFormat { version: 2 }) => {}
            other => panic!("expected UnsupportedFormat(2), got {other:?}"),
        }

        // No instance directory should have been created.
        let instances_dir = paths.instances_dir();
        if instances_dir.exists() {
            let count = std::fs::read_dir(&instances_dir).unwrap().count();
            assert_eq!(
                count, 0,
                "instances_dir must be empty -- no instance created: {instances_dir:?}"
            );
        }
    }

    // ── Test 6: disallowed source URL rejects with cleanup ────────────────────

    #[tokio::test]
    async fn test_import_disallowed_source_url_rejects_with_cleanup() {
        let tmp = TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let mrpack = tmp.path().join("evil.mrpack");

        // Build a pack whose mod download URL is non-allowlisted
        let mod_body = b"evil bytes";
        let sha = sha512_hex(mod_body);
        let manifest = format!(
            r#"{{
                "formatVersion": 1,
                "game": "minecraft",
                "versionId": "0.1.0",
                "name": "Evil Pack",
                "dependencies": {{ "minecraft": "1.20.4" }},
                "files": [{{
                    "path": "mods/bad.jar",
                    "hashes": {{ "sha1": "aa", "sha512": "{sha}" }},
                    "env": {{ "client": "required", "server": "unsupported" }},
                    "downloads": ["http://attacker.com/bad.jar"],
                    "fileSize": {}
                }}]
            }}"#,
            mod_body.len()
        );
        {
            let file = std::fs::File::create(&mrpack).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            writer.start_file("modrinth.index.json", opts).unwrap();
            writer.write_all(manifest.as_bytes()).unwrap();
            writer.finish().unwrap();
        }

        let svc = ModpackService::with_client(make_http_client());
        let loader_svc = crate::loader::service::LoaderService::new().unwrap();
        let java_svc = crate::java::service::JavaService::new().unwrap();
        let mojang_svc = make_mojang_client();
        pre_install_vanilla_marker(&paths, "1.20.4").await;
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();

        let result = svc
            .import_mrpack(
                &paths,
                &mrpack,
                &mojang_svc,
                &loader_svc,
                &java_svc,
                tx,
                token,
                JobId(6),
            )
            .await;

        assert!(
            matches!(result, Err(ModpackError::DisallowedSource { .. })),
            "non-allowlisted URL must return DisallowedSource: {result:?}"
        );

        // Instance dir must be cleaned up
        assert!(
            !paths.instance_dir("evil-pack").exists(),
            "instance dir must not remain after DisallowedSource rejection"
        );
    }

    // ── Test 7: idempotent re-run after cancel ────────────────────────────────

    #[tokio::test]
    async fn test_import_idempotent_re_run_after_cancel() {
        let server = MockServer::start();
        let mod_body = b"mod content for idempotency test";

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/idem-mod.jar");
            then.status(200).body(mod_body.as_ref());
        });

        let tmp = TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let mrpack = tmp.path().join("test.mrpack");

        build_mrpack(
            &mrpack,
            &server.url("/idem-mod.jar"),
            mod_body,
            "",
            "",
            None,
            &[],
        );

        // First run -- pre-cancelled (leaves clean filesystem)
        {
            let svc = ModpackService::with_client(make_http_client());
            let loader_svc = crate::loader::service::LoaderService::new().unwrap();
            let java_svc = crate::java::service::JavaService::new().unwrap();
            let mojang_svc = make_mojang_client();
            pre_install_vanilla_marker(&paths, "1.20.4").await;
            let (tx, _rx) = mpsc::channel(64);
            let token = CancellationToken::new();
            token.cancel();
            let r = svc
                .import_mrpack(
                    &paths,
                    &mrpack,
                    &mojang_svc,
                    &loader_svc,
                    &java_svc,
                    tx,
                    token,
                    JobId(7),
                )
                .await;
            assert!(
                matches!(r, Err(ModpackError::Cancelled)),
                "first run must cancel: {r:?}"
            );
        }

        // Second run -- same .mrpack, no pre-cancel
        {
            let svc = ModpackService::with_client(make_http_client());
            let loader_svc = crate::loader::service::LoaderService::new().unwrap();
            let java_svc = crate::java::service::JavaService::new().unwrap();
            let mojang_svc = make_mojang_client();
            pre_install_vanilla_marker(&paths, "1.20.4").await;
            let (tx, _rx) = mpsc::channel(64);
            let token = CancellationToken::new();
            let r = svc
                .import_mrpack(
                    &paths,
                    &mrpack,
                    &mojang_svc,
                    &loader_svc,
                    &java_svc,
                    tx,
                    token,
                    JobId(8),
                )
                .await;
            assert!(r.is_ok(), "second run must succeed: {r:?}");
            let manifest = r.unwrap();
            assert!(
                paths.instance_manifest(&manifest.slug).exists(),
                "instance.json must exist after second run"
            );
        }
    }

    // ── Test 8: with_client test ctor ─────────────────────────────────────────

    #[test]
    fn test_with_client_test_ctor() {
        let svc = ModpackService::with_client(reqwest::Client::new());
        // Compile-time proof that with_client produces a ModpackService.
        assert!(matches!(svc, ModpackService { .. }));
    }

    // ── Test 9: progress events emitted ──────────────────────────────────────

    #[tokio::test]
    async fn test_import_emits_progress_events() {
        let server = MockServer::start();
        let mod_body = b"mod for progress test";

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/prog-mod.jar");
            then.status(200).body(mod_body.as_ref());
        });

        let tmp = TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let mrpack = tmp.path().join("test.mrpack");

        build_mrpack(
            &mrpack,
            &server.url("/prog-mod.jar"),
            mod_body,
            "",
            "",
            None,
            &[],
        );

        let svc = ModpackService::with_client(make_http_client());
        let loader_svc = crate::loader::service::LoaderService::new().unwrap();
        let java_svc = crate::java::service::JavaService::new().unwrap();
        let mojang_svc = make_mojang_client();
        pre_install_vanilla_marker(&paths, "1.20.4").await;

        let (tx, mut rx) = mpsc::channel::<crate::tasks::TaskEvent>(64);
        let token = CancellationToken::new();

        let result = svc
            .import_mrpack(
                &paths,
                &mrpack,
                &mojang_svc,
                &loader_svc,
                &java_svc,
                tx,
                token,
                JobId(9),
            )
            .await;

        assert!(result.is_ok(), "must succeed: {result:?}");

        // Drain all received events
        let mut labels: Vec<String> = Vec::new();
        rx.close();
        while let Ok(evt) = rx.try_recv() {
            if let crate::tasks::TaskEvent::Progress { msg, .. } = evt {
                labels.push(msg);
            }
        }

        // Must include the required step labels in order
        let required = &[
            "Parsing manifest",
            "Creating instance",
            "Mods installed",
            "Done",
        ];
        for req in required {
            assert!(
                labels.iter().any(|l| l.contains(req)),
                "progress must include '{req}'; got: {labels:?}"
            );
        }

        // Ordering: Parsing manifest must come before Done
        let parsing_pos = labels
            .iter()
            .position(|l| l.contains("Parsing manifest"))
            .unwrap();
        let done_pos = labels.iter().position(|l| l.contains("Done")).unwrap();
        assert!(parsing_pos < done_pos, "Parsing manifest must precede Done");
    }

    // ── Test 10: loader install failure cleans up instance dir ───────────────

    /// A modpack with a fabric-loader dep that points to a non-existent version
    /// will fail at LoaderService::install_loader because the Fabric meta
    /// endpoint won't have the version. This is #[ignore]-gated for CI because
    /// it makes a real network call to meta.fabricmc.net.
    #[tokio::test]
    #[ignore = "requires real network: calls meta.fabricmc.net with unrealizable loader version"]
    async fn test_loader_install_failure_cleans_up_instance_dir() {
        let server = MockServer::start();
        let mod_body = b"mod for loader fail test";

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/loader-fail-mod.jar");
            then.status(200).body(mod_body.as_ref());
        });

        let tmp = TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let mrpack = tmp.path().join("test.mrpack");

        // Use an unrealizable fabric-loader version
        build_mrpack(
            &mrpack,
            &server.url("/loader-fail-mod.jar"),
            mod_body,
            "fabric-loader",
            "99.99.99", // non-existent
            None,
            &[],
        );

        let svc = ModpackService::with_client(make_http_client());
        let loader_svc = crate::loader::service::LoaderService::new().unwrap();
        let java_svc = crate::java::service::JavaService::new().unwrap();
        let mojang_svc = make_mojang_client();
        pre_install_vanilla_marker(&paths, "1.20.4").await;
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();

        let result = svc
            .import_mrpack(
                &paths,
                &mrpack,
                &mojang_svc,
                &loader_svc,
                &java_svc,
                tx,
                token,
                JobId(10),
            )
            .await;

        assert!(result.is_err(), "loader install failure must return Err");
        // Atomicity: instance dir must be gone
        assert!(
            !paths.instance_dir("minimal-test-pack").exists(),
            "instance dir must be cleaned up after loader install failure"
        );
    }

    // ── Test 11: unsupported client files are skipped ─────────────────────────

    #[tokio::test]
    async fn test_modpack_with_unsupported_client_files_skipped() {
        let server = MockServer::start();
        let mod_body = b"client required mod bytes";

        // Mock for the Required file (must be hit exactly once)
        let mock_required = server.mock(|when, then| {
            when.method(GET).path("/client-req.jar");
            then.status(200).body(mod_body.as_ref());
        });

        // Mock for the Unsupported file (must NOT be hit)
        let mock_unsupported = server.mock(|when, then| {
            when.method(GET).path("/server-only.jar");
            then.status(200).body(b"server-only bytes");
        });

        let tmp = TempDir::new().unwrap();
        let paths = make_paths(&tmp);
        let mrpack = tmp.path().join("test.mrpack");

        // Build pack with 2 files: one Required, one Unsupported
        let sha_req = sha512_hex(mod_body);
        let manifest = format!(
            r#"{{
                "formatVersion": 1,
                "game": "minecraft",
                "versionId": "0.1.0",
                "name": "Filter Test Pack",
                "dependencies": {{ "minecraft": "1.20.4" }},
                "files": [
                    {{
                        "path": "mods/client-req.jar",
                        "hashes": {{ "sha1": "aa", "sha512": "{sha_req}" }},
                        "env": {{ "client": "required", "server": "unsupported" }},
                        "downloads": ["{}"],
                        "fileSize": {}
                    }},
                    {{
                        "path": "mods/server-only.jar",
                        "hashes": {{ "sha1": "bb", "sha512": "{}"}},
                        "env": {{ "client": "unsupported", "server": "required" }},
                        "downloads": ["{}"],
                        "fileSize": 10
                    }}
                ]
            }}"#,
            server.url("/client-req.jar"),
            mod_body.len(),
            "a".repeat(128),
            server.url("/server-only.jar"),
        );

        {
            let file = std::fs::File::create(&mrpack).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            writer.start_file("modrinth.index.json", opts).unwrap();
            writer.write_all(manifest.as_bytes()).unwrap();
            writer.finish().unwrap();
        }

        let svc = ModpackService::with_client(make_http_client());
        let loader_svc = crate::loader::service::LoaderService::new().unwrap();
        let java_svc = crate::java::service::JavaService::new().unwrap();
        let mojang_svc = make_mojang_client();
        pre_install_vanilla_marker(&paths, "1.20.4").await;
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();

        let result = svc
            .import_mrpack(
                &paths,
                &mrpack,
                &mojang_svc,
                &loader_svc,
                &java_svc,
                tx,
                token,
                JobId(11),
            )
            .await;

        assert!(
            result.is_ok(),
            "filter-test import must succeed: {result:?}"
        );
        let manifest_out = result.unwrap();

        // Only the Required file's mock must have been hit
        mock_required.assert_calls(1);
        mock_unsupported.assert_calls(0);

        // Only the Required mod jar should exist in the mods dir
        let mods_dir = paths
            .instance_minecraft_dir(&manifest_out.slug)
            .join("mods");
        let jars: Vec<_> = std::fs::read_dir(&mods_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "jar").unwrap_or(false))
            .collect();
        assert_eq!(jars.len(), 1, "only 1 mod jar must exist; got: {jars:?}");
        assert!(
            jars[0].file_name().to_str().unwrap().contains("client-req"),
            "only client-req.jar must be present"
        );
    }
}
