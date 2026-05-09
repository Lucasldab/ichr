//! Pack service façade -- composes Modrinth client + ledger ops for resource
//! packs and shader packs. Mirrors `src/mods/service.rs::ModrinthService`
//! field-for-field with PackKind parametrization.
//!
//! Per CONTEXT.md D-LOCK module symmetry: ONE service for both pack kinds,
//! parameterized by PackKind on every method.
//!
//! Pitfall 1: DO NOT delegate toggle/uninstall to `src/mods/ledger::toggle_enabled`
//! or `src/mods/ledger::uninstall` -- those functions hardcode the `mods/`
//! subdirectory. We rebuild toggle + uninstall inline using `instance_packs_dir`.
//!
//! Pitfall 2: DO NOT call `is_safe_mod_filename` anywhere in this file.
//! Packs are `.zip`, not `.jar`. Use `is_safe_pack_filename` exclusively.

use std::collections::HashSet;

use futures::StreamExt;
use sha1::Sha1;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::mods::error::ModrinthError;
use crate::mods::filter::{
    disabled_filename, is_safe_pack_filename, pick_primary_file, sanitize_pack_filename,
    MAX_PACK_FILE_BYTES,
};
use crate::mods::installer::{sha1_hex, MOD_DOWNLOAD_CONCURRENCY};
use crate::mods::ledger::{per_instance_lock, read_ledger, upsert_pack, write_ledger};
use crate::mods::modrinth::client::SEARCH_DEFAULT_LIMIT;
use crate::mods::modrinth::ModrinthClient;
use crate::mods::types::{
    HashAlgo, InstalledItemKind, InstalledModRow, ModSource, ModrinthSearchHit, ModrinthVersion,
    ModrinthVersionEntry,
};
use crate::packs::error::PackError;
use crate::packs::kind::PackKind;
use crate::persistence::paths::AppPaths;
use crate::tasks::{JobId, TaskEvent};

/// Re-export so consumers can reference one canonical const.
pub use crate::mods::installer::MOD_DOWNLOAD_CONCURRENCY as PARALLEL_DOWNLOAD_CAP;
const _: usize = MOD_DOWNLOAD_CONCURRENCY; // tie alias to const so renames cascade

/// True iff the URL is acceptable for a pack-file download.
/// Production rule: must be `https://`. Loopback (`http://127.0.0.1:*` or
/// `http://localhost:*`) allowed for httpmock-backed unit tests.
fn is_acceptable_pack_url(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:")
}

/// Map PackKind discriminator → ledger InstalledItemKind discriminator.
///
/// Internal helper -- keeps the kind ↔ item_kind mapping in one place so the
/// ledger filter in `list_installed` and the row construction in
/// `install_modrinth` agree.
fn pack_kind_to_item_kind(kind: PackKind) -> InstalledItemKind {
    match kind {
        PackKind::Resource => InstalledItemKind::ResourcePack,
        PackKind::Shader => InstalledItemKind::Shader,
    }
}

#[derive(Debug)]
pub struct PackService {
    client: ModrinthClient,
}

impl PackService {
    #[tracing::instrument(skip_all)]
    pub fn new() -> Result<Self, PackError> {
        Ok(Self {
            client: ModrinthClient::new()?,
        })
    }

    /// Construct a service from an already-built client.
    ///
    /// Used by unit tests with httpmock-injected base URLs and by integration
    /// tests (plan 05) that exercise the full service stack against a mock server.
    pub fn with_client(client: ModrinthClient) -> Self {
        Self { client }
    }

    // ====================================================================
    // === Read-only browse                                              ===
    // ====================================================================

    /// Search Modrinth for packs of the given `kind`.
    ///
    /// D-LOCK: NO loader filter for packs. Passes empty loaders slice so
    /// `search_facets` omits the `categories:` group entirely.
    ///
    /// HIGH-1 invariant: project_type facet is emitted UNCONDITIONALLY,
    /// even when `mc == None`, to prevent Modrinth returning mods mixed in.
    /// Uses `ModrinthClient::search_with_project_type` (not `search`) for
    /// this reason.
    ///
    /// Stamps `already_installed` against the per-instance ledger filtered
    /// by kind before returning.
    #[tracing::instrument(skip_all, fields(query = %query, kind = ?kind, mc = ?mc, slug = ?slug))]
    pub async fn search(
        &self,
        query: &str,
        kind: PackKind,
        mc: Option<&str>,
        paths: Option<&AppPaths>,
        slug: Option<&str>,
    ) -> Result<Vec<ModrinthSearchHit>, PackError> {
        // D-LOCK: empty loaders slice -- packs are not loader-specific.
        let mut hits = self
            .client
            .search_with_project_type(
                query,
                mc,
                &[],
                kind.modrinth_project_type(),
                SEARCH_DEFAULT_LIMIT,
            )
            .await?;

        // Stamp already_installed from the ledger (best-effort -- if read fails, leave false).
        // Filter by kind so a Mod row with the same project_id does not cause false positives.
        if let (Some(p), Some(s)) = (paths, slug) {
            if let Ok(led) = read_ledger(p, s).await {
                let item_kind = pack_kind_to_item_kind(kind);
                let installed: HashSet<&str> = led
                    .mods
                    .iter()
                    .filter(|m| m.kind == item_kind)
                    .map(|m| m.mod_id.as_str())
                    .collect();
                for h in &mut hits {
                    if installed.contains(h.project_id.as_str()) {
                        h.already_installed = true;
                    }
                }
            }
        }
        Ok(hits)
    }

