//! CurseForge service facade — composes the HTTP client, API-key resolver,
//! download-url fallback, hash-algo-parameterized installer, and ledger into
//! the surface the TUI consumes.
//!
//! Mirrors `src/mods/service.rs` (ModrinthService) field-by-field with these
//! Phase 9-specific deltas:
//! - `client: Option<CurseForgeClient>` (Pitfall 1 fail-fast: launcher continues
//!   even when no API key is configured; methods return Err(NoApiKey) instead of
//!   the entire service failing to construct).
//! - `api_key_present: bool` field surfaced via `pub fn api_key_present()` for
//!   the F-keybind guard in run.rs (09-07).
//! - Single-mod install path (no dep resolution per 09-RESEARCH.md Q4).
//! - SHA-1 verify via `download_one_with_hash_algo` from 09-05 Task 1.
//! - Pitfall 8 atomicity protocol: ledger upsert BEFORE tmp→final.jar rename.
//!
//! Every public async method carries `#[tracing::instrument(skip_all, fields(...))]`.
//! API key NEVER appears in any tracing field (CI grep enforced).

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::loader::types::LoaderInfo;
use crate::mods::curseforge::api_key as cf_api_key;
use crate::mods::curseforge::client::{CurseForgeClient, SEARCH_DEFAULT_PAGE_SIZE};
use crate::mods::curseforge::error::CurseForgeError;
use crate::mods::curseforge::filter::curseforge_loader_type;
use crate::mods::curseforge::installer::{resolve_download_url, DownloadResolution};
use crate::mods::curseforge::types::{
    CurseForgeFileEntry, CurseForgeProjectDetail, CurseForgeSearchHit,
};
use crate::mods::error::ModrinthError;
use crate::mods::installer::{download_one_with_hash_algo, MOD_DOWNLOAD_CONCURRENCY};
use crate::mods::ledger::{per_instance_lock, read_ledger, upsert_mod};
use crate::mods::types::{HashAlgo, InstalledModRow, ModSource};
use crate::persistence::paths::AppPaths;
use crate::tasks::{JobId, TaskEvent};

// Documents the inheritance from Phase 8's parallel install pipeline. v1
// CurseForge install is single-mod, so the constant isn't actually used at
// call sites; keeping the const-binding tie ensures any rename in Phase 8
// surfaces here as a compile error.
const _: usize = MOD_DOWNLOAD_CONCURRENCY;

/// CurseForge service facade.
///
/// Holds `Option<CurseForgeClient>` (Pitfall 1 fail-fast). When the API key
/// is absent, every method returns `Err(CurseForgeError::NoApiKey)` and the
/// launcher continues to function normally (the F keybind silently no-ops via
/// `api_key_present()` guard in run.rs 09-07).
pub struct CurseForgeService {
    client: Option<CurseForgeClient>,
    api_key_present: bool,
}

/// Inline struct for parsing the relevant slice of config.toml.
///
/// Per MOD-08, config.toml `[api_keys] curseforge` field MUST be honored.
/// All defaults are `None` so missing file / missing table / missing field
/// gracefully degrades to env/compiled-in fallback.
#[derive(serde::Deserialize, Default)]
struct AppConfigSlim {
    #[serde(default)]
    api_keys: Option<ApiKeysSection>,
}

#[derive(serde::Deserialize, Default)]
struct ApiKeysSection {
    #[serde(default)]
    curseforge: Option<String>,
}

/// Read `~/.config/mineltui/config.toml` `[api_keys] curseforge` field.
///
/// Best-effort: returns None on any of (a) `AppPaths::resolve` returning None
/// (directories crate failed); (b) the file not existing; (c) a parse error;
/// (d) a missing `[api_keys]` table; (e) a missing `curseforge` field;
/// (f) a whitespace-only value. The empty-string filter matches the
/// `cf_api_key::resolve_api_key` empty-skip behavior (one bug surface
/// instead of two).
fn read_config_curseforge_key() -> Option<String> {
    AppPaths::resolve()
        .and_then(|p| std::fs::read_to_string(p.app_config_file()).ok())
        .and_then(|s| toml::from_str::<AppConfigSlim>(&s).ok())
        .and_then(|c| c.api_keys.and_then(|k| k.curseforge))
        .filter(|s| !s.trim().is_empty())
}

