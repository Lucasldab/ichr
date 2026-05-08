//! Version install orchestrator -- stub; full implementation in Task 2-06-02.

use std::collections::HashMap;
use std::sync::Arc;

use futures::future::try_join_all;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::domain::platform::{Arch, OsName};
use crate::error::AppError;
use crate::mojang::client::{MojangClient, ASSET_CDN_BASE};
use crate::mojang::inherits::{resolve_inherits, MAX_INHERITS_DEPTH};
use crate::mojang::natives::{native_classifier_artifact, needs_native_extraction};
use crate::mojang::rules::{evaluate_rules, RuleContext};
use crate::mojang::types::{AssetIndexFile, Library, ResolvedVersion, VersionEntry, VersionJson};
use crate::persistence::paths::AppPaths;
use crate::tasks::job::{JobId, TaskEvent};

use super::natives_extract::extract_native_jar;

/// Concurrent library downloads within a single install job.
pub const LIB_CONCURRENCY: usize = 8;

/// Concurrent asset object downloads.
pub const ASSET_CONCURRENCY: usize = 16;

/// Run the full vanilla install for `version_entry` into the shared data trees
/// and per-instance natives dir.
pub async fn install_version(
    job_id: JobId,
    paths: &AppPaths,
    mojang: &MojangClient,
    progress_tx: mpsc::Sender<TaskEvent>,
    token: CancellationToken,
    slug: &str,
    version_entry: &VersionEntry,
) -> Result<(), AppError> {
    let ctx = RuleContext::for_os_arch(OsName::current(), Arch::current());

    // ---- Step 1: version JSON ----
    send_progress(&progress_tx, job_id, 1, "fetching version JSON").await;
    let version_json_path = paths.version_json(&version_entry.id);
    let raw = mojang
        .fetch_version_json(&version_entry.url, &version_entry.sha1, &version_json_path)
        .await?;
    if token.is_cancelled() {
        return Err(AppError::Cancelled);
    }

    // ---- Step 2 + 3: inheritsFrom chain resolution ----
    let parents = collect_inherits_chain(paths, mojang, &raw).await?;
    let version = resolve_inherits(&raw, &parents)?;
    if token.is_cancelled() {
        return Err(AppError::Cancelled);
    }

    // ---- Step 4: client.jar ----
    send_progress(&progress_tx, job_id, 10, "downloading client.jar").await;
    let client_dl = version
        .downloads
        .client
        .as_ref()
        .ok_or_else(|| AppError::Http("version JSON missing downloads.client".into()))?;
    mojang
        .download_verified(
            &client_dl.url,
            &paths.version_jar(&version.id),
            &client_dl.sha1,
        )
        .await?;
    if token.is_cancelled() {
        return Err(AppError::Cancelled);
    }

    // ---- Step 5: libraries ----
    let selected = selected_libraries(&version, &ctx);
    send_progress(
        &progress_tx,
        job_id,
        20,
        &format!("downloading {} libraries", selected.len()),
    )
    .await;
    let lib_sem = Arc::new(Semaphore::new(LIB_CONCURRENCY));
    download_libraries(mojang, paths, &selected, OsName::current(), lib_sem).await?;
    if token.is_cancelled() {
        return Err(AppError::Cancelled);
    }

    // ---- Step 6: asset index ----
    send_progress(&progress_tx, job_id, 60, "fetching asset index").await;
    let idx_file = mojang
        .fetch_asset_index(
            &version.asset_index.url,
            &version.asset_index.sha1,
            &paths.asset_index(&version.asset_index.id),
        )
        .await?;
    if token.is_cancelled() {
        return Err(AppError::Cancelled);
    }

    // ---- Step 7: asset objects (DISTINCT semaphore from libraries) ----
    send_progress(
        &progress_tx,
        job_id,
        70,
        &format!("downloading {} asset objects", idx_file.objects.len()),
    )
    .await;
    let asset_sem = Arc::new(Semaphore::new(ASSET_CONCURRENCY));
    download_assets(mojang, paths, &idx_file, asset_sem).await?;
    if token.is_cancelled() {
        return Err(AppError::Cancelled);
    }

    // ---- Step 8: natives extraction (legacy only) ----
    send_progress(&progress_tx, job_id, 90, "extracting natives").await;
    extract_natives_for_libraries(paths, &selected, OsName::current(), slug).await?;
    if token.is_cancelled() {
        return Err(AppError::Cancelled);
    }

    // ---- Step 9: virtual asset copy (pre-1.7.2 only) ----
    if idx_file.virtual_ == Some(true) {
        send_progress(&progress_tx, job_id, 95, "materializing virtual assets").await;
        copy_virtual_assets(paths, &idx_file, &version.asset_index.id).await?;
    }

    send_progress(&progress_tx, job_id, 100, "install complete").await;
    Ok(())
}