    /// List versions for a Modrinth project filtered only by MC version (no loader filter).
    ///
    /// D-LOCK: packs are not loader-specific. Empty loaders slice passed so
    /// `ModrinthClient::list_versions` emits no `loaders=` query param.
    #[tracing::instrument(skip_all, fields(project_id = %project_id, mc = ?mc, kind = ?kind))]
    pub async fn list_versions(
        &self,
        project_id: &str,
        mc: Option<&str>,
        kind: PackKind,
    ) -> Result<Vec<ModrinthVersionEntry>, PackError> {
        let _ = kind; // kind kept for API symmetry with install_modrinth; unused for versioning
        let mut versions = self.client.list_versions(project_id, mc, &[]).await?;
        versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));

        let mut latest_stable_marked = false;
        let entries: Vec<ModrinthVersionEntry> = versions
            .into_iter()
            .map(|v| {
                let channel = v.version_type.clone();
                let is_latest_stable = if !latest_stable_marked && channel == "release" {
                    latest_stable_marked = true;
                    true
                } else {
                    false
                };
                ModrinthVersionEntry {
                    version_id: v.id,
                    version_label: v.version_number,
                    channel,
                    is_latest_stable,
                }
            })
            .collect();
        Ok(entries)
    }

    // ====================================================================
    // === Install pipeline                                              ===
    // ====================================================================

    /// Download, verify, and install a Modrinth pack file into the instance.
    ///
    /// Atomicity protocol (Pitfall 8): upsert_pack into ledger BEFORE rename so
    /// that a mid-rename crash leaves the ledger in the correct state.
    ///
    /// Pitfall 2: uses `is_safe_pack_filename` (NOT `is_safe_mod_filename`).
    /// Inlines a streaming download loop (NOT calling `download_one_with_hash_algo`
    /// which gates on `is_safe_mod_filename`).
    ///
    /// SHA-1 verification per RESEARCH.md: Modrinth always carries sha1 on pack
    /// files. SHA-1 is stored in `InstalledModRow.sha512` with `hash_algo = Sha1`
    /// (historical-naming carve-out, Phase 9 precedent).
    #[tracing::instrument(skip_all, fields(slug = %slug, kind = ?kind))]
    #[allow(clippy::too_many_arguments)]
    pub async fn install_modrinth(
        &self,
        paths: &AppPaths,
        slug: &str,
        kind: PackKind,
        version: &ModrinthVersion,
        project_slug: &str,
        project_id: &str,
        project_title: &str,
        progress_tx: mpsc::Sender<TaskEvent>,
        token: CancellationToken,
        job_id: JobId,
    ) -> Result<InstalledModRow, PackError> {
        // Step 1: pick primary file.
        let file = pick_primary_file(&version.files).ok_or_else(|| {
            PackError::Modrinth(ModrinthError::FileNotDownloadable {
                project_slug: project_slug.to_string(),
            })
        })?;

        // Step 2 (T-11-02-01): validate filename BEFORE any HTTP call.
        // Modrinth pack uploads regularly include Minecraft formatting codes
        // (`§6`, `§r`) and bracket characters in filenames. Project them
        // through `sanitize_pack_filename` so legitimate uploads are not
        // rejected; the strict allowlist still gates the result, so a
        // sanitization that somehow produces a path-traversing name is
        // still refused.
        let safe_filename = sanitize_pack_filename(&file.filename).filter(|n| {
            is_safe_pack_filename(n)
        });
        let safe_filename = match safe_filename {
            Some(n) => n,
            None => {
                return Err(PackError::UnsafeFilename {
                    filename: file.filename.clone(),
                })
            }
        };

        // Step 3 (T-11-02-02): validate URL (HTTPS-only; loopback for test).
        if !is_acceptable_pack_url(&file.url) {
            return Err(PackError::Modrinth(ModrinthError::FileNotDownloadable {
                project_slug: format!("{} (non-https URL: {})", file.filename, file.url),
            }));
        }

        // Step 4 (T-11-02-03): pre-send size cap check.
        if file.size > MAX_PACK_FILE_BYTES {
            return Err(PackError::FileTooLarge {
                bytes: file.size,
                cap: MAX_PACK_FILE_BYTES,
            });
        }

        // Step 5: cancellation gate.
        if token.is_cancelled() {
            return Err(PackError::Cancelled);
        }

        // Step 6: compute dest paths. On-disk uses the sanitized filename
        // (`safe_filename`) so the bytes hitting the filesystem are
        // allowlist-clean; the original `file.filename` survives only in
        // log/progress messages for user reference.
        let final_path = paths.instance_pack_file(slug, kind, &safe_filename);
        let tmp_path = {
            let mut p = final_path.clone();
            let mut name = p.file_name().unwrap_or_default().to_os_string();
            name.push(".tmp");
            p.set_file_name(name);
            p
        };

        // Step 7 (Pitfall 3 -- old instances): ensure dest dir exists.
        tokio::fs::create_dir_all(paths.instance_packs_dir(slug, kind))
            .await
            .map_err(PackError::Io)?;

        // Step 8: streaming download + SHA-1 verify (inline -- Pitfall 2).
        let resp = self
            .client
            .http()
            .get(&file.url)
            .send()
            .await
            .map_err(|e| {
                PackError::Modrinth(ModrinthError::Http(format!("GET {}: {e}", file.url)))
            })?
            .error_for_status()
            .map_err(|e| {
                PackError::Modrinth(ModrinthError::Http(format!("status {}: {e}", file.url)))
            })?;

        let mut buf: Vec<u8> = Vec::with_capacity(file.size as usize);
        let mut bytes_done: u64 = 0;
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            if token.is_cancelled() {
                return Err(PackError::Cancelled);
            }
            let chunk = chunk.map_err(|e| {
                PackError::Modrinth(ModrinthError::Http(format!("body {}: {e}", file.url)))
            })?;
            bytes_done += chunk.len() as u64;
            // T-11-02-03: mid-stream cap (defense in depth against tampered Content-Length).
            if bytes_done > MAX_PACK_FILE_BYTES {
                return Err(PackError::FileTooLarge {
                    bytes: bytes_done,
                    cap: MAX_PACK_FILE_BYTES,
                });
            }
            buf.extend_from_slice(&chunk);

            // Per-chunk progress emit (single-file install -- completed=0, total=1).
            let intra_pct: u64 = if file.size == 0 {
                100
            } else {
                (bytes_done.saturating_mul(100)) / file.size.max(1)
            };
            let pct = intra_pct.min(99) as u8;
            let _ = progress_tx
                .send(TaskEvent::Progress {
                    id: job_id,
                    pct,
                    msg: format!("Downloading pack: {}", file.filename),
                })
                .await;
        }

        // Step 9 (T-11-02-04): SHA-1 hash verify (case-insensitive, Pitfall 3).
        let got = {
            use sha1::Digest as _;
            let mut h = Sha1::new();
            h.update(&buf);
            sha1_hex(h.finalize().as_slice())
        };
        if !got.eq_ignore_ascii_case(&file.hashes.sha1) {
            return Err(PackError::Modrinth(ModrinthError::Sha512Mismatch {
                url: file.url.clone(),
                expected: file.hashes.sha1.clone(),
                got,
            }));
        }

        // Step 10: write to .tmp file.
        tokio::fs::write(&tmp_path, &buf).await.map_err(|e| {
            PackError::Io(std::io::Error::other(format!(
                "write {}: {e}",
                tmp_path.display()
            )))
        })?;

        // Step 11: build ledger row.
        let installed_at = crate::domain::instance::now_iso8601_utc();
        let row = InstalledModRow {
            mod_id: project_id.to_string(),
            project_slug: project_slug.to_string(),
            display_name: project_title.to_string(),
            version_id: version.id.clone(),
            version_label: version.version_number.clone(),
            file_name: safe_filename.clone(),
            // Historical-naming carve-out: sha1 hex stored in sha512 field.
            sha512: file.hashes.sha1.clone(),
            size: file.size,
            hash_algo: HashAlgo::Sha1,
            source: ModSource::Modrinth,
            kind: pack_kind_to_item_kind(kind),
            enabled: true,
            installed_at,
        };

        // Step 12 (Pitfall 8): upsert_pack BEFORE rename.
        upsert_pack(paths, slug, row.clone()).await?;

        // Step 13: rename .tmp → final path.
        if let Err(e) = tokio::fs::rename(&tmp_path, &final_path).await {
            // Best-effort cleanup of .tmp on rename failure.
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(PackError::Io(std::io::Error::other(format!(
                "rename {} -> {}: {e}",
                tmp_path.display(),
                final_path.display()
            ))));
        }

        Ok(row)
    }

    // ====================================================================
    // === Ledger ops                                                    ===
    // ====================================================================

    /// List packs installed in the instance, filtered to the given `kind`.
    #[tracing::instrument(skip_all, fields(slug = %slug, kind = ?kind))]
    pub async fn list_installed(
        &self,
        paths: &AppPaths,
        slug: &str,
        kind: PackKind,
    ) -> Result<Vec<InstalledModRow>, PackError> {
        let item_kind = pack_kind_to_item_kind(kind);
        Ok(read_ledger(paths, slug)
            .await?
            .mods
            .into_iter()
            .filter(|r| r.kind == item_kind)
            .collect())
    }

    /// Toggle a resource pack between enabled and disabled.
    ///
    /// D-LOCK: shader packs have no enable/disable concept (Iris/OptiFine
    /// manage shader selection in their own config). Returns
    /// `PackError::ShaderToggleNotSupported` for `PackKind::Shader`.
    ///
    /// Pitfall 1: does NOT delegate to `src/mods/ledger::toggle_enabled` (which
    /// hardcodes the `mods/` subdirectory). Rebuilds toggle inline using
    /// `instance_packs_dir(slug, kind)`.
    #[tracing::instrument(skip_all, fields(slug = %slug, kind = ?kind, mod_id = %mod_id))]
    pub async fn toggle_pack_enabled(
        &self,
        paths: &AppPaths,
        slug: &str,
        mod_id: &str,
        kind: PackKind,
    ) -> Result<bool, PackError> {
        // D-LOCK: shader packs have no enable/disable.
        if kind == PackKind::Shader {
            return Err(PackError::ShaderToggleNotSupported { kind });
        }

        let lock = per_instance_lock(slug);
        let _guard = lock.lock().await;

        let mut ledger = read_ledger(paths, slug).await?;
        let item_kind = pack_kind_to_item_kind(kind);
        let row = ledger
            .mods
            .iter_mut()
            .find(|m| m.mod_id == mod_id && m.kind == item_kind)
            .ok_or_else(|| PackError::Modrinth(ModrinthError::ModNotFound(mod_id.to_string())))?;

        // V5: validate filename before building fs path.
        if !is_safe_pack_filename(&row.file_name) {
            return Err(PackError::UnsafeFilename {
                filename: row.file_name.clone(),
            });
        }

        // Use instance_packs_dir (NOT mods_dir) -- Pitfall 1.
        let packs_dir = paths.instance_packs_dir(slug, kind);
        let (current_path, new_path) = if row.enabled {
            (
                packs_dir.join(&row.file_name),
                packs_dir.join(disabled_filename(&row.file_name)),
            )
        } else {
            (
                packs_dir.join(disabled_filename(&row.file_name)),
                packs_dir.join(&row.file_name),
            )
        };

        tokio::fs::rename(&current_path, &new_path)
            .await
            .map_err(|e| {
                PackError::Io(std::io::Error::other(format!(
                    "rename {} -> {}: {e}",
                    current_path.display(),
                    new_path.display()
                )))
            })?;

        row.enabled = !row.enabled;
        let new_state = row.enabled;
        write_ledger(paths, slug, &ledger).await?;
        Ok(new_state)
    }

    /// Remove a pack file and its ledger row.
    ///
    /// Defensive behavior: if the file is already missing (user manually
    /// deleted), drops the ledger row anyway (mirrors `src/mods/ledger::uninstall`
    /// NotFound-tolerance, but uses `instance_packs_dir` -- Pitfall 1).
    #[tracing::instrument(skip_all, fields(slug = %slug, kind = ?kind, mod_id = %mod_id))]
    pub async fn uninstall_pack(
        &self,
        paths: &AppPaths,
        slug: &str,
        mod_id: &str,
        kind: PackKind,
    ) -> Result<(), PackError> {
        let lock = per_instance_lock(slug);
        let _guard = lock.lock().await;

        let ledger_pre = read_ledger(paths, slug).await?;
        let item_kind = pack_kind_to_item_kind(kind);
        let row = ledger_pre
            .mods
            .iter()
            .find(|m| m.mod_id == mod_id && m.kind == item_kind)
            .ok_or_else(|| PackError::Modrinth(ModrinthError::ModNotFound(mod_id.to_string())))?
            .clone();

        // Use instance_packs_dir (NOT mods_dir) -- Pitfall 1.
        let packs_dir = paths.instance_packs_dir(slug, kind);
        let target = if row.enabled {
            packs_dir.join(&row.file_name)
        } else {
            packs_dir.join(disabled_filename(&row.file_name))
        };

        // Defensive: NotFound is not an error (user may have deleted manually).
        match tokio::fs::remove_file(&target).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(
                    "uninstall_pack: pack file {} already missing, dropping ledger row anyway",
                    target.display()
                );
            }
            Err(e) => {
                return Err(PackError::Io(std::io::Error::other(format!(
                    "remove_file {}: {e}",
                    target.display()
                ))));
            }
        }

        let mut ledger = ledger_pre;
        let idx = ledger
            .mods
            .iter()
            .position(|m| m.mod_id == mod_id && m.kind == item_kind)
            .ok_or_else(|| PackError::Modrinth(ModrinthError::ModNotFound(mod_id.to_string())))?;
        ledger.mods.remove(idx);
        write_ledger(paths, slug, &ledger).await?;
        Ok(())
    }

    // ====================================================================
    // === Version accessor (for live tests and TUI install flow)        ===
    // ====================================================================

    /// Fetch a full `ModrinthVersion` by version_id.
    ///
    /// Delegates to `ModrinthClient::get_version`. Added in Plan 05 so live
    /// tests can resolve the full version object from a pinned version_id
    /// without going through the search → list_versions → select flow.
    #[tracing::instrument(skip_all, fields(version_id = %version_id))]
    pub async fn get_version(&self, version_id: &str) -> Result<ModrinthVersion, PackError> {
        Ok(self.client.get_version(version_id).await?)
    }
}