impl CurseForgeService {
    /// Construct the service, running the precedence chain:
    ///   env (`CURSEFORGE_API_KEY`) > config.toml (`[api_keys] curseforge`)
    ///   > compiled-in default (`option_env!("MINELTUI_CURSEFORGE_API_KEY_DEFAULT")`)
    ///   > NoApiKey (Pitfall 1: returns `Ok` with `api_key_present: false`).
    ///
    /// MOD-08 invariant: config.toml IS read here. Pass-None is FORBIDDEN.
    #[tracing::instrument(skip_all)]
    pub fn new() -> Result<Self, CurseForgeError> {
        let config_value: Option<String> = read_config_curseforge_key();
        match cf_api_key::resolve_runtime(config_value.as_deref()) {
            Ok(key) => {
                let client = CurseForgeClient::new(&key)?;
                Ok(Self {
                    client: Some(client),
                    api_key_present: true,
                })
            }
            Err(_no_key) => {
                // Pitfall 1 fail-fast: do NOT propagate NoApiKey as Err.
                // The launcher continues to function for Modrinth + everything else;
                // the F keybind silently no-ops via api_key_present()=false.
                tracing::info!("CurseForge API key not configured; F keybind disabled");
                Ok(Self {
                    client: None,
                    api_key_present: false,
                })
            }
        }
    }

    /// Test ctor for httpmock injection.
    pub fn with_client(client: CurseForgeClient) -> Self {
        Self {
            client: Some(client),
            api_key_present: true,
        }
    }

    /// True iff `new()` resolved a non-empty API key. Read by run.rs F-keybind guard.
    pub fn api_key_present(&self) -> bool {
        self.api_key_present
    }

    // ====================================================================
    // === Read-only browse                                              ===
    // ====================================================================

    /// Search CurseForge with optional MC version + loader filter.
    /// Stamps `already_installed` against the per-instance ledger before returning.
    #[tracing::instrument(skip_all, fields(query = %query, mc = ?mc, slug = ?slug))]
    pub async fn search(
        &self,
        query: &str,
        mc: Option<&str>,
        loader: Option<&LoaderInfo>,
        paths: Option<&AppPaths>,
        slug: Option<&str>,
    ) -> Result<Vec<CurseForgeSearchHit>, CurseForgeError> {
        let client = self.client.as_ref().ok_or(CurseForgeError::NoApiKey)?;
        let loader_type = curseforge_loader_type(loader);
        let mut hits = client
            .search(query, mc, loader_type, SEARCH_DEFAULT_PAGE_SIZE)
            .await?;

        // Best-effort ledger stamping (mirrors Phase 8 ModrinthService).
        if let (Some(p), Some(s)) = (paths, slug) {
            if let Ok(led) = read_ledger(p, s).await {
                let installed: std::collections::HashSet<String> =
                    led.mods.iter().map(|m| m.mod_id.clone()).collect();
                for h in &mut hits {
                    if installed.contains(&h.id.to_string()) {
                        h.already_installed = true;
                    }
                }
            }
        }
        Ok(hits)
    }

    /// Fetch full mod detail by id.
    #[tracing::instrument(skip_all, fields(mod_id))]
    pub async fn get_mod(
        &self,
        mod_id: u64,
    ) -> Result<CurseForgeProjectDetail, CurseForgeError> {
        let client = self.client.as_ref().ok_or(CurseForgeError::NoApiKey)?;
        client.get_mod(mod_id).await
    }

    /// List files for a mod, filtered by MC version + loader.
    #[tracing::instrument(skip_all, fields(mod_id, mc = ?mc))]
    pub async fn list_files(
        &self,
        mod_id: u64,
        mc: Option<&str>,
        loader: Option<&LoaderInfo>,
    ) -> Result<Vec<CurseForgeFileEntry>, CurseForgeError> {
        let client = self.client.as_ref().ok_or(CurseForgeError::NoApiKey)?;
        let loader_type = curseforge_loader_type(loader);
        client.list_files(mod_id, mc, loader_type).await
    }

    // ====================================================================
    // === Install pipeline                                              ===
    // ====================================================================