/// Return the subset of `version.libraries` that pass rule evaluation for `ctx`.
/// Pub for testability.
pub fn selected_libraries<'a>(version: &'a ResolvedVersion, ctx: &RuleContext) -> Vec<&'a Library> {
    version
        .libraries
        .iter()
        .filter(|lib| evaluate_rules(&lib.rules, ctx))
        .collect()
}

/// Walk the inheritsFrom chain starting at `child` and fetch every parent's
/// VersionJson via `MojangClient::fetch_version_json`. Returns a HashMap
/// keyed by parent id. Returns an empty map when `child.inherits_from` is None.
pub(crate) async fn collect_inherits_chain(
    paths: &AppPaths,
    mojang: &MojangClient,
    child: &VersionJson,
) -> Result<HashMap<String, VersionJson>, AppError> {
    let mut parents: HashMap<String, VersionJson> = HashMap::new();
    let Some(first) = child.inherits_from.clone() else {
        return Ok(parents);
    };

    // Load the manifest so we can resolve parent URLs by id.
    let manifest = mojang
        .fetch_manifest(&paths.cache_dir.join("manifest_v2.json"))
        .await?;

    let mut next_id = Some(first);
    let mut depth: u32 = 0;
    while let Some(id) = next_id.take() {
        if depth >= MAX_INHERITS_DEPTH {
            return Err(AppError::InheritsFromDepthExceeded {
                current: id,
                max: MAX_INHERITS_DEPTH,
            });
        }
        if parents.contains_key(&id) {
            return Err(AppError::InheritsFromCycle(id));
        }
        let entry = manifest
            .versions
            .iter()
            .find(|v| v.id == id)
            .ok_or_else(|| AppError::InheritsFromParentMissing(id.clone()))?;
        let parent = mojang
            .fetch_version_json(&entry.url, &entry.sha1, &paths.version_json(&entry.id))
            .await?;
        next_id = parent.inherits_from.clone();
        parents.insert(id, parent);
        depth += 1;
    }
    Ok(parents)
}

