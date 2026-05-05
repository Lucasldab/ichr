//! Modrinth service facade -- composes the HTTP client, dep resolver, ledger,
//! and parallel installer into the surface the TUI consumes.
//!
//! Mirrors `src/loader/service.rs` (LoaderService) field-by-field. Will be held
//! as `Arc<ModrinthService>` in `src/tui/run.rs` exactly like
//! `Arc<LoaderService>` once 08-08 lands.
//!
//! Every public async method carries `#[tracing::instrument(skip_all, fields(...))]`.
//! Construction (`new`) is also instrumented.

use std::collections::HashMap;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::loader::types::LoaderInfo;
use crate::mods::dep_resolve::{resolve_required_deps, ResolvedDepGraph};
use crate::mods::error::ModrinthError;
use crate::mods::filter::{modrinth_filter_for, pick_latest_by_date};
use crate::mods::installer::{
    build_install_plan, install_mods_into_instance, MOD_DOWNLOAD_CONCURRENCY,
};
use crate::mods::ledger::{read_ledger, toggle_enabled, uninstall as ledger_uninstall};
use crate::mods::modrinth::client::SEARCH_DEFAULT_LIMIT;
use crate::mods::modrinth::ModrinthClient;
use crate::mods::types::{
    InstalledModRow, ModrinthProjectDetail, ModrinthSearchHit, ModrinthVersion,
    ModrinthVersionEntry,
};
use crate::persistence::paths::AppPaths;
use crate::tasks::{JobId, TaskEvent};

/// Re-export so consumers (08-08 run.rs) can reference one canonical const.
pub use crate::mods::installer::MOD_DOWNLOAD_CONCURRENCY as PARALLEL_DOWNLOAD_CAP;
const _: usize = MOD_DOWNLOAD_CONCURRENCY; // tie alias to const so renames cascade

#[derive(Debug)]
pub struct ModrinthService {
    client: ModrinthClient,
}

impl ModrinthService {
    #[tracing::instrument(skip_all)]
    pub fn new() -> Result<Self, ModrinthError> {
        Ok(Self {
            client: ModrinthClient::new()?,
        })
    }

    #[cfg(test)]
    pub fn with_client(client: ModrinthClient) -> Self {
        Self { client }
    }

    // ====================================================================
    // === Read-only browse                                              ===
    // ====================================================================