// ============================================================================
// === Tests                                                                ===
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::ledger::upsert_pack;
    use crate::mods::types::{HashAlgo, InstalledItemKind, InstalledModRow, ModSource};
    use httpmock::prelude::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_client(server: &MockServer) -> ModrinthClient {
        ModrinthClient::new_with_base_url(server.base_url()).expect("client::new_with_base_url")
    }

    fn test_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    fn make_token() -> CancellationToken {
        CancellationToken::new()
    }

    fn make_progress() -> (mpsc::Sender<TaskEvent>, mpsc::Receiver<TaskEvent>) {
        mpsc::channel(64)
    }

    // Minimal InstalledModRow for testing.
    fn pack_row(
        mod_id: &str,
        file_name: &str,
        kind: InstalledItemKind,
        enabled: bool,
    ) -> InstalledModRow {
        InstalledModRow {
            mod_id: mod_id.to_string(),
            project_slug: mod_id.to_string(),
            display_name: mod_id.to_string(),
            version_id: "v1".to_string(),
            version_label: "1.0".to_string(),
            file_name: file_name.to_string(),
            sha512: "abc123".to_string(),
            size: 100,
            hash_algo: HashAlgo::Sha1,
            source: ModSource::Modrinth,
            kind,
            enabled,
            installed_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    // --- search tests --------------------------------------------------------

    #[tokio::test]
    async fn test_search_resource_pack_uses_resourcepack_project_type() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/search")
                .query_param("query", "faithful")
                .query_param_exists("facets")
                // Must contain project_type:resourcepack in facets
                .is_true(|req| {
                    let facets = req
                        .query_params()
                        .iter()
                        .find(|(k, _)| k == "facets")
                        .map(|(_, v)| urlencoding::decode(v).unwrap_or_default().into_owned())
                        .unwrap_or_default();
                    facets.contains("project_type:resourcepack")
                });
            then.status(200).body(
                json!({
                    "hits": [
                        {"project_id":"RPID1","slug":"faithful","title":"Faithful 32x",
                         "description":"HD textures","downloads":100000}
                    ],
                    "offset":0,"limit":20,"total_hits":1
                })
                .to_string(),
            );
        });
        let svc = PackService::with_client(make_client(&server));
        let hits = svc
            .search("faithful", PackKind::Resource, Some("1.21.4"), None, None)
            .await
            .unwrap();
        m.assert();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "faithful");
    }

    #[tokio::test]
    async fn test_search_shader_pack_uses_shader_project_type() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/v2/search").is_true(|req| {
                let facets = req
                    .query_params()
                    .iter()
                    .find(|(k, _)| k == "facets")
                    .map(|(_, v)| urlencoding::decode(v).unwrap_or_default().into_owned())
                    .unwrap_or_default();
                facets.contains("project_type:shader")
            });
            then.status(200).body(
                json!({
                    "hits": [
                        {"project_id":"SHID1","slug":"bsl-shaders","title":"BSL Shaders",
                         "description":"Lighting","downloads":50000}
                    ],
                    "offset":0,"limit":20,"total_hits":1
                })
                .to_string(),
            );
        });
        let svc = PackService::with_client(make_client(&server));
        let hits = svc
            .search("bsl", PackKind::Shader, Some("1.21.4"), None, None)
            .await
            .unwrap();
        m.assert();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "bsl-shaders");
    }

    #[tokio::test]
    async fn test_search_omits_loader_filter() {
        // MockServer rejects requests whose facets contain "categories:" -- would fire
        // if a loader filter was added.
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/v2/search").is_true(|req| {
                let facets = req
                    .query_params()
                    .iter()
                    .find(|(k, _)| k == "facets")
                    .map(|(_, v)| urlencoding::decode(v).unwrap_or_default().into_owned())
                    .unwrap_or_default();
                // Must have project_type but NO categories group (D-LOCK no loader)
                facets.contains("project_type:resourcepack") && !facets.contains("categories:")
            });
            then.status(200)
                .body(json!({"hits":[], "offset":0,"limit":20,"total_hits":0}).to_string());
        });
        let svc = PackService::with_client(make_client(&server));
        let hits = svc
            .search("x", PackKind::Resource, Some("1.21.4"), None, None)
            .await
            .unwrap();
        m.assert();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn test_search_stamps_already_installed_filtered_by_kind() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/search");
            then.status(200).body(
                json!({
                    "hits": [
                        {"project_id":"RPID1","slug":"faithful","title":"Faithful 32x",
                         "description":"HD textures","downloads":100000}
                    ],
                    "offset":0,"limit":20,"total_hits":1
                })
                .to_string(),
            );
        });

        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);

        // Pre-populate ledger with a ResourcePack row for RPID1 and a Shader row
        // for a different id.
        upsert_pack(
            &paths,
            "inst",
            pack_row(
                "RPID1",
                "faithful-32x.zip",
                InstalledItemKind::ResourcePack,
                true,
            ),
        )
        .await
        .unwrap();
        // Shader row for a DIFFERENT project -- confirms kind filter works.
        upsert_pack(
            &paths,
            "inst",
            pack_row("SHID9", "some-shader.zip", InstalledItemKind::Shader, true),
        )
        .await
        .unwrap();

        let svc = PackService::with_client(make_client(&server));

        // Search for Resource packs -- RPID1 should be already_installed.
        let hits = svc
            .search(
                "faithful",
                PackKind::Resource,
                None,
                Some(&paths),
                Some("inst"),
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].already_installed,
            "ResourcePack row should mark RPID1 as installed"
        );

        // Search for Shader packs -- RPID1 is a ResourcePack row, NOT a Shader row.
        // Even though the search returns the same hit (mock always returns RPID1),
        // the kind filter must prevent the ResourcePack row from marking it as installed.
        let hits2 = svc
            .search(
                "faithful",
                PackKind::Shader,
                None,
                Some(&paths),
                Some("inst"),
            )
            .await
            .unwrap();
        assert_eq!(hits2.len(), 1);
        assert!(
            !hits2[0].already_installed,
            "Shader search: ResourcePack row for RPID1 must NOT cause false positive"
        );
    }

    #[tokio::test]
    async fn test_list_versions_omits_loader_filter() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/project/RPID1/version")
                // Must NOT have a loaders query param
                .query_param_missing("loaders");
            then.status(200).body(
                json!([{
                    "id":"v1","project_id":"RPID1","name":"Faithful 1.0",
                    "version_number":"1.0","version_type":"release",
                    "game_versions":["1.21.4"],"loaders":[],
                    "date_published":"2026-01-01T00:00:00Z",
                    "files":[{
                        "url":"https://cdn.modrinth.com/faithful.zip",
                        "filename":"faithful-32x.zip","primary":true,
                        "size":1024,"hashes":{"sha1":"abc","sha512":"def"}
                    }]
                }])
                .to_string(),
            );
        });
        let svc = PackService::with_client(make_client(&server));
        let entries = svc
            .list_versions("RPID1", Some("1.21.4"), PackKind::Resource)
            .await
            .unwrap();
        m.assert();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_latest_stable);
    }

    /// HIGH-1 regression pin: mc=None must still emit project_type facet.
    /// Without this, ModrinthClient::search's `if mc.is_some() || !loaders.is_empty()`
    /// gate would suppress ALL facets (including project_type), causing Modrinth to
    /// return mods, modpacks, and shaders mixed into resource-pack results.
    #[tokio::test]
    async fn test_search_resource_pack_without_mc_filter_still_sends_project_type_facet() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/search")
                .query_param("query", "x")
                // facets MUST be present and contain project_type even when mc=None.
                .query_param_exists("facets")
                .is_true(|req| {
                    let facets = req
                        .query_params()
                        .iter()
                        .find(|(k, _)| k == "facets")
                        .map(|(_, v)| urlencoding::decode(v).unwrap_or_default().into_owned())
                        .unwrap_or_default();
                    facets.contains("project_type:resourcepack")
                });
            then.status(200)
                .body(json!({"hits":[],"offset":0,"limit":20,"total_hits":0}).to_string());
        });
        let svc = PackService::with_client(make_client(&server));
        // mc = None, kind = Resource -- project_type facet MUST still be sent.
        let _ = svc
            .search("x", PackKind::Resource, None, None, None)
            .await
            .unwrap();
        m.assert();
    }

    // --- install_modrinth tests ----------------------------------------------

    fn make_version(filename: &str, sha1: &str, url: &str, size: u64) -> ModrinthVersion {
        ModrinthVersion {
            id: "VER1".to_string(),
            project_id: "RPID1".to_string(),
            name: "Faithful 1.0".to_string(),
            version_number: "1.0".to_string(),
            version_type: "release".to_string(),
            game_versions: vec!["1.21.4".to_string()],
            loaders: vec![],
            downloads: 100,
            date_published: "2026-01-01T00:00:00Z".to_string(),
            dependencies: vec![],
            files: vec![crate::mods::types::ModrinthFile {
                url: url.to_string(),
                filename: filename.to_string(),
                primary: true,
                size,
                hashes: crate::mods::types::ModrinthHashes {
                    sha1: sha1.to_string(),
                    sha512: "".to_string(),
                },
            }],
        }
    }

    fn sha1_of(bytes: &[u8]) -> String {
        use sha1::Digest;
        let mut h = Sha1::new();
        h.update(bytes);
        sha1_hex(h.finalize().as_slice())
    }

    #[tokio::test]
    async fn test_install_modrinth_resource_pack_writes_to_resourcepacks_dir() {
        let server = MockServer::start();
        let body = b"fakepng-content-rp";
        let sha1 = sha1_of(body);
        let url = format!("{}/files/faithful-32x.zip", server.base_url());

        server.mock(|when, then| {
            when.method(GET).path("/files/faithful-32x.zip");
            then.status(200).body(body.as_slice());
        });

        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let svc = PackService::with_client(make_client(&server));
        let version = make_version("faithful-32x.zip", &sha1, &url, body.len() as u64);

        let (tx, _rx) = make_progress();
        let row = svc
            .install_modrinth(
                &paths,
                "test-instance",
                PackKind::Resource,
                &version,
                "faithful",
                "RPID1",
                "Faithful 32x",
                tx,
                make_token(),
                JobId(1),
            )
            .await
            .unwrap();

        // File written to resourcepacks dir.
        let expected =
            paths.instance_pack_file("test-instance", PackKind::Resource, "faithful-32x.zip");
        assert!(
            expected.exists(),
            "file should exist at {}",
            expected.display()
        );
        assert_eq!(row.kind, InstalledItemKind::ResourcePack);
        assert_eq!(row.source, ModSource::Modrinth);
        assert_eq!(row.hash_algo, HashAlgo::Sha1);
    }

    #[tokio::test]
    async fn test_install_modrinth_shader_writes_to_shaderpacks_dir() {
        let server = MockServer::start();
        let body = b"fake-shader-content";
        let sha1 = sha1_of(body);
        let url = format!("{}/files/bsl-shaders.zip", server.base_url());

        server.mock(|when, then| {
            when.method(GET).path("/files/bsl-shaders.zip");
            then.status(200).body(body.as_slice());
        });

        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let svc = PackService::with_client(make_client(&server));
        let version = make_version("bsl-shaders.zip", &sha1, &url, body.len() as u64);

        let (tx, _rx) = make_progress();
        let row = svc
            .install_modrinth(
                &paths,
                "test-instance",
                PackKind::Shader,
                &version,
                "bsl-shaders",
                "SHID1",
                "BSL Shaders",
                tx,
                make_token(),
                JobId(1),
            )
            .await
            .unwrap();

        let expected =
            paths.instance_pack_file("test-instance", PackKind::Shader, "bsl-shaders.zip");
        assert!(
            expected.exists(),
            "file should exist at {}",
            expected.display()
        );
        assert_eq!(row.kind, InstalledItemKind::Shader);
    }

    #[tokio::test]
    async fn test_install_modrinth_uses_sha1_verify() {
        let server = MockServer::start();
        let body = b"known-body";
        let correct_sha1 = sha1_of(body);
        let url = format!("{}/files/pack.zip", server.base_url());

        server.mock(|when, then| {
            when.method(GET).path("/files/pack.zip");
            then.status(200).body(body.as_slice());
        });

        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let svc = PackService::with_client(make_client(&server));

        // Use wrong sha1 to trigger mismatch.
        let version = make_version("pack.zip", "wronghash", &url, body.len() as u64);
        let (tx, _rx) = make_progress();
        let result = svc
            .install_modrinth(
                &paths,
                "inst",
                PackKind::Resource,
                &version,
                "p",
                "PID",
                "P",
                tx,
                make_token(),
                JobId(1),
            )
            .await;

        // Should fail with Sha512Mismatch (variant name preserved, Phase 9 carve-out).
        assert!(
            matches!(
                result,
                Err(PackError::Modrinth(ModrinthError::Sha512Mismatch { .. }))
            ),
            "expected Sha512Mismatch, got {result:?}"
        );

        // Verify correct sha1 succeeds and row has Sha1 algo.
        let server2 = MockServer::start();
        server2.mock(|when, then| {
            when.method(GET).path("/files/pack.zip");
            then.status(200).body(body.as_slice());
        });
        let url2 = format!("{}/files/pack.zip", server2.base_url());
        let version2 = make_version("pack.zip", &correct_sha1, &url2, body.len() as u64);
        let svc2 = PackService::with_client(make_client(&server2));
        let (tx2, _rx2) = make_progress();
        let row = svc2
            .install_modrinth(
                &paths,
                "inst2",
                PackKind::Resource,
                &version2,
                "p",
                "PID",
                "P",
                tx2,
                make_token(),
                JobId(2),
            )
            .await
            .unwrap();
        assert_eq!(row.hash_algo, HashAlgo::Sha1);
        assert_eq!(row.sha512, correct_sha1);
    }

    #[tokio::test]
    async fn test_install_modrinth_rejects_zip_filename_via_pack_validator() {
        // MockServer should receive 0 requests -- unsafe filename rejected before HTTP.
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET);
            then.status(200).body(b"body".as_slice());
        });

        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let svc = PackService::with_client(make_client(&server));
        let url = format!("{}/files/escape.zip", server.base_url());
        // Path traversal filename.
        let version = make_version("../escape.zip", "abc", &url, 100);

        let (tx, _rx) = make_progress();
        let result = svc
            .install_modrinth(
                &paths,
                "inst",
                PackKind::Resource,
                &version,
                "p",
                "PID",
                "P",
                tx,
                make_token(),
                JobId(1),
            )
            .await;

        assert!(
            matches!(result, Err(PackError::UnsafeFilename { .. })),
            "expected UnsafeFilename, got {result:?}"
        );
        // Assert mock received 0 calls.
        assert_eq!(m.calls(), 0, "HTTP request should NOT have been made");
    }

    #[tokio::test]
    async fn test_install_modrinth_creates_dest_dir_defensively() {
        // Instance dir tree is missing resourcepacks/ -- install should still succeed.
        let server = MockServer::start();
        let body = b"pack-content";
        let sha1 = sha1_of(body);
        let url = format!("{}/files/new-pack.zip", server.base_url());

        server.mock(|when, then| {
            when.method(GET).path("/files/new-pack.zip");
            then.status(200).body(body.as_slice());
        });

        let td = TempDir::new().unwrap();
        // Use a sub-path that doesn't exist yet.
        let base = td.path().join("nested").join("dirs");
        let paths = AppPaths::with_roots(base.clone(), base.clone(), base);
        let svc = PackService::with_client(make_client(&server));
        let version = make_version("new-pack.zip", &sha1, &url, body.len() as u64);

        let (tx, _rx) = make_progress();
        let result = svc
            .install_modrinth(
                &paths,
                "inst",
                PackKind::Resource,
                &version,
                "p",
                "PID",
                "P",
                tx,
                make_token(),
                JobId(1),
            )
            .await;
        assert!(
            result.is_ok(),
            "should succeed despite missing dir: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_install_modrinth_atomicity_upsert_before_rename() {
        // Simulate rename failure by pre-creating a directory at the final path.
        // After the failed install, the ledger row MUST still be present (upsert
        // happened before rename -- Pitfall 8).
        let server = MockServer::start();
        let body = b"pack-data";
        let sha1 = sha1_of(body);
        let url = format!("{}/files/conflict.zip", server.base_url());

        server.mock(|when, then| {
            when.method(GET).path("/files/conflict.zip");
            then.status(200).body(body.as_slice());
        });

        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);

        // Pre-create the target final path as a DIRECTORY to make rename fail.
        let final_path = paths.instance_pack_file("inst", PackKind::Resource, "conflict.zip");
        tokio::fs::create_dir_all(&final_path).await.unwrap();

        let svc = PackService::with_client(make_client(&server));
        let version = make_version("conflict.zip", &sha1, &url, body.len() as u64);

        let (tx, _rx) = make_progress();
        let result = svc
            .install_modrinth(
                &paths,
                "inst",
                PackKind::Resource,
                &version,
                "conflict-pack",
                "CONFLICT-PID",
                "Conflict Pack",
                tx,
                make_token(),
                JobId(1),
            )
            .await;

        // Install should fail due to rename conflict.
        assert!(result.is_err(), "expected error due to rename conflict");

        // But ledger row MUST be present (upsert happened before rename).
        // project_id passed is "CONFLICT-PID" -- that becomes row.mod_id.
        let ledger = crate::mods::ledger::read_ledger(&paths, "inst")
            .await
            .unwrap();
        assert!(
            ledger.mods.iter().any(|r| r.mod_id == "CONFLICT-PID"),
            "ledger row must be present even after rename failure (Pitfall 8)"
        );

        // .tmp file should have been cleaned up.
        let tmp_path = {
            let p = final_path.clone();
            // final_path is a dir here, so look for .tmp sibling
            let parent = p.parent().unwrap().to_path_buf();
            parent.join("conflict.zip.tmp")
        };
        assert!(
            !tmp_path.exists(),
            ".tmp file should be cleaned up after rename failure"
        );
    }

    // --- list_installed tests ------------------------------------------------

    #[tokio::test]
    async fn test_list_installed_filters_by_kind() {
        let server = MockServer::start();
        let svc = PackService::with_client(make_client(&server));
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);

        // Insert 1 mod + 1 resource pack + 1 shader.
        crate::mods::ledger::upsert_mod(
            &paths,
            "inst",
            InstalledModRow {
                mod_id: "MOD1".to_string(),
                project_slug: "mod1".to_string(),
                display_name: "Mod One".to_string(),
                version_id: "v1".to_string(),
                version_label: "1.0".to_string(),
                file_name: "mod1.jar".to_string(),
                sha512: "x".to_string(),
                size: 1,
                hash_algo: HashAlgo::Sha512,
                kind: InstalledItemKind::Mod,
                source: ModSource::Modrinth,
                enabled: true,
                installed_at: "now".to_string(),
            },
        )
        .await
        .unwrap();
        upsert_pack(
            &paths,
            "inst",
            pack_row("RP1", "rp1.zip", InstalledItemKind::ResourcePack, true),
        )
        .await
        .unwrap();
        upsert_pack(
            &paths,
            "inst",
            pack_row("SH1", "sh1.zip", InstalledItemKind::Shader, true),
        )
        .await
        .unwrap();

        let rp_rows = svc
            .list_installed(&paths, "inst", PackKind::Resource)
            .await
            .unwrap();
        assert_eq!(rp_rows.len(), 1, "should return only resource pack rows");
        assert_eq!(rp_rows[0].mod_id, "RP1");

        let sh_rows = svc
            .list_installed(&paths, "inst", PackKind::Shader)
            .await
            .unwrap();
        assert_eq!(sh_rows.len(), 1, "should return only shader rows");
        assert_eq!(sh_rows[0].mod_id, "SH1");
    }

    // --- toggle_pack_enabled tests -------------------------------------------

    #[tokio::test]
    async fn test_toggle_pack_enabled_resource_renames_in_resourcepacks_dir() {
        let server = MockServer::start();
        let svc = PackService::with_client(make_client(&server));
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);

        // Create the file in resourcepacks dir.
        let packs_dir = paths.instance_packs_dir("inst", PackKind::Resource);
        tokio::fs::create_dir_all(&packs_dir).await.unwrap();
        let file_path = packs_dir.join("rp1.zip");
        tokio::fs::write(&file_path, b"content").await.unwrap();

        // Insert ledger row.
        upsert_pack(
            &paths,
            "inst",
            pack_row("RP1", "rp1.zip", InstalledItemKind::ResourcePack, true),
        )
        .await
        .unwrap();

        // Toggle to disabled.
        let new_state = svc
            .toggle_pack_enabled(&paths, "inst", "RP1", PackKind::Resource)
            .await
            .unwrap();
        assert!(!new_state, "should be disabled after toggle");

        // File should now be rp1.zip.disabled in resourcepacks dir (NOT mods/).
        assert!(!file_path.exists(), "original file should be gone");
        assert!(
            packs_dir.join("rp1.zip.disabled").exists(),
            "disabled file should exist in resourcepacks dir"
        );

        // Toggle back to enabled.
        let new_state2 = svc
            .toggle_pack_enabled(&paths, "inst", "RP1", PackKind::Resource)
            .await
            .unwrap();
        assert!(new_state2, "should be enabled after second toggle");
        assert!(file_path.exists(), "file should be back to rp1.zip");
    }

    #[tokio::test]
    async fn test_toggle_pack_enabled_shader_returns_error() {
        let server = MockServer::start();
        let svc = PackService::with_client(make_client(&server));
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);

        let result = svc
            .toggle_pack_enabled(&paths, "inst", "SH1", PackKind::Shader)
            .await;
        assert!(
            matches!(result, Err(PackError::ShaderToggleNotSupported { .. })),
            "expected ShaderToggleNotSupported, got {result:?}"
        );
    }

    // --- uninstall_pack tests ------------------------------------------------

    #[tokio::test]
    async fn test_uninstall_pack_removes_file_and_drops_row() {
        let server = MockServer::start();
        let svc = PackService::with_client(make_client(&server));
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);

        // Create file + ledger row.
        let packs_dir = paths.instance_packs_dir("inst", PackKind::Resource);
        tokio::fs::create_dir_all(&packs_dir).await.unwrap();
        tokio::fs::write(packs_dir.join("pack.zip"), b"data")
            .await
            .unwrap();

        upsert_pack(
            &paths,
            "inst",
            pack_row("RP1", "pack.zip", InstalledItemKind::ResourcePack, true),
        )
        .await
        .unwrap();

        svc.uninstall_pack(&paths, "inst", "RP1", PackKind::Resource)
            .await
            .unwrap();

        assert!(
            !packs_dir.join("pack.zip").exists(),
            "file should be removed"
        );
        let ledger = crate::mods::ledger::read_ledger(&paths, "inst")
            .await
            .unwrap();
        assert!(ledger.mods.is_empty(), "ledger row should be dropped");
    }

    #[tokio::test]
    async fn test_uninstall_pack_handles_missing_file_defensively() {
        let server = MockServer::start();
        let svc = PackService::with_client(make_client(&server));
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);

        // Only ledger row, no file on disk.
        upsert_pack(
            &paths,
            "inst",
            pack_row("RP1", "pack.zip", InstalledItemKind::ResourcePack, true),
        )
        .await
        .unwrap();

        // Should succeed (NotFound-tolerant) and drop ledger row.
        svc.uninstall_pack(&paths, "inst", "RP1", PackKind::Resource)
            .await
            .unwrap();

        let ledger = crate::mods::ledger::read_ledger(&paths, "inst")
            .await
            .unwrap();
        assert!(
            ledger.mods.is_empty(),
            "ledger row should be dropped even if file missing"
        );
    }
}