    /// Install a single CurseForge file into an instance's mods directory.
    ///
    /// Pipeline (Pitfall 8 atomicity protocol):
    ///   1. resolve URL via 09-04 helper (handles inline + downloadUrl-null fallback)
    ///   2. extract SHA-1 from file.hashes (algo==1) — error if absent
    ///   3. acquire per-instance ledger lock (cross-source: shared with Phase 8 Modrinth)
    ///   4. download to <file>.tmp with SHA-1 verify via download_one_with_hash_algo
    ///   5. upsert ledger row (BEFORE rename — Pitfall 8)
    ///   6. atomic rename .tmp → final.jar
    ///   7. release lock
    ///
    /// On FileNotDownloadable: returns the error directly; caller (run.rs) maps to
    /// Action::CfModInstallFailed with web_url for the modal.
    #[tracing::instrument(
        skip_all,
        fields(slug = %slug, mod_id = mod_detail.id, file_id = file.id)
    )]
    #[allow(clippy::too_many_arguments)]
    pub async fn install_mod_into_instance(
        &self,
        paths: &AppPaths,
        slug: &str,
        mod_detail: &CurseForgeProjectDetail,
        file: &CurseForgeFileEntry,
        progress_tx: mpsc::Sender<TaskEvent>,
        token: CancellationToken,
        job_id: JobId,
    ) -> Result<(), CurseForgeError> {
        let client = self.client.as_ref().ok_or(CurseForgeError::NoApiKey)?;

        // Pitfall 8 atomicity protocol: the per-instance ledger lock is acquired
        // INSIDE `upsert_mod` (Phase 8 invariant — `tokio::sync::Mutex` is NOT
        // reentrant, so taking it here AND inside `upsert_mod` deadlocks). We
        // touch the lock map at the start so the entry exists for the slug, and
        // serialization between Modrinth + CurseForge installs on the same
        // instance is provided by `upsert_mod` itself (both sources call into
        // `per_instance_lock(slug)` → same `Arc<Mutex<()>>` per slug).
        let _lock = per_instance_lock(slug);

        // 1. Resolve URL (handles inline + downloadUrl-null fallback chain).
        let DownloadResolution::Resolved(url) =
            resolve_download_url(client, mod_detail, file).await?;

        // 2. Extract SHA-1 (CurseForge canonical algo=1).
        let sha1 = file
            .hashes
            .iter()
            .find(|h| h.algo == 1)
            .map(|h| h.value.clone())
            .ok_or_else(|| {
                CurseForgeError::Http(format!(
                    "no SHA-1 hash on file {}",
                    file.file_name
                ))
            })?;

        // 3. Build dest paths.
        let dest_final = paths.instance_mod_file(slug, &file.file_name);
        let dest_tmp = {
            let mut s = dest_final.clone().into_os_string();
            s.push(".tmp");
            std::path::PathBuf::from(s)
        };

        // 4. Stream-download with SHA-1 verify.
        let http = client.http().clone();
        download_one_with_hash_algo(
            &http,
            &url,
            &sha1,
            HashAlgo::Sha1,
            &dest_tmp,
            &file.file_name,
            file.file_length,
            &progress_tx,
            job_id,
            &token,
            0,
            1,
        )
        .await
        .map_err(|e| match e {
            ModrinthError::Cancelled => CurseForgeError::Cancelled,
            ModrinthError::Sha512Mismatch { url, expected, got } => {
                CurseForgeError::ShaMismatch {
                    algo: "sha1",
                    url,
                    expected,
                    got,
                }
            }
            ModrinthError::FileNotDownloadable { project_slug: _ } => {
                CurseForgeError::FileNotDownloadable {
                    web_url: crate::mods::curseforge::url::web_url_for_file(
                        &mod_detail.slug,
                        file.id,
                    ),
                    mod_slug: mod_detail.slug.clone(),
                    file_id: file.id,
                }
            }
            ModrinthError::Io(io_err) => CurseForgeError::Io(io_err),
            other => CurseForgeError::Http(other.to_string()),
        })?;

        // 5. Build + upsert ledger row (BEFORE rename — Pitfall 8).
        let row = InstalledModRow {
            mod_id: mod_detail.id.to_string(),
            project_slug: mod_detail.slug.clone(),
            display_name: mod_detail.name.clone(),
            version_id: file.id.to_string(),
            version_label: file.display_name.clone(),
            file_name: file.file_name.clone(),
            sha512: sha1.clone(), // historical field name; stores SHA-1 hex when hash_algo=Sha1
            size: file.file_length,
            source: ModSource::CurseForge,
            enabled: true,
            installed_at: crate::domain::instance::now_iso8601_utc(),
            hash_algo: HashAlgo::Sha1,
        };
        upsert_mod(paths, slug, row).await.map_err(|e| match e {
            ModrinthError::Io(io_err) => CurseForgeError::Io(io_err),
            other => CurseForgeError::Http(format!("ledger upsert: {other}")),
        })?;

        // 6. Atomic rename .tmp → final.jar.
        tokio::fs::rename(&dest_tmp, &dest_final).await.map_err(|e| {
            let _ = std::fs::remove_file(&dest_tmp);
            CurseForgeError::Io(std::io::Error::other(format!(
                "rename {} -> {}: {e}",
                dest_tmp.display(),
                dest_final.display(),
            )))
        })?;

        // 7. Terminal progress event.
        let _ = progress_tx
            .send(TaskEvent::Progress {
                id: job_id,
                pct: 100,
                msg: format!("Installed {}", file.file_name),
            })
            .await;

        Ok(())
    }
}