    /// Search Modrinth with optional MC version and loader filter.
    /// Stamps `already_installed` against the per-instance ledger before returning.
    #[tracing::instrument(
        skip_all,
        fields(query = %query, mc = ?mc, slug = ?slug)
    )]
    pub async fn search(
        &self,
        query: &str,
        mc: Option<&str>,
        loader: Option<&LoaderInfo>,
        paths: Option<&AppPaths>,
        slug: Option<&str>,
    ) -> Result<Vec<ModrinthSearchHit>, ModrinthError> {
        // Compute Modrinth loaders from the instance loader (Pitfall 2 expansion lives in filter.rs).
        let (loaders_strs, _gv) = match (loader, mc) {
            (Some(l), Some(m)) => modrinth_filter_for(Some(l), m),
            _ => (vec![], vec![]),
        };
        let mut hits = self
            .client
            .search(query, mc, &loaders_strs, SEARCH_DEFAULT_LIMIT)
            .await?;

        // Stamp already_installed from the ledger (best-effort -- if read fails, leave false).
        if let (Some(p), Some(s)) = (paths, slug) {
            if let Ok(led) = read_ledger(p, s).await {
                let installed: std::collections::HashSet<&str> =
                    led.mods.iter().map(|m| m.mod_id.as_str()).collect();
                for h in &mut hits {
                    if installed.contains(h.project_id.as_str()) {
                        h.already_installed = true;
                    }
                }
            }
        }
        Ok(hits)
    }

    #[tracing::instrument(skip_all, fields(id_or_slug = %id_or_slug))]
    pub async fn get_project(
        &self,
        id_or_slug: &str,
    ) -> Result<ModrinthProjectDetail, ModrinthError> {
        self.client.get_project(id_or_slug).await
    }

    /// List versions filtered by MC + loader, returning UI-friendly
    /// `ModrinthVersionEntry`s.
    ///
    /// Sorted by `date_published` descending; the first `release`-channel entry
    /// is marked `is_latest_stable`.
    #[tracing::instrument(skip_all, fields(project_id = %project_id, mc = ?mc))]
    pub async fn list_versions(
        &self,
        project_id: &str,
        mc: Option<&str>,
        loader: Option<&LoaderInfo>,
    ) -> Result<Vec<ModrinthVersionEntry>, ModrinthError> {
        let (loaders_strs, _) = match (loader, mc) {
            (Some(l), Some(m)) => modrinth_filter_for(Some(l), m),
            _ => (vec![], vec![]),
        };
        let mut versions = self
            .client
            .list_versions(project_id, mc, &loaders_strs)
            .await?;
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

    /// Resolve the required dep graph for a chosen version.
    ///
    /// Builds the two fetch closures from the underlying client: latest-by-date
    /// + version-from-id (Q2 path).
    #[tracing::instrument(
        skip_all,
        fields(slug = %slug, root_version_id = %root_version_id)
    )]
    pub async fn resolve_dependencies(
        &self,
        paths: &AppPaths,
        slug: &str,
        root_version_id: &str,
        mc: &str,
        loader: Option<&LoaderInfo>,
    ) -> Result<ResolvedDepGraph, ModrinthError> {
        let root = self.client.get_version(root_version_id).await?;
        let (loaders_vec, _) = modrinth_filter_for(loader, mc);
        let loaders_strings: Vec<String> =
            loaders_vec.iter().map(|s| (*s).to_string()).collect();

        let installed: HashMap<String, String> = read_ledger(paths, slug)
            .await?
            .mods
            .into_iter()
            .map(|m| (m.mod_id, m.version_id))
            .collect();

        // Closures must be `Fn + Sync` (per resolve_required_deps signature) so
        // we capture by clone and re-clone inside each invocation. ModrinthClient
        // is `Clone + Send + Sync` (its only field is reqwest::Client).
        let client_for_latest = self.client.clone();
        let fetch_latest = move |project_id: String,
                                 mc_q: String,
                                 loaders_q: Vec<String>| {
            let client = client_for_latest.clone();
            async move {
                let loaders_refs: Vec<&str> =
                    loaders_q.iter().map(|s| s.as_str()).collect();
                let versions = client
                    .list_versions(&project_id, Some(&mc_q), &loaders_refs)
                    .await?;
                Ok(pick_latest_by_date(&versions).cloned())
            }
        };
        let client_for_byid = self.client.clone();
        let fetch_by_id = move |version_id: String| {
            let client = client_for_byid.clone();
            async move { client.get_version(&version_id).await }
        };

        resolve_required_deps(
            root,
            mc,
            &loaders_strings,
            &installed,
            fetch_latest,
            fetch_by_id,
        )
        .await
    }

    /// Install the root version + its required-and-not-already-satisfied deps.
    ///
    /// Progress flows through `progress_tx` as `TaskEvent::Progress` events;
    /// 08-08 wires this into the existing `download_pane` (NOT a blocking modal --
    /// UI-SPEC §11).
    ///
    /// `now_iso8601_utc` is sourced from `crate::domain::instance` (the same helper
    /// that stamps `InstanceManifest.created_at`). No new time-formatting code path.
    #[tracing::instrument(
        skip_all,
        fields(slug = %slug, root_project = %root_project_slug, job_id = job_id.0)
    )]
    #[allow(clippy::too_many_arguments)]
    pub async fn install_mod_into_instance(
        &self,
        paths: &AppPaths,
        slug: &str,
        root_project_slug: &str,
        root_project_title: &str,
        root_version: &ModrinthVersion,
        graph: &ResolvedDepGraph,
        progress_tx: mpsc::Sender<TaskEvent>,
        token: CancellationToken,
        job_id: JobId,
    ) -> Result<(), ModrinthError> {
        let plan = build_install_plan(
            root_version,
            root_project_slug,
            root_project_title,
            &graph.deps,
            crate::domain::instance::now_iso8601_utc,
        )?;
        install_mods_into_instance(
            self.client.http().clone(),
            paths.clone(),
            slug.to_string(),
            plan,
            progress_tx,
            token,
            job_id,
        )
        .await
    }

    // ====================================================================
    // === Ledger ops                                                    ===
    // ====================================================================

    #[tracing::instrument(skip_all, fields(slug = %slug))]
    pub async fn list_installed_mods(
        &self,
        paths: &AppPaths,
        slug: &str,
    ) -> Result<Vec<InstalledModRow>, ModrinthError> {
        Ok(read_ledger(paths, slug).await?.mods)
    }

    #[tracing::instrument(skip_all, fields(slug = %slug, mod_id))]
    pub async fn enable_mod(
        &self,
        paths: &AppPaths,
        slug: &str,
        mod_id: &str,
    ) -> Result<(), ModrinthError> {
        let l = read_ledger(paths, slug).await?;
        let row = l
            .mods
            .iter()
            .find(|m| m.mod_id == mod_id)
            .ok_or_else(|| ModrinthError::ModNotFound(mod_id.to_string()))?;
        if row.enabled {
            return Ok(());
        }
        let _ = toggle_enabled(paths, slug, mod_id).await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(slug = %slug, mod_id))]
    pub async fn disable_mod(
        &self,
        paths: &AppPaths,
        slug: &str,
        mod_id: &str,
    ) -> Result<(), ModrinthError> {
        let l = read_ledger(paths, slug).await?;
        let row = l
            .mods
            .iter()
            .find(|m| m.mod_id == mod_id)
            .ok_or_else(|| ModrinthError::ModNotFound(mod_id.to_string()))?;
        if !row.enabled {
            return Ok(());
        }
        let _ = toggle_enabled(paths, slug, mod_id).await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(slug = %slug, mod_id))]
    pub async fn uninstall_mod(
        &self,
        paths: &AppPaths,
        slug: &str,
        mod_id: &str,
    ) -> Result<(), ModrinthError> {
        ledger_uninstall(paths, slug, mod_id).await
    }
}