async fn download_libraries(
    mojang: &MojangClient,
    paths: &AppPaths,
    libs: &[&Library],
    os: OsName,
    sem: Arc<Semaphore>,
) -> Result<(), AppError> {
    let mut futs = Vec::new();
    for lib in libs {
        // Main artifact (if present).
        if let Some(art) = lib.downloads.artifact.as_ref() {
            let url = art.url.clone();
            let dest = paths.library_path(&art.path);
            let client = mojang.clone();
            let sem = Arc::clone(&sem);
            let coord = lib.name.clone();
            let sha1_opt = art.sha1.clone();
            futs.push(tokio::spawn(async move {
                let _permit = sem.acquire_owned().await.map_err(|_| AppError::Cancelled)?;
                match sha1_opt.as_deref() {
                    Some(sha1) => client.download_verified(&url, &dest, sha1).await,
                    None => {
                        // Phase 8.4 GAP-LIBRARY-SHAPE-08: Quilt loader libraries
                        // have no upstream sha1 (Quilt-meta API limitation). We
                        // download without checksum verification and log the
                        // trade-off; this is a deliberate compromise documented
                        // in 8.4-01-PLAN.md.
                        tracing::info!(
                            coord = %coord,
                            "library downloaded without sha1 verification (Quilt loader API has no checksums)"
                        );
                        client.download_unverified(&url, &dest).await
                    }
                }
            }));
        }
        // Classifier artifact for legacy natives.
        if needs_native_extraction(lib) {
            if let Some(cl) = native_classifier_artifact(lib, os) {
                let url = cl.url.clone();
                let dest = paths.library_path(&cl.path);
                let client = mojang.clone();
                let sem = Arc::clone(&sem);
                let coord = lib.name.clone();
                let sha1_opt = cl.sha1.clone();
                futs.push(tokio::spawn(async move {
                    let _permit =
                        sem.acquire_owned().await.map_err(|_| AppError::Cancelled)?;
                    match sha1_opt.as_deref() {
                        Some(sha1) => client.download_verified(&url, &dest, sha1).await,
                        None => {
                            tracing::info!(
                                coord = %coord,
                                "classifier downloaded without sha1 verification (Phase 8.4 Quilt-loader path)"
                            );
                            client.download_unverified(&url, &dest).await
                        }
                    }
                }));
            }
        }
    }
    for h in try_join_all(futs)
        .await
        .map_err(|e| AppError::Http(format!("join: {e}")))?
    {
        h?;
    }
    Ok(())
}

async fn download_assets(
    mojang: &MojangClient,
    paths: &AppPaths,
    idx: &AssetIndexFile,
    sem: Arc<Semaphore>,
) -> Result<(), AppError> {
    let mut futs = Vec::new();
    for obj in idx.objects.values() {
        // Validate hash shape before using as path component (security).
        if obj.hash.len() != 40 || !obj.hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(AppError::Sha1Mismatch {
                target: format!("asset {}", obj.hash),
                expected: "40-char lowercase hex".into(),
                got: obj.hash.clone(),
            });
        }
        let hash = obj.hash.clone();
        let url = format!("{ASSET_CDN_BASE}/{}/{}", &hash[..2], hash);
        let dest = paths.asset_object(&hash);
        let client = mojang.clone();
        let sem = Arc::clone(&sem);
        futs.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.map_err(|_| AppError::Cancelled)?;
            client.download_verified(&url, &dest, &hash).await
        }));
    }
    for h in try_join_all(futs)
        .await
        .map_err(|e| AppError::Http(format!("join: {e}")))?
    {
        h?;
    }
    Ok(())
}

async fn extract_natives_for_libraries(
    paths: &AppPaths,
    libs: &[&Library],
    os: OsName,
    slug: &str,
) -> Result<(), AppError> {
    let natives_dir = paths.instance_natives_dir(slug);
    for lib in libs {
        if !needs_native_extraction(lib) {
            continue;
        }
        let Some(cl) = native_classifier_artifact(lib, os) else {
            continue;
        };
        let jar_path = paths.library_path(&cl.path);
        let exclude: Vec<String> = lib
            .extract
            .as_ref()
            .map(|e| e.exclude.clone())
            .unwrap_or_default();
        extract_native_jar(&jar_path, &natives_dir, &exclude).await?;
    }
    Ok(())
}

async fn copy_virtual_assets(
    paths: &AppPaths,
    idx: &AssetIndexFile,
    index_id: &str,
) -> Result<(), AppError> {
    for (virtual_path, obj) in &idx.objects {
        let src = paths.asset_object(&obj.hash);
        let dst = paths.asset_virtual(index_id, virtual_path);
        if let Some(p) = dst.parent() {
            tokio::fs::create_dir_all(p).await?;
        }
        tokio::fs::copy(&src, &dst).await?;
    }
    Ok(())
}

async fn send_progress(tx: &mpsc::Sender<TaskEvent>, id: JobId, pct: u8, msg: &str) {
    let _ = tx
        .send(TaskEvent::Progress {
            id,
            pct,
            msg: msg.to_string(),
        })
        .await;
}