// ============================================================================
// === Tests                                                                ===
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::curseforge::types::{
        CurseForgeAuthor, CurseForgeHash, CurseForgeLinks,
    };
    use httpmock::prelude::*;
    use tempfile::TempDir;

    fn paths_for(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    fn fx_detail(id: u64, slug: &str) -> CurseForgeProjectDetail {
        CurseForgeProjectDetail {
            id,
            slug: slug.into(),
            name: "X".into(),
            summary: String::new(),
            description: String::new(),
            download_count: 0,
            authors: vec![CurseForgeAuthor {
                id: 1,
                name: "Author".into(),
                url: String::new(),
            }],
            links: CurseForgeLinks::default(),
        }
    }

    #[test]
    fn test_with_client_sets_api_key_present_true() {
        let server = MockServer::start();
        let client =
            CurseForgeClient::new_with_base_url("test-key", server.base_url()).unwrap();
        let svc = CurseForgeService::with_client(client);
        assert!(svc.api_key_present());
    }

    #[tokio::test]
    async fn test_methods_return_no_api_key_when_disabled() {
        let svc = CurseForgeService {
            client: None,
            api_key_present: false,
        };
        let r = svc.search("x", None, None, None, None).await;
        assert!(matches!(r, Err(CurseForgeError::NoApiKey)), "got {r:?}");
    }

    #[tokio::test]
    async fn test_install_with_inline_url_writes_ledger_row() {
        use sha1::{Digest, Sha1};
        let server = MockServer::start();
        let body = b"fake-jar-body".to_vec();
        let mut h = Sha1::new();
        h.update(&body);
        let sha1 = crate::mods::installer::sha1_hex(h.finalize().as_slice());

        // Mock CDN.
        server.mock(|when, then| {
            when.method(GET).path("/cdn/sodium.jar");
            then.status(200).body(body.clone());
        });

        let client =
            CurseForgeClient::new_with_base_url("test-key", server.base_url()).unwrap();
        let svc = CurseForgeService::with_client(client);

        let td = TempDir::new().unwrap();
        let paths = paths_for(&td);
        let slug = "happy-install";
        tokio::fs::create_dir_all(
            paths
                .instance_mod_file(slug, "_dummy.jar")
                .parent()
                .unwrap(),
        )
        .await
        .unwrap();

        let detail = fx_detail(443959, "sodium");
        let file = CurseForgeFileEntry {
            id: 4567890,
            display_name: "Sodium 0.5.8".into(),
            file_name: "sodium-fabric.jar".into(),
            release_type: 1,
            file_status: 4,
            hashes: vec![CurseForgeHash {
                value: sha1.clone(),
                algo: 1,
            }],
            file_date: "2026-01-01T00:00:00Z".into(),
            file_length: body.len() as u64,
            download_count: 100,
            download_url: Some(format!("{}/cdn/sodium.jar", server.base_url())),
            game_versions: vec!["1.20.4".into()],
            dependencies: vec![],
            is_available: true,
        };

        let (tx, mut rx) = mpsc::channel(64);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let token = CancellationToken::new();

        svc.install_mod_into_instance(&paths, slug, &detail, &file, tx, token, JobId(0))
            .await
            .expect("install");

        let ledger_raw =
            tokio::fs::read_to_string(paths.instance_mod_ledger(slug)).await.unwrap();
        let ledger: crate::mods::types::Ledger = toml::from_str(&ledger_raw).unwrap();
        assert_eq!(ledger.mods.len(), 1);
        let row = &ledger.mods[0];
        assert_eq!(row.source, ModSource::CurseForge);
        assert_eq!(row.hash_algo, HashAlgo::Sha1);
        assert_eq!(row.mod_id, "443959");
        assert_eq!(row.version_id, "4567890");

        // Final file present at instance_mod_file path.
        let final_path = paths.instance_mod_file(slug, "sodium-fabric.jar");
        assert!(final_path.exists(), "final jar must exist: {final_path:?}");
        // .tmp must be gone after rename.
        let mut tmp = final_path.clone().into_os_string();
        tmp.push(".tmp");
        assert!(
            !std::path::PathBuf::from(tmp).exists(),
            ".tmp must be renamed away"
        );
    }

    #[tokio::test]
    async fn test_install_with_null_download_url_no_ledger_row() {
        let server = MockServer::start();
        // /download-url fallback returns 404 → FileNotDownloadable per 09-04.
        server.mock(|when, then| {
            when.method(GET)
                .path("/v1/mods/443959/files/4567890/download-url");
            then.status(404).body(r#"{"error":"restricted"}"#);
        });

        let client =
            CurseForgeClient::new_with_base_url("test-key", server.base_url()).unwrap();
        let svc = CurseForgeService::with_client(client);

        let td = TempDir::new().unwrap();
        let paths = paths_for(&td);
        let slug = "restricted-test";

        let detail = fx_detail(443959, "wonderful-world-mod");
        let file = CurseForgeFileEntry {
            id: 4567890,
            display_name: "Wonderful World 1.5.0".into(),
            file_name: "wwm.jar".into(),
            release_type: 1,
            file_status: 4,
            hashes: vec![CurseForgeHash {
                value: "abc".into(),
                algo: 1,
            }],
            file_date: "2026-01-01T00:00:00Z".into(),
            file_length: 1024,
            download_count: 100,
            download_url: None, // CRITICAL: triggers null fallback
            game_versions: vec!["1.20.4".into()],
            dependencies: vec![],
            is_available: true,
        };

        let (tx, mut rx) = mpsc::channel(64);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let token = CancellationToken::new();

        let r = svc
            .install_mod_into_instance(&paths, slug, &detail, &file, tx, token, JobId(0))
            .await;
        match r {
            Err(CurseForgeError::FileNotDownloadable {
                web_url,
                mod_slug,
                file_id,
            }) => {
                assert!(
                    web_url.starts_with("https://www.curseforge.com/minecraft/mc-mods/"),
                    "web_url shape: {web_url}"
                );
                assert_eq!(mod_slug, "wonderful-world-mod");
                assert_eq!(file_id, 4567890);
            }
            other => panic!("expected FileNotDownloadable, got {other:?}"),
        }
        // Atomicity: NO ledger row.
        let ledger_path = paths.instance_mod_ledger(slug);
        if ledger_path.exists() {
            let raw = tokio::fs::read_to_string(&ledger_path).await.unwrap();
            let ledger: crate::mods::types::Ledger =
                toml::from_str(&raw).unwrap_or_default();
            assert!(ledger.mods.is_empty(), "no ledger row on FileNotDownloadable");
        }
        // No orphan tmp file at the final destination.
        let final_path = paths.instance_mod_file(slug, "wwm.jar");
        assert!(!final_path.exists(), "no final jar must exist");
        let mut tmp = final_path.clone().into_os_string();
        tmp.push(".tmp");
        assert!(
            !std::path::PathBuf::from(tmp).exists(),
            "no orphan .tmp file"
        );
    }

    // --- config.toml support tests (MOD-08) ------------------------------------
    // The 3 sub-cases (a)/(b)/(c) operate on the AppConfigSlim parser directly,
    // exercising the load-bearing logic in `read_config_curseforge_key` without
    // mocking AppPaths::resolve. Case (c) (file absent) is structurally
    // equivalent — read_config_curseforge_key short-circuits at the
    // .ok().and_then(...) chain — and is verified by the live smoke in 09-08.

    #[test]
    fn test_app_config_slim_parses_curseforge_key_when_present() {
        // Case (a): file present with [api_keys] curseforge = "test-key" → key extracted.
        let toml_src = r#"
            [api_keys]
            curseforge = "test-key"
        "#;
        let parsed: AppConfigSlim = toml::from_str(toml_src).unwrap();
        assert_eq!(
            parsed.api_keys.and_then(|k| k.curseforge),
            Some("test-key".to_string())
        );
    }

    #[test]
    fn test_app_config_slim_falls_through_when_no_api_keys_table() {
        // Case (b): file present without [api_keys] → None (falls through to env).
        let toml_src = r#"
            [other_section]
            foo = "bar"
        "#;
        let parsed: AppConfigSlim = toml::from_str(toml_src).unwrap();
        assert!(parsed.api_keys.is_none());
    }

    #[test]
    fn test_app_config_slim_falls_through_when_api_keys_lacks_curseforge() {
        // Case (b'): file present with [api_keys] but no curseforge field → None.
        let toml_src = r#"
            [api_keys]
            modrinth = "ignored"
        "#;
        let parsed: AppConfigSlim = toml::from_str(toml_src).unwrap();
        assert!(parsed.api_keys.unwrap().curseforge.is_none());
    }
    // Case (c): file absent → read_config_curseforge_key returns None via the
    // AppPaths::resolve.and_then(read_to_string).and_then(...) chain. The empty-
    // string filter at the end handles whitespace-only values.
}