// ============================================================================
// === Tests                                                                ===
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::instance::ModloaderKind;
    use crate::mods::types::{InstalledModRow, ModSource};
    use httpmock::prelude::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_client(server: &MockServer) -> ModrinthClient {
        ModrinthClient::new_with_base_url(server.base_url())
            .expect("client::new_with_base_url")
    }

    fn test_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    fn fabric_loader() -> LoaderInfo {
        LoaderInfo {
            kind: ModloaderKind::Fabric,
            version: "0.16.9".into(),
            version_id: "fabric-loader-0.16.9-1.20.4".into(),
        }
    }

    // --- search --------------------------------------------------------------

    #[tokio::test]
    async fn test_search_passes_through_to_client_with_loader_expansion() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/search")
                .query_param("query", "sodium")
                .query_param_exists("facets");
            then.status(200).body(
                json!({
                    "hits": [
                        {"project_id":"AANobbMI","slug":"sodium","title":"Sodium",
                         "description":"Modern rendering","downloads":12345}
                    ],
                    "offset":0,"limit":20,"total_hits":1
                })
                .to_string(),
            );
        });
        let svc = ModrinthService::with_client(make_client(&server));
        let loader = fabric_loader();
        let hits = svc
            .search("sodium", Some("1.20.4"), Some(&loader), None, None)
            .await
            .unwrap();
        m.assert();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "sodium");
        assert!(!hits[0].already_installed, "no ledger context provided");
    }

    #[tokio::test]
    async fn test_search_stamps_already_installed_from_ledger() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/search");
            then.status(200).body(
                json!({
                    "hits": [
                        {"project_id":"AANobbMI","slug":"sodium","title":"Sodium",
                         "description":"x","downloads":1}
                    ]
                })
                .to_string(),
            );
        });
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        // Pre-populate ledger with the searched project.
        crate::mods::ledger::upsert_mod(
            &paths,
            "inst",
            InstalledModRow {
                mod_id: "AANobbMI".into(),
                project_slug: "sodium".into(),
                display_name: "Sodium".into(),
                version_id: "v".into(),
                version_label: "0.5".into(),
                file_name: "sodium.jar".into(),
                sha512: "x".into(),
                size: 1,
                source: ModSource::Modrinth,
                enabled: true,
                installed_at: "now".into(),
            },
        )
        .await
        .unwrap();

        let svc = ModrinthService::with_client(make_client(&server));
        let hits = svc
            .search("sodium", None, None, Some(&paths), Some("inst"))
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].already_installed, "ledger contains AANobbMI");
    }

    #[tokio::test]
    async fn test_get_project_delegates_to_client() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/project/sodium");
            then.status(200).body(
                json!({
                    "id":"AANobbMI","title":"Sodium","description":"x","downloads":1,
                    "license":{"id":"LGPL-3.0-only"},"categories":["library"]
                })
                .to_string(),
            );
        });
        let svc = ModrinthService::with_client(make_client(&server));
        let p = svc.get_project("sodium").await.unwrap();
        assert_eq!(p.title, "Sodium");
        assert_eq!(p.license_id, "LGPL-3.0-only");
    }

    // --- list_versions: sort + is_latest_stable -----------------------------

    #[tokio::test]
    async fn test_list_versions_sorts_descending_and_marks_first_release_stable() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/project/AANobbMI/version");
            then.status(200).body(
                json!([
                    // older release
                    {
                        "id":"old","project_id":"AANobbMI","name":"Sodium 0.4",
                        "version_number":"0.4","version_type":"release",
                        "game_versions":["1.20.4"],"loaders":["fabric"],
                        "date_published":"2024-01-01T00:00:00Z",
                        "files":[{"url":"https://cdn.modrinth.com/x.jar","filename":"x.jar","primary":true,
                                  "size":1,"hashes":{"sha1":"a","sha512":"b"}}]
                    },
                    // newer beta
                    {
                        "id":"beta","project_id":"AANobbMI","name":"Sodium 0.6-beta",
                        "version_number":"0.6-beta","version_type":"beta",
                        "game_versions":["1.20.4"],"loaders":["fabric"],
                        "date_published":"2026-03-01T00:00:00Z",
                        "files":[{"url":"https://cdn.modrinth.com/x.jar","filename":"x.jar","primary":true,
                                  "size":1,"hashes":{"sha1":"a","sha512":"b"}}]
                    },
                    // newest release
                    {
                        "id":"new","project_id":"AANobbMI","name":"Sodium 0.5",
                        "version_number":"0.5","version_type":"release",
                        "game_versions":["1.20.4"],"loaders":["fabric"],
                        "date_published":"2026-04-01T00:00:00Z",
                        "files":[{"url":"https://cdn.modrinth.com/x.jar","filename":"x.jar","primary":true,
                                  "size":1,"hashes":{"sha1":"a","sha512":"b"}}]
                    }
                ])
                .to_string(),
            );
        });
        let svc = ModrinthService::with_client(make_client(&server));
        let entries = svc.list_versions("AANobbMI", None, None).await.unwrap();
        assert_eq!(entries.len(), 3);
        // descending by date: new -> beta -> old
        assert_eq!(entries[0].version_id, "new");
        assert_eq!(entries[1].version_id, "beta");
        assert_eq!(entries[2].version_id, "old");
        // is_latest_stable is on `new` (first release-channel entry).
        assert!(entries[0].is_latest_stable);
        assert!(!entries[1].is_latest_stable, "beta is not stable");
        assert!(
            !entries[2].is_latest_stable,
            "older release: only the FIRST release-channel entry is marked"
        );
    }

    // --- ledger ops via service --------------------------------------------

    #[tokio::test]
    async fn test_list_installed_mods_returns_ledger_rows() {
        let server = MockServer::start();
        let svc = ModrinthService::with_client(make_client(&server));
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        crate::mods::ledger::upsert_mod(
            &paths,
            "i",
            InstalledModRow {
                mod_id: "M1".into(),
                project_slug: "p".into(),
                display_name: "P".into(),
                version_id: "v".into(),
                version_label: "0".into(),
                file_name: "p.jar".into(),
                sha512: "x".into(),
                size: 1,
                source: ModSource::Modrinth,
                enabled: true,
                installed_at: "now".into(),
            },
        )
        .await
        .unwrap();
        let rows = svc.list_installed_mods(&paths, "i").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].mod_id, "M1");
    }

    #[tokio::test]
    async fn test_enable_mod_noop_when_already_enabled() {
        let server = MockServer::start();
        let svc = ModrinthService::with_client(make_client(&server));
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        // Mod present + enabled. enable_mod must NOT call toggle_enabled (which
        // would try to rename a non-existent file and fail).
        crate::mods::ledger::upsert_mod(
            &paths,
            "i",
            InstalledModRow {
                mod_id: "M1".into(),
                project_slug: "p".into(),
                display_name: "P".into(),
                version_id: "v".into(),
                version_label: "0".into(),
                file_name: "p.jar".into(),
                sha512: "x".into(),
                size: 1,
                source: ModSource::Modrinth,
                enabled: true,
                installed_at: "now".into(),
            },
        )
        .await
        .unwrap();
        // No file on disk -- if enable_mod did call toggle_enabled it would Err.
        svc.enable_mod(&paths, "i", "M1").await.unwrap();
        let l = crate::mods::ledger::read_ledger(&paths, "i").await.unwrap();
        assert!(l.mods[0].enabled, "still enabled, no flip");
    }

    #[tokio::test]
    async fn test_disable_mod_noop_when_already_disabled() {
        let server = MockServer::start();
        let svc = ModrinthService::with_client(make_client(&server));
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        crate::mods::ledger::upsert_mod(
            &paths,
            "i",
            InstalledModRow {
                mod_id: "M1".into(),
                project_slug: "p".into(),
                display_name: "P".into(),
                version_id: "v".into(),
                version_label: "0".into(),
                file_name: "p.jar".into(),
                sha512: "x".into(),
                size: 1,
                source: ModSource::Modrinth,
                enabled: false,
                installed_at: "now".into(),
            },
        )
        .await
        .unwrap();
        svc.disable_mod(&paths, "i", "M1").await.unwrap();
        let l = crate::mods::ledger::read_ledger(&paths, "i").await.unwrap();
        assert!(!l.mods[0].enabled, "still disabled, no flip");
    }

    #[tokio::test]
    async fn test_enable_mod_unknown_returns_mod_not_found() {
        let server = MockServer::start();
        let svc = ModrinthService::with_client(make_client(&server));
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let r = svc.enable_mod(&paths, "i", "ghost").await;
        assert!(matches!(r, Err(ModrinthError::ModNotFound(_))), "got {r:?}");
    }

    #[tokio::test]
    async fn test_uninstall_mod_delegates_to_ledger() {
        let server = MockServer::start();
        let svc = ModrinthService::with_client(make_client(&server));
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        crate::mods::ledger::upsert_mod(
            &paths,
            "i",
            InstalledModRow {
                mod_id: "M1".into(),
                project_slug: "p".into(),
                display_name: "P".into(),
                version_id: "v".into(),
                version_label: "0".into(),
                file_name: "p.jar".into(),
                sha512: "x".into(),
                size: 1,
                source: ModSource::Modrinth,
                enabled: true,
                installed_at: "now".into(),
            },
        )
        .await
        .unwrap();
        svc.uninstall_mod(&paths, "i", "M1").await.unwrap();
        let l = crate::mods::ledger::read_ledger(&paths, "i").await.unwrap();
        assert!(l.mods.is_empty());
    }

    // --- export sanity ------------------------------------------------------

    #[test]
    fn test_parallel_download_cap_alias_matches_const() {
        assert_eq!(PARALLEL_DOWNLOAD_CAP, MOD_DOWNLOAD_CONCURRENCY);
        assert_eq!(PARALLEL_DOWNLOAD_CAP, 6);
    }
}
