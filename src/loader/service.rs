//! Loader install / remove facade. Wraps FabricMetaClient + QuiltMetaClient
//! and persists `InstanceManifest.loader` via `instance::store`.
//!
//! Held as `Arc<LoaderService>` in `src/tui/run.rs` (mirrors `Arc<JavaService>`).
//!
//! See 06-RESEARCH.md §Loader Install Flow for the four-step pipeline.

use std::path::Path;
use std::sync::Arc;

use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::domain::instance::ModloaderKind;
use crate::error::AppError;
use crate::install::version_installer::LIB_CONCURRENCY;
use crate::loader::error::LoaderError;
use crate::loader::fabric::{FabricMetaClient, FabricProfile};
use crate::loader::forge_meta::ForgeMetaClient;
use crate::loader::maven::{maven_coord_to_path, maven_download_url};
use crate::loader::neoforge_meta::NeoForgeMetaClient;
use crate::loader::quilt::{QuiltMetaClient, QuiltProfile};
use crate::loader::types::{LoaderInfo, LoaderType, LoaderVersionEntry};
use crate::mojang::cache::{atomic_write, verify_sha1};
use crate::persistence::paths::AppPaths;
use crate::tasks::TaskEvent;

/// Re-attach is the on-disk state where the loader version JSON exists AND
/// every library is present in `libraries/`. We skip steps 1-3 of install
/// in that case (Open Question 1 lock-in from 06-RESEARCH.md).
const _OPEN_Q1_REATTACH_LOCK: () = ();

#[derive(Debug)]
pub struct LoaderService {
    fabric: FabricMetaClient,
    quilt: QuiltMetaClient,
    forge: ForgeMetaClient,
    neoforge: NeoForgeMetaClient,
}

impl LoaderService {
    #[tracing::instrument(skip_all)]
    pub fn new() -> Result<Self, LoaderError> {
        Ok(Self {
            fabric: FabricMetaClient::new()?,
            quilt: QuiltMetaClient::new()?,
            forge: ForgeMetaClient::new()?,
            neoforge: NeoForgeMetaClient::new()?,
        })
    }

    #[cfg(test)]
    pub fn with_clients(
        fabric: FabricMetaClient,
        quilt: QuiltMetaClient,
        forge: ForgeMetaClient,
        neoforge: NeoForgeMetaClient,
    ) -> Self {
        Self { fabric, quilt, forge, neoforge }
    }

    /// List all loader versions for the given loader type.
    ///
    /// For Fabric and Quilt, `mc_version` is ignored (both meta APIs return ALL
    /// loader versions regardless of game version). For Forge and NeoForge,
    /// `mc_version` is used to filter the version list by MC compatibility.
    #[tracing::instrument(skip_all, fields(?loader_type, mc_version = %mc_version))]
    pub async fn list_loader_versions(
        &self,
        loader_type: LoaderType,
        mc_version: &str,
    ) -> Result<Vec<LoaderVersionEntry>, LoaderError> {
        match loader_type {
            LoaderType::Fabric => self.fabric.list_loader_versions().await,
            LoaderType::Quilt => self.quilt.list_loader_versions().await,
            LoaderType::Forge => self.forge.list_loader_versions(mc_version).await,
            LoaderType::NeoForge => self.neoforge.list_loader_versions(mc_version).await,
        }
    }

    /// Remove the active loader from `slug`.
    ///
    /// 1. Read manifest; if no loader, return Ok(()) (no-op).
    /// 2. `remove_dir_all(versions/{loader.version_id}/)` (NotFound is non-fatal).
    /// 3. `manifest.loader = None` and write_instance_manifest.
    ///
    /// Does NOT touch `libraries/` — Maven layout is shared across instances.
    #[tracing::instrument(skip_all, fields(slug = %slug))]
    pub async fn remove_loader(
        &self,
        paths: &AppPaths,
        slug: &str,
    ) -> Result<(), LoaderError> {
        let mut manifest = crate::instance::store::read_instance_manifest(paths, slug)
            .await
            .map_err(map_app_error)?;
        let Some(loader) = manifest.loader.take() else {
            return Ok(());
        };
        let dir = paths.versions_dir().join(&loader.version_id);
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(LoaderError::Io(e)),
        }
        // manifest.loader is already None (taken above)
        crate::instance::store::write_instance_manifest(paths, &manifest)
            .await
            .map_err(map_app_error)?;
        Ok(())
    }

    /// Install a modloader into the instance.
    ///
    /// For Fabric/Quilt: four-step pipeline (fetch profile → download libs →
    /// write version JSON → write manifest LAST).
    ///
    /// For Forge/NeoForge: subprocess pipeline (re-attach check → download
    /// installer JAR → stage + ForgeWrapper → run installer → harvest → write
    /// manifest LAST). Requires `jre_path` — obtain via
    /// `JavaService::resolve_jre_for_mc_version_install` before calling.
    ///
    /// For Fabric/Quilt callers, pass a dummy path (e.g. `Path::new(".")`) until
    /// Wave 5 wires up proper JRE resolution in `run.rs`.
    ///
    /// Cancellation: token checked at every await; returns `LoaderError::Cancelled`
    /// without modifying instance.json (atomicity invariant — Pitfall 7).
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip_all, fields(slug = %slug, loader = ?loader_type, version = %loader_version))]
    pub async fn install_loader(
        &self,
        paths: &AppPaths,
        slug: &str,
        mc_version: &str,
        loader_type: LoaderType,
        loader_version: &str,
        jre_path: &Path,
        progress_tx: mpsc::Sender<TaskEvent>,
        token: CancellationToken,
        job_id: crate::tasks::JobId,
    ) -> Result<(), LoaderError> {
        match loader_type {
            LoaderType::Fabric | LoaderType::Quilt => {
                // Existing Phase 6 pipeline — `jre_path` is unused for HTTP-only loaders.
                let _ = jre_path;
                install_loader_impl(
                    self,
                    InstallArgs {
                        paths,
                        slug,
                        mc_version,
                        loader_type,
                        loader_version,
                        progress_tx,
                        token,
                        job_id,
                    },
                )
                .await
            }
            LoaderType::Forge | LoaderType::NeoForge => {
                install_subprocess_loader(
                    self,
                    paths,
                    slug,
                    mc_version,
                    loader_type,
                    loader_version,
                    jre_path,
                    progress_tx,
                    token,
                    job_id,
                )
                .await
            }
        }
    }
}

/// Map an `AppError` from the persistence layer into a `LoaderError`.
fn map_app_error(e: AppError) -> LoaderError {
    match e {
        AppError::Io(io) => LoaderError::Io(io),
        other => LoaderError::ProfileWrite {
            path: String::new(),
            reason: other.to_string(),
        },
    }
}

// -----------------------------------------------------------------
// install_subprocess_loader: Forge/NeoForge six-step pipeline
// -----------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn install_subprocess_loader(
    svc: &LoaderService,
    paths: &AppPaths,
    slug: &str,
    mc_version: &str,
    loader_type: LoaderType,
    loader_version: &str,
    jre_path: &Path,
    progress_tx: mpsc::Sender<TaskEvent>,
    token: CancellationToken,
    job_id: crate::tasks::JobId,
) -> Result<(), LoaderError> {
    use crate::loader::{forgewrapper, harvest, installer_subprocess, staging};

    let modloader_kind = match loader_type {
        LoaderType::Forge => ModloaderKind::Forge,
        LoaderType::NeoForge => ModloaderKind::NeoForge,
        _ => unreachable!("install_subprocess_loader only handles Forge/NeoForge"),
    };

    // Predict the version-id the installer will produce. Re-attach uses this
    // for the disk-existence check; harvest will validate after the fact.
    let expected_version_id = match loader_type {
        LoaderType::Forge => format!("{mc_version}-forge-{loader_version}"),
        LoaderType::NeoForge => format!("neoforge-{loader_version}"),
        _ => unreachable!(),
    };

    macro_rules! send_progress {
        ($pct:expr, $msg:expr) => {{
            let _ = progress_tx
                .send(TaskEvent::Progress { id: job_id, pct: $pct, msg: $msg.to_string() })
                .await;
        }};
    }

    // ------------------------------------------------------------------
    // STEP 1 — Re-attach pre-check (D-04)
    // If predicted version JSON exists AND all declared libraries are on disk,
    // skip steps 2-5 and jump straight to manifest write.
    // ------------------------------------------------------------------
    let predicted_json = paths.version_json(&expected_version_id);
    if tokio::fs::try_exists(&predicted_json).await.unwrap_or(false) {
        if let Ok(bytes) = tokio::fs::read(&predicted_json).await {
            #[derive(serde::Deserialize)]
            struct VJ {
                #[serde(default)]
                libraries: Vec<crate::loader::types::LoaderLibrary>,
            }
            if let Ok(vj) = serde_json::from_slice::<VJ>(&bytes) {
                let mut all_present = true;
                for lib in &vj.libraries {
                    if let Ok(rel) = crate::loader::maven::maven_coord_to_path(&lib.name) {
                        if !tokio::fs::try_exists(&paths.library_path(&rel))
                            .await
                            .unwrap_or(false)
                        {
                            all_present = false;
                            break;
                        }
                    }
                }
                if all_present {
                    tracing::info!(
                        version_id = %expected_version_id,
                        "re-attach: all artifacts on disk, skipping install"
                    );
                    send_progress!(99, "Re-attach: writing manifest");
                    write_manifest_loader(paths, slug, modloader_kind, loader_version, &expected_version_id).await?;
                    send_progress!(100, "Done");
                    return Ok(());
                }
            }
        }
    }

    if token.is_cancelled() {
        return Err(LoaderError::Cancelled);
    }

    // ------------------------------------------------------------------
    // STEP 2 — Download installer JAR (skip if already cached)
    // ------------------------------------------------------------------
    send_progress!(5, "Downloading installer");
    let installer_url = match loader_type {
        LoaderType::Forge => svc.forge.installer_url(mc_version, loader_version),
        LoaderType::NeoForge => svc.neoforge.installer_url(loader_version),
        _ => unreachable!(),
    };
    let installer_filename = match loader_type {
        LoaderType::Forge => format!("forge-{mc_version}-{loader_version}-installer.jar"),
        LoaderType::NeoForge => format!("neoforge-{loader_version}-installer.jar"),
        _ => unreachable!(),
    };
    let installer_dst = paths.cache_dir.join("installers").join(&installer_filename);
    if !tokio::fs::try_exists(&installer_dst).await.unwrap_or(false) {
        if let Some(parent) = installer_dst.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                LoaderError::InstallerJarFetch {
                    reason: format!("mkdir installers: {e}"),
                }
            })?;
        }
        let bytes = reqwest::get(&installer_url)
            .await
            .map_err(|e| LoaderError::InstallerJarFetch {
                reason: format!("GET {installer_url}: {e}"),
            })?
            .error_for_status()
            .map_err(|e| LoaderError::InstallerJarFetch {
                reason: format!("status {installer_url}: {e}"),
            })?
            .bytes()
            .await
            .map_err(|e| LoaderError::InstallerJarFetch {
                reason: format!("body {installer_url}: {e}"),
            })?;
        crate::mojang::cache::atomic_write(&installer_dst, &bytes)
            .await
            .map_err(|e| LoaderError::InstallerJarFetch {
                reason: format!("write {}: {e}", installer_dst.display()),
            })?;
        // Best-effort: verify .sha1 sidecar when available (T-07-12, Pitfall 9).
        let sha_url = format!("{installer_url}.sha1");
        if let Ok(resp) = reqwest::get(&sha_url).await {
            if resp.status().is_success() {
                if let Ok(text) = resp.text().await {
                    let expected = text.trim().to_string();
                    if expected.len() == 40 {
                        match crate::mojang::cache::verify_sha1(&installer_dst, &expected).await {
                            Ok(true) => {}
                            Ok(false) => {
                                return Err(LoaderError::InstallerJarFetch {
                                    reason: format!(
                                        "SHA1 mismatch for {} (expected {expected})",
                                        installer_dst.display()
                                    ),
                                });
                            }
                            Err(e) => {
                                tracing::warn!(%e, "SHA1 verify failed; continuing");
                            }
                        }
                    }
                }
            } else if resp.status().as_u16() != 404 {
                tracing::warn!(
                    status = %resp.status(),
                    "SHA1 sidecar returned non-404 error; ignoring"
                );
            }
        }
    }
    send_progress!(25, "Installer JAR ready");

    if token.is_cancelled() {
        return Err(LoaderError::Cancelled);
    }

    // ------------------------------------------------------------------
    // STEP 3 — Build staging dir
    // ------------------------------------------------------------------
    let staging = staging::StagingDir::create(paths, slug).await?;
    let staging_root = staging.root().to_path_buf();

    // Inner async block so we can centralise cleanup on any error.
    let outcome: Result<harvest::HarvestedInstall, LoaderError> = async {
        staging.populate_vanilla(paths, mc_version).await?;
        staging.write_launcher_profiles().await?;
        let wrapper_jar = forgewrapper::ensure_extracted(paths).await?;

        if token.is_cancelled() {
            return Err(LoaderError::Cancelled);
        }

        // --------------------------------------------------------------
        // STEP 4 — Run installer subprocess
        // --------------------------------------------------------------
        send_progress!(30, "Running installer");
        let unix_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let log_path = paths
            .instance_dir(slug)
            .join(format!("loader-install-{unix_ts}.log"));
        if let Some(parent) = log_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }

        // Classpath: ForgeWrapper.jar<sep>installer.jar
        // Per 07-RESEARCH.md Pattern 5 + ForgeWrapper 1.6.0 README install invocation:
        //   java -cp <wrapper>:<installer> \
        //     io.github.zekerzhayard.forgewrapper.installer.Installer \
        //     --installer=<jar> --instance=<staging>
        //
        // INSTALL-TIME class: forgewrapper.installer.Installer (calls
        // PostProcessors.process() directly; bypasses SimpleInstaller.main()
        // GUI/server branch). NOT forgewrapper.installer.Main, which is the
        // LAUNCH-TIME mainClass override (Phase 12 wiring). GAP-7-A: pre-7.1-02
        // wired Main here, throwing NoClassDefFoundError: cpw/mods/modlauncher/Launcher
        // because Main's static init resolves modlauncher (absent from the
        // install-time classpath).
        let sep = if cfg!(windows) { ";" } else { ":" };
        let classpath = format!(
            "{}{sep}{}",
            wrapper_jar.display(),
            installer_dst.display()
        );
        let args: Vec<String> = vec![
            "-cp".into(),
            classpath,
            forgewrapper::FORGE_WRAPPER_INSTALLER_CLASS.to_string(),
            format!("--installer={}", installer_dst.display()),
            format!("--instance={}", staging.root().display()),
        ];

        installer_subprocess::run_installer(
            jre_path,
            &[],
            &args,
            staging.root(),
            &log_path,
            progress_tx.clone(),
            job_id,
            token.clone(),
        )
        .await?;

        send_progress!(85, "Harvesting libraries");

        if token.is_cancelled() {
            return Err(LoaderError::Cancelled);
        }

        // --------------------------------------------------------------
        // STEP 5 — Harvest
        // 07-WARNING-4 fix: pass vanilla_mc_id so harvest excludes vanilla dir.
        // --------------------------------------------------------------
        let harvested = match harvest::harvest_install(
            staging.root(),
            Some(&expected_version_id),
            mc_version,
        )
        .await
        {
            Ok(h) => h,
            Err(_) => {
                // Fallback: accept whatever the installer produced.
                tracing::warn!(
                    expected = %expected_version_id,
                    "expected version_id not found in staging; accepting installer output"
                );
                harvest::harvest_install(staging.root(), None, mc_version).await?
            }
        };
        harvest::copy_libraries_into_shared(paths, &harvested.libraries, &token).await?;
        harvest::write_version_json(paths, &harvested.version_id, &harvested.version_json_bytes)
            .await?;
        send_progress!(95, "Writing manifest");
        Ok(harvested)
    }
    .await;

    let harvested = match outcome {
        Ok(h) => h,
        Err(e) => {
            // Steps 3-5 failed — clean staging, propagate.
            staging::cleanup_staging(&staging_root).await;
            return Err(e);
        }
    };

    // ------------------------------------------------------------------
    // STEP 6 — Manifest write LAST (atomicity invariant — Pitfall 7)
    // The instance.json loader field is NEVER written if any step above
    // failed or was cancelled.
    // ------------------------------------------------------------------
    write_manifest_loader(
        paths,
        slug,
        modloader_kind,
        loader_version,
        &harvested.version_id,
    )
    .await?;
    send_progress!(100, "Done");

    // STEP 7 — Cleanup staging (post-success best-effort)
    staging::cleanup_staging(&staging_root).await;
    Ok(())
}

/// Write or update the instance manifest `loader` field.
///
/// Reads the existing manifest, sets `loader = Some(LoaderInfo { … })`,
/// and writes back atomically. Used by both the re-attach fast path and the
/// full subprocess pipeline.
async fn write_manifest_loader(
    paths: &AppPaths,
    slug: &str,
    kind: ModloaderKind,
    version: &str,
    version_id: &str,
) -> Result<(), LoaderError> {
    let mut manifest = crate::instance::store::read_instance_manifest(paths, slug)
        .await
        .map_err(map_app_error)?;
    manifest.loader = Some(LoaderInfo {
        kind,
        version: version.to_string(),
        version_id: version_id.to_string(),
    });
    crate::instance::store::write_instance_manifest(paths, &manifest)
        .await
        .map_err(map_app_error)?;
    Ok(())
}

// -----------------------------------------------------------------
// install_loader_impl: four-step pipeline (Fabric/Quilt)
// -----------------------------------------------------------------

struct InstallArgs<'a> {
    paths: &'a AppPaths,
    slug: &'a str,
    mc_version: &'a str,
    loader_type: LoaderType,
    loader_version: &'a str,
    progress_tx: mpsc::Sender<TaskEvent>,
    token: CancellationToken,
    job_id: crate::tasks::JobId,
}

async fn install_loader_impl(
    svc: &LoaderService,
    args: InstallArgs<'_>,
) -> Result<(), LoaderError> {
    let InstallArgs {
        paths,
        slug,
        mc_version,
        loader_type,
        loader_version,
        progress_tx,
        token,
        job_id,
    } = args;
    // Helper closure to send progress updates without caring about channel errors.
    let send_progress = |pct: u8, msg: String| {
        let progress_tx = progress_tx.clone();
        async move {
            let _ = progress_tx
                .send(TaskEvent::Progress { id: job_id, pct, msg })
                .await;
        }
    };

    macro_rules! check_cancel {
        () => {
            if token.is_cancelled() {
                return Err(LoaderError::Cancelled);
            }
        };
    }

    // -----------------------------------------------------------------
    // Re-attach pre-check (Open Question 1 lock-in):
    // If we can predict the version_id from the known Fabric/Quilt ID
    // format AND the version JSON + all library paths from a prior
    // install are present on disk, skip Steps 1-3 entirely (no HTTP
    // fetches at all) — go straight to Step 4.
    //
    // The predicted format ("fabric-loader-{v}-{mc}" / "quilt-loader-{v}-{mc}")
    // matches what the meta APIs return for the `id` field. If this
    // prediction is wrong (edge case), we fall through to the full install.
    // -----------------------------------------------------------------
    let predicted_version_id = predict_version_id(loader_type, loader_version, mc_version);
    let predicted_version_json = paths.version_json(&predicted_version_id);

    // Quick path: check if version JSON exists using the predicted id.
    // We can only do the full library-list check after we have the profile,
    // so here we check the version JSON only; if present we fetch the
    // profile JSON from disk (not the meta API) to get library list.
    let pre_reattach = if tokio::fs::try_exists(&predicted_version_json)
        .await
        .unwrap_or(false)
    {
        // Parse the on-disk version JSON to extract library list without
        // calling the meta API — avoids a network round-trip on re-attach.
        try_reattach_from_disk(paths, loader_type, &predicted_version_id).await
    } else {
        None
    };

    if let Some((profile_id, libs_on_disk)) = pre_reattach {
        check_cancel!();
        tracing::info!(
            slug,
            version_id = %profile_id,
            "loader re-attach: all artifacts already on disk (fast path)"
        );
        send_progress(95, "Re-attaching existing loader".to_string()).await;
        // Jump directly to Step 4 — no downloads, no profile fetch.
        check_cancel!();
        let mut manifest = crate::instance::store::read_instance_manifest(paths, slug)
            .await
            .map_err(map_app_error)?;
        manifest.loader = Some(LoaderInfo {
            kind: match loader_type {
                LoaderType::Fabric => ModloaderKind::Fabric,
                LoaderType::Quilt => ModloaderKind::Quilt,
                LoaderType::Forge => ModloaderKind::Forge,
                LoaderType::NeoForge => ModloaderKind::NeoForge,
            },
            version: loader_version.to_string(),
            version_id: profile_id.clone(),
        });
        crate::instance::store::write_instance_manifest(paths, &manifest)
            .await
            .map_err(map_app_error)?;
        send_progress(100, "Install complete".to_string()).await;
        drop(libs_on_disk); // not used past this point
        return Ok(());
    }

    // -----------------------------------------------------------------
    // Step 1: fetch loader profile JSON (1%)
    // -----------------------------------------------------------------
    check_cancel!();
    send_progress(1, format!("Fetching {} meta", loader_label(loader_type))).await;

    // Both profiles carry (id, raw_bytes, libraries).
    // Consume canonical `crate::loader::types::LoaderLibrary` directly from
    // both client profiles — no per-loader bridge struct (no NormLib).
    let (profile_id, raw_bytes, libs, http) = match loader_type {
        LoaderType::Fabric => {
            let p: FabricProfile = svc.fabric.fetch_profile(mc_version, loader_version).await?;
            (p.id, p.raw_bytes, p.libraries, svc.fabric.http().clone())
        }
        LoaderType::Quilt => {
            let p: QuiltProfile = svc.quilt.fetch_profile(mc_version, loader_version).await?;
            // Quilt's no-hash invariant is parse-time (asserted by 06-04-01) —
            // every sha1/sha256/sha512/md5 is already None on these entries.
            (p.id, p.raw_bytes, p.libraries, svc.quilt.http().clone())
        }
        LoaderType::Forge | LoaderType::NeoForge => {
            return Err(LoaderError::MetaFetch {
                loader: if loader_type == LoaderType::Forge { "forge" } else { "neoforge" },
                reason: "Forge/NeoForge installer pipeline not yet implemented (07-03)".into(),
            });
        }
    };

    check_cancel!();

    // -----------------------------------------------------------------
    // Post-fetch re-attach check:
    // Now that we have the actual profile.id and library list, do a
    // thorough is_already_installed check.
    // -----------------------------------------------------------------
    let version_json_path = paths.version_json(&profile_id);
    let already_attached =
        is_already_installed(paths, &version_json_path, &libs).await?;

    if already_attached {
        tracing::info!(
            slug,
            version_id = %profile_id,
            "loader re-attach: all artifacts on disk"
        );
        send_progress(95, "Re-attaching existing loader".to_string()).await;
        // Skip downloads + version JSON write; fall through to Step 4 below.
    } else {
        // -----------------------------------------------------------
        // Step 2: download libraries (2 → 90%)
        // -----------------------------------------------------------
        let total = libs.len() as u64;
        let sem = Arc::new(Semaphore::new(LIB_CONCURRENCY));
        let mut set = tokio::task::JoinSet::new();

        for (i, lib) in libs.iter().cloned().enumerate() {
            let sem = Arc::clone(&sem);
            let http = http.clone();
            let paths = paths.clone();
            let token = token.clone();
            set.spawn(async move {
                let _permit = sem
                    .acquire_owned()
                    .await
                    .map_err(|e| LoaderError::ProfileWrite {
                        path: lib.name.clone(),
                        reason: format!("semaphore closed: {e}"),
                    })?;
                if token.is_cancelled() {
                    return Err(LoaderError::Cancelled);
                }
                download_one_library(&http, &paths, &lib).await?;
                Ok::<usize, LoaderError>(i)
            });
        }

        let mut completed: u64 = 0;
        while let Some(res) = set.join_next().await {
            if token.is_cancelled() {
                set.abort_all();
                return Err(LoaderError::Cancelled);
            }
            match res {
                Ok(Ok(_idx)) => {
                    completed += 1;
                    let pct = if total == 0 {
                        90
                    } else {
                        2 + ((completed * 88) / total) as u8
                    };
                    send_progress(pct, "Downloading loader libraries".to_string()).await;
                }
                Ok(Err(e)) => {
                    set.abort_all();
                    return Err(e);
                }
                Err(e) if e.is_cancelled() => {
                    set.abort_all();
                    return Err(LoaderError::Cancelled);
                }
                Err(e) => {
                    set.abort_all();
                    return Err(LoaderError::ProfileWrite {
                        path: profile_id.clone(),
                        reason: format!("library task panic: {e}"),
                    });
                }
            }
        }

        // -----------------------------------------------------------
        // Step 3: write loader version JSON (95%)
        // -----------------------------------------------------------
        check_cancel!();
        send_progress(95, "Writing version JSON".to_string()).await;
        // Phase 8.4 GAP-LIBRARY-SHAPE-08 (round-4 BLOCKER closure): translate
        // fabric-meta / quilt-meta wire shape to Mojang on-disk shape BEFORE
        // atomic_write. The launcher reads ONE shape (Mojang); the loader's
        // upstream API shape stops at this boundary. Forge/NeoForge harvests
        // already write Mojang shape (07-RESEARCH.md §640) and pass through.
        let mojang_bytes: Vec<u8> = match loader_type {
            LoaderType::Fabric => crate::loader::fabric::to_mojang_shape(&raw_bytes)?,
            LoaderType::Quilt => crate::loader::quilt::to_mojang_shape(&raw_bytes)?,
            LoaderType::Forge | LoaderType::NeoForge => raw_bytes.to_vec(),
        };
        atomic_write(&version_json_path, &mojang_bytes)
            .await
            .map_err(|e| LoaderError::ProfileWrite {
                path: version_json_path.display().to_string(),
                reason: e.to_string(),
            })?;
    }

    // -----------------------------------------------------------------
    // Step 4: write instance manifest — LAST (atomicity invariant)
    // instance.json is NEVER written if cancelled before this point.
    // -----------------------------------------------------------------
    check_cancel!();
    let mut manifest = crate::instance::store::read_instance_manifest(paths, slug)
        .await
        .map_err(map_app_error)?;
    manifest.loader = Some(LoaderInfo {
        kind: match loader_type {
            LoaderType::Fabric => ModloaderKind::Fabric,
            LoaderType::Quilt => ModloaderKind::Quilt,
            LoaderType::Forge => ModloaderKind::Forge,
            LoaderType::NeoForge => ModloaderKind::NeoForge,
        },
        version: loader_version.to_string(),
        version_id: profile_id.clone(), // VERBATIM from profile.id (Pitfall 7)
    });
    crate::instance::store::write_instance_manifest(paths, &manifest)
        .await
        .map_err(map_app_error)?;

    send_progress(100, "Install complete".to_string()).await;
    Ok(())
}

// -----------------------------------------------------------------
// Internal helpers — operate directly on canonical LoaderLibrary
// (no NormLib bridge struct — canonical type consumed directly)
// -----------------------------------------------------------------

fn loader_label(t: LoaderType) -> &'static str {
    match t {
        LoaderType::Fabric => "Fabric",
        LoaderType::Quilt => "Quilt",
        LoaderType::Forge => "Forge",
        LoaderType::NeoForge => "NeoForge",
    }
}

/// Best-effort fallback Maven repo when a library entry omits its `url`.
/// Per 06-RESEARCH.md, Fabric and Quilt profile entries occasionally
/// drop the field; we then fall back to the loader's primary repo.
fn fallback_repo_for(name: &str) -> &'static str {
    if name.starts_with("org.quiltmc:") {
        "https://maven.quiltmc.org/"
    } else {
        "https://maven.fabricmc.net/"
    }
}

/// Predict the loader version_id from loader_type + loader_version + mc_version.
///
/// Fabric and Quilt both use the format `{prefix}-{loader_version}-{mc_version}`.
/// This lets us check the on-disk path before hitting the meta API.
fn predict_version_id(loader_type: LoaderType, loader_version: &str, mc_version: &str) -> String {
    match loader_type {
        LoaderType::Fabric => {
            format!("fabric-loader-{loader_version}-{mc_version}")
        }
        LoaderType::Quilt => {
            format!("quilt-loader-{loader_version}-{mc_version}")
        }
        // Forge/NeoForge version IDs are determined by the installer output;
        // this prediction is unused for them (07-03 pipeline harvests the real ID).
        LoaderType::Forge => {
            format!("{mc_version}-forge-{loader_version}")
        }
        LoaderType::NeoForge => {
            format!("neoforge-{loader_version}")
        }
    }
}

/// Try to perform a re-attach purely from on-disk state.
///
/// Reads the on-disk version JSON, parses Mojang shape (Phase 8.4 invariant),
/// extracts each library's maven coord (`name`), and checks the per-coord
/// JAR exists. Returns `Some((version_id, libs))` on full presence; `None`
/// otherwise. The returned LoaderLibrary entries have ONLY `name` populated
/// — re-attach callers must NOT depend on sha1/url/size being Some for
/// post-reattach reads.
async fn try_reattach_from_disk(
    paths: &AppPaths,
    _loader_type: LoaderType,
    version_id: &str,
) -> Option<(String, Vec<crate::loader::types::LoaderLibrary>)> {
    let version_json_path = paths.version_json(version_id);
    let bytes = tokio::fs::read(&version_json_path).await.ok()?;

    // Phase 8.4 GAP-LIBRARY-SHAPE-08: on-disk JSON is now Mojang shape; the
    // re-attach existence check only needs the maven coord (`name`) per library
    // entry — derived path comes from maven_coord_to_path(&lib.name) below,
    // which is what download_one_library uses to write the file in the first
    // place. The Mojang-shape `downloads.artifact.path` field would yield the
    // SAME relative path; either is acceptable. We derive from `name` to keep
    // the existence check decoupled from the download artifact (one source of
    // truth).
    #[derive(serde::Deserialize)]
    struct MinimalProfile {
        id: String,
        #[serde(default)]
        libraries: Vec<MinimalLibrary>,
    }
    #[derive(serde::Deserialize)]
    struct MinimalLibrary {
        name: String,
    }

    let parsed: MinimalProfile = serde_json::from_slice(&bytes).ok()?;

    // Check every library is on disk (existence-only after re-attach).
    let mut libs: Vec<crate::loader::types::LoaderLibrary> = Vec::with_capacity(parsed.libraries.len());
    for lib in &parsed.libraries {
        let rel = maven_coord_to_path(&lib.name).ok()?;
        let dest = paths.library_path(&rel);
        if !tokio::fs::try_exists(&dest).await.unwrap_or(false) {
            return None;
        }
        libs.push(crate::loader::types::LoaderLibrary {
            name: lib.name.clone(),
            url: None, sha1: None, sha256: None, sha512: None, md5: None, size: None,
        });
    }

    Some((parsed.id, libs))
}

/// Re-attach detection: version JSON present AND every library present on disk.
/// Returns true only when all artifacts exist — a partial install returns false
/// and triggers a full download pass.
async fn is_already_installed(
    paths: &AppPaths,
    version_json_path: &std::path::PathBuf,
    libs: &[crate::loader::types::LoaderLibrary],
) -> Result<bool, LoaderError> {
    if !tokio::fs::try_exists(version_json_path).await.unwrap_or(false) {
        return Ok(false);
    }
    for lib in libs {
        let rel = maven_coord_to_path(&lib.name)?;
        let dest = paths.library_path(&rel);
        if !tokio::fs::try_exists(&dest).await.unwrap_or(false) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Download one library and write to its Maven path.
/// Skip if already present + (sha1 present and verifies / no sha1 → existence is enough).
async fn download_one_library(
    http: &reqwest::Client,
    paths: &AppPaths,
    lib: &crate::loader::types::LoaderLibrary,
) -> Result<(), LoaderError> {
    let rel = maven_coord_to_path(&lib.name)?;
    let dest = paths.library_path(&rel);

    // Idempotent skip: if file exists and hash matches (or no hash), skip download.
    if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
        match &lib.sha1 {
            Some(expected) => {
                let ok = verify_sha1(&dest, expected)
                    .await
                    .map_err(|e| LoaderError::ProfileWrite {
                        path: dest.display().to_string(),
                        reason: format!("sha1 verify (existing): {e}"),
                    })?;
                if ok {
                    return Ok(());
                }
                // Hash mismatch on existing file — fall through to re-download.
            }
            None => {
                // Quilt (or hashless Fabric entry): no hash → existence is sufficient (Pattern 6).
                return Ok(());
            }
        }
    }

    let repo = lib
        .url
        .as_deref()
        .unwrap_or_else(|| fallback_repo_for(&lib.name));
    let url = maven_download_url(repo, &lib.name)?;

    let bytes = http
        .get(&url)
        .send()
        .await
        .map_err(|e| LoaderError::MetaFetch {
            loader: "library",
            reason: format!("GET {url}: {e}"),
        })?
        .error_for_status()
        .map_err(|e| LoaderError::MetaFetch {
            loader: "library",
            reason: format!("status {url}: {e}"),
        })?
        .bytes()
        .await
        .map_err(|e| LoaderError::MetaFetch {
            loader: "library",
            reason: format!("body {url}: {e}"),
        })?
        .to_vec();

    // Verify SHA1 BEFORE writing — fail fast (T-06-11).
    if let Some(expected) = &lib.sha1 {
        let got = sha1_hex(&bytes);
        if !got.eq_ignore_ascii_case(expected) {
            return Err(LoaderError::Sha1Mismatch {
                path: dest.display().to_string(),
                expected: expected.clone(),
                got,
            });
        }
    }

    atomic_write(&dest, &bytes)
        .await
        .map_err(|e| LoaderError::ProfileWrite {
            path: dest.display().to_string(),
            reason: e.to_string(),
        })?;
    Ok(())
}

/// SHA1 hex of bytes (40 chars lowercase).
fn sha1_hex(bytes: &[u8]) -> String {
    crate::mojang::cache::sha1_hex_of_bytes(bytes)
}

// -----------------------------------------------------------------
// Tests
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::forge_meta::ForgeMetaClient;
    use crate::loader::neoforge_meta::NeoForgeMetaClient;
    use httpmock::Method::GET;
    use httpmock::MockServer;
    use tempfile::TempDir;

    fn make_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    fn make_service(server: &MockServer) -> LoaderService {
        let fabric = FabricMetaClient::new_with_base_url(server.base_url()).unwrap();
        let quilt = QuiltMetaClient::new_with_base_url(server.base_url()).unwrap();
        let forge = ForgeMetaClient::new().unwrap();
        let neoforge = NeoForgeMetaClient::new().unwrap();
        LoaderService::with_clients(fabric, quilt, forge, neoforge)
    }

    // ------------------------------------------------------------------
    // Helper: write a vanilla instance manifest to disk
    // ------------------------------------------------------------------

    async fn write_initial_vanilla_manifest(paths: &AppPaths, slug: &str) {
        use crate::domain::InstanceManifest;
        let m = InstanceManifest::new(slug.into(), slug.into(), "1.21.4".into());
        crate::instance::store::write_instance_manifest(paths, &m)
            .await
            .unwrap();
    }

    // ------------------------------------------------------------------
    // Profile JSON builders
    // ------------------------------------------------------------------

    fn fabric_profile_json(server_base: &str, library_sha1: &str) -> String {
        format!(
            r#"{{
                "id": "fabric-loader-0.16.9-1.21.4",
                "inheritsFrom": "1.21.4",
                "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
                "arguments": {{ "game": [], "jvm": [] }},
                "libraries": [
                    {{
                        "name": "net.fabricmc:fabric-loader:0.16.9",
                        "url": "{server_base}/",
                        "sha1": "{library_sha1}"
                    }}
                ]
            }}"#
        )
    }

    fn quilt_profile_json(server_base: &str) -> String {
        format!(
            r#"{{
                "id": "quilt-loader-0.30.0-beta.7-1.21.4",
                "inheritsFrom": "1.21.4",
                "mainClass": "org.quiltmc.loader.impl.launch.knot.KnotClient",
                "arguments": {{ "game": [] }},
                "libraries": [
                    {{
                        "name": "org.quiltmc:quilt-loader:0.30.0-beta.7",
                        "url": "{server_base}/"
                    }}
                ]
            }}"#
        )
    }

    // ------------------------------------------------------------------
    // list_loader_versions tests
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_loader_versions_dispatches_fabric() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/versions/loader");
            then.status(200).body(
                r#"[{"version":"0.16.9","stable":true,"maven":"net.fabricmc:fabric-loader:0.16.9","build":509,"separator":"."}]"#,
            );
        });
        let svc = make_service(&server);
        let v = svc
            .list_loader_versions(LoaderType::Fabric, "1.21.4")
            .await
            .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].version, "0.16.9");
        assert!(v[0].stable);
    }

    #[tokio::test]
    async fn test_list_loader_versions_dispatches_quilt() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v3/versions/loader");
            then.status(200).body(
                r#"[{"version":"0.30.0-beta.7","maven":"org.quiltmc:quilt-loader:0.30.0-beta.7","build":120,"separator":"-"}]"#,
            );
        });
        let svc = make_service(&server);
        let v = svc
            .list_loader_versions(LoaderType::Quilt, "1.21.4")
            .await
            .unwrap();
        assert_eq!(v.len(), 1);
        assert!(!v[0].stable, "beta should be unstable");
        use crate::loader::quilt::is_quilt_stable;
        assert!(!is_quilt_stable("0.30.0-beta.7"));
    }

    // ------------------------------------------------------------------
    // remove_loader tests
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_remove_loader_clears_manifest_and_removes_version_dir() {
        use crate::domain::InstanceManifest;
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();
        let svc = make_service(&server);

        let slug = "ti";
        let mut m = InstanceManifest::new("ti".into(), slug.into(), "1.21.4".into());
        let version_id = "fabric-loader-0.16.9-1.21.4";
        m.loader = Some(LoaderInfo {
            kind: ModloaderKind::Fabric,
            version: "0.16.9".into(),
            version_id: version_id.to_string(),
        });
        crate::instance::store::write_instance_manifest(&paths, &m)
            .await
            .unwrap();
        // Create the version dir with a fake JSON inside.
        let vd = paths.versions_dir().join(version_id);
        tokio::fs::create_dir_all(&vd).await.unwrap();
        tokio::fs::write(vd.join(format!("{version_id}.json")), b"{}")
            .await
            .unwrap();
        assert!(vd.exists());

        svc.remove_loader(&paths, slug).await.unwrap();

        let m2 = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        assert!(m2.loader.is_none(), "loader should be cleared");
        assert!(!vd.exists(), "version dir should be removed");
    }

    #[tokio::test]
    async fn test_remove_loader_noop_when_no_loader() {
        use crate::domain::InstanceManifest;
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();
        let svc = make_service(&server);

        let slug = "vanilla";
        let m = InstanceManifest::new("v".into(), slug.into(), "1.21.4".into());
        crate::instance::store::write_instance_manifest(&paths, &m)
            .await
            .unwrap();
        // No loader, no versions/ dir — should still be Ok
        svc.remove_loader(&paths, slug).await.unwrap();
    }

    // ------------------------------------------------------------------
    // install_loader tests
    // ------------------------------------------------------------------

    /// Build a MockServer + svc with all 4 mocks pre-registered for switch test.
    fn make_switch_server_and_svc(
        fabric_lib_bytes: &[u8],
        fabric_sha1: &str,
    ) -> (MockServer, LoaderService) {
        let server = MockServer::start();
        let base = server.base_url();
        let fabric_sha1 = fabric_sha1.to_string();
        let fabric_profile = fabric_profile_json(&base, &fabric_sha1);
        let quilt_profile = quilt_profile_json(&base);

        server.mock(|when, then| {
            when.method(GET)
                .path("/v2/versions/loader/1.21.4/0.16.9/profile/json");
            then.status(200)
                .header("content-type", "application/json")
                .body(fabric_profile);
        });
        server.mock(|when, then| {
            when.method(GET)
                .path("/v3/versions/loader/1.21.4/0.30.0-beta.7/profile/json");
            then.status(200)
                .header("content-type", "application/json")
                .body(quilt_profile);
        });
        server.mock(|when, then| {
            when.method(GET)
                .path("/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar");
            then.status(200).body(fabric_lib_bytes);
        });
        server.mock(|when, then| {
            when.method(GET)
                .path("/org/quiltmc/quilt-loader/0.30.0-beta.7/quilt-loader-0.30.0-beta.7.jar");
            then.status(200).body(b"" as &[u8]);
        });

        let svc = {
            let fabric = FabricMetaClient::new_with_base_url(server.base_url()).unwrap();
            let quilt = QuiltMetaClient::new_with_base_url(server.base_url()).unwrap();
            let forge = ForgeMetaClient::new().unwrap();
            let neoforge = NeoForgeMetaClient::new().unwrap();
            LoaderService::with_clients(fabric, quilt, forge, neoforge)
        };
        (server, svc)
    }

    #[tokio::test]
    async fn test_install_fabric_full_flow() {
        let fabric_lib_bytes: Vec<u8> = vec![0x01, 0x02, 0x03, 0x04];
        let fabric_sha1 = crate::mojang::cache::sha1_hex_of_bytes(&fabric_lib_bytes);

        let server = MockServer::start();
        let base = server.base_url();
        server.mock(|when, then| {
            when.method(GET)
                .path("/v2/versions/loader/1.21.4/0.16.9/profile/json");
            then.status(200)
                .header("content-type", "application/json")
                .body(fabric_profile_json(&base, &fabric_sha1));
        });
        server.mock(|when, then| {
            when.method(GET)
                .path("/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar");
            then.status(200).body(fabric_lib_bytes.clone());
        });

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let svc = make_service(&server);
        let slug = "inst";
        write_initial_vanilla_manifest(&paths, slug).await;

        let (tx, mut rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let job_id = crate::tasks::JobId(0);

        svc.install_loader(
            &paths,
            slug,
            "1.21.4",
            LoaderType::Fabric,
            "0.16.9",
            std::path::Path::new("."),
            tx,
            token,
            job_id,
        )
        .await
        .unwrap();

        // Check instance manifest has loader set
        let m = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        let loader = m.loader.unwrap();
        assert_eq!(loader.version_id, "fabric-loader-0.16.9-1.21.4");
        assert_eq!(loader.kind, ModloaderKind::Fabric);
        assert_eq!(loader.version, "0.16.9");

        // Check version JSON written
        let vj = paths.version_json("fabric-loader-0.16.9-1.21.4");
        assert!(vj.exists(), "version JSON should be written");

        // Check library written
        let lib_path = paths.library_path(
            "net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar",
        );
        assert!(lib_path.exists(), "library file should exist");
        assert_eq!(tokio::fs::read(&lib_path).await.unwrap(), fabric_lib_bytes);

        // Drain progress events
        rx.close();
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        assert!(!events.is_empty(), "should have emitted progress events");
    }

    #[tokio::test]
    async fn test_install_quilt_full_flow() {
        let server = MockServer::start();
        let base = server.base_url();
        server.mock(|when, then| {
            when.method(GET)
                .path("/v3/versions/loader/1.21.4/0.30.0-beta.7/profile/json");
            then.status(200)
                .header("content-type", "application/json")
                .body(quilt_profile_json(&base));
        });
        server.mock(|when, then| {
            when.method(GET)
                .path("/org/quiltmc/quilt-loader/0.30.0-beta.7/quilt-loader-0.30.0-beta.7.jar");
            then.status(200).body(b"quilt-bytes" as &[u8]);
        });

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let svc = make_service(&server);
        let slug = "quilt-inst";
        write_initial_vanilla_manifest(&paths, slug).await;

        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let job_id = crate::tasks::JobId(0);

        svc.install_loader(
            &paths,
            slug,
            "1.21.4",
            LoaderType::Quilt,
            "0.30.0-beta.7",
            std::path::Path::new("."),
            tx,
            token,
            job_id,
        )
        .await
        .unwrap();

        let m = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        let loader = m.loader.unwrap();
        assert_eq!(loader.version_id, "quilt-loader-0.30.0-beta.7-1.21.4");
        assert_eq!(loader.kind, ModloaderKind::Quilt);
        assert_eq!(loader.version, "0.30.0-beta.7");

        // Version JSON written
        let vj = paths.version_json("quilt-loader-0.30.0-beta.7-1.21.4");
        assert!(vj.exists(), "quilt version JSON should be written");

        // Library written (no hash check for Quilt)
        let lib_path = paths.library_path(
            "org/quiltmc/quilt-loader/0.30.0-beta.7/quilt-loader-0.30.0-beta.7.jar",
        );
        assert!(lib_path.exists(), "quilt library should exist");
    }

    #[tokio::test]
    async fn test_install_skips_when_already_attached() {
        let fabric_lib_bytes: Vec<u8> = vec![0x0A, 0x0B];
        let fabric_sha1 = crate::mojang::cache::sha1_hex_of_bytes(&fabric_lib_bytes);

        let server = MockServer::start();
        let base = server.base_url();

        // Register both profile and library mocks for the first install.
        // After the first install, we assert the profile mock was called exactly
        // once — the second install should skip the meta API entirely (re-attach).
        let profile_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/versions/loader/1.21.4/0.16.9/profile/json");
            then.status(200)
                .header("content-type", "application/json")
                .body(fabric_profile_json(&base, &fabric_sha1));
        });
        server.mock(|when, then| {
            when.method(GET)
                .path("/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar");
            then.status(200).body(fabric_lib_bytes.clone());
        });

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let svc = make_service(&server);
        let slug = "already";
        write_initial_vanilla_manifest(&paths, slug).await;

        // First install to populate artifacts
        let (tx, _rx) = mpsc::channel(64);
        svc.install_loader(
            &paths,
            slug,
            "1.21.4",
            LoaderType::Fabric,
            "0.16.9",
            std::path::Path::new("."),
            tx,
            CancellationToken::new(),
            crate::tasks::JobId(0),
        )
        .await
        .unwrap();

        // Verify artifacts landed on disk
        assert!(
            paths.version_json("fabric-loader-0.16.9-1.21.4").exists(),
            "version JSON should be present after first install"
        );
        let lib_path = paths.library_path(
            "net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar",
        );
        assert!(lib_path.exists(), "library should be present after first install");

        // Reset loader field so we can re-install (simulates re-launch after crash etc.)
        let mut m = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        m.loader = None;
        crate::instance::store::write_instance_manifest(&paths, &m)
            .await
            .unwrap();

        // Second install — should NOT fetch profile again (re-attach via disk check)
        let (tx2, _rx2) = mpsc::channel(64);
        svc.install_loader(
            &paths,
            slug,
            "1.21.4",
            LoaderType::Fabric,
            "0.16.9",
            std::path::Path::new("."),
            tx2,
            CancellationToken::new(),
            crate::tasks::JobId(1),
        )
        .await
        .unwrap();

        // profile_mock should have been called exactly once (first install only)
        profile_mock.assert_calls(1);

        // manifest has loader set after re-attach
        let m2 = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        assert!(m2.loader.is_some(), "loader should be set after re-attach");
        assert_eq!(
            m2.loader.unwrap().version_id,
            "fabric-loader-0.16.9-1.21.4"
        );
    }

    #[tokio::test]
    async fn test_install_sha1_mismatch_returns_sha1mismatch() {
        let correct_bytes: Vec<u8> = vec![0xAA, 0xBB, 0xCC];
        let correct_sha1 = crate::mojang::cache::sha1_hex_of_bytes(&correct_bytes);
        // Serve WRONG bytes
        let wrong_bytes: Vec<u8> = vec![0xFF, 0xEE];

        let server = MockServer::start();
        let base = server.base_url();
        server.mock(|when, then| {
            when.method(GET)
                .path("/v2/versions/loader/1.21.4/0.16.9/profile/json");
            then.status(200)
                .header("content-type", "application/json")
                .body(fabric_profile_json(&base, &correct_sha1));
        });
        server.mock(|when, then| {
            when.method(GET)
                .path("/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar");
            then.status(200).body(wrong_bytes.clone());
        });

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let svc = make_service(&server);
        let slug = "sha1test";
        write_initial_vanilla_manifest(&paths, slug).await;

        let (tx, _rx) = mpsc::channel(64);
        let result = svc
            .install_loader(
                &paths,
                slug,
                "1.21.4",
                LoaderType::Fabric,
                "0.16.9",
                std::path::Path::new("."),
                tx,
                CancellationToken::new(),
                crate::tasks::JobId(0),
            )
            .await;

        assert!(
            matches!(result, Err(LoaderError::Sha1Mismatch { .. })),
            "expected Sha1Mismatch, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_install_cancelled_before_completion_returns_cancelled() {
        let server = MockServer::start();
        // No mocks needed — token fires before any HTTP call
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let svc = make_service(&server);
        let slug = "cancel-test";
        write_initial_vanilla_manifest(&paths, slug).await;

        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        token.cancel(); // Fire before install begins

        let result = svc
            .install_loader(
                &paths,
                slug,
                "1.21.4",
                LoaderType::Fabric,
                "0.16.9",
                std::path::Path::new("."),
                tx,
                token,
                crate::tasks::JobId(0),
            )
            .await;

        assert!(
            matches!(result, Err(LoaderError::Cancelled)),
            "expected Cancelled, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_install_does_not_write_instance_manifest_on_cancel() {
        let server = MockServer::start();
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let svc = make_service(&server);
        let slug = "cancel-no-write";
        write_initial_vanilla_manifest(&paths, slug).await;

        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        token.cancel(); // Fire before install begins

        let _ = svc
            .install_loader(
                &paths,
                slug,
                "1.21.4",
                LoaderType::Fabric,
                "0.16.9",
                std::path::Path::new("."),
                tx,
                token,
                crate::tasks::JobId(0),
            )
            .await;

        // Manifest loader field MUST remain None
        let m = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        assert!(
            m.loader.is_none(),
            "loader should NOT be set after cancellation"
        );
    }

    #[tokio::test]
    async fn test_switch_loader_via_remove_then_install() {
        let fabric_lib_bytes: Vec<u8> = vec![0x01, 0x02, 0x03, 0x04];
        let fabric_sha1 = crate::mojang::cache::sha1_hex_of_bytes(&fabric_lib_bytes);

        let (_server, svc) = make_switch_server_and_svc(&fabric_lib_bytes, &fabric_sha1);
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let slug = "switch-test";
        write_initial_vanilla_manifest(&paths, slug).await;

        // 1. Install Fabric
        let (tx, _rx) = mpsc::channel(64);
        svc.install_loader(
            &paths,
            slug,
            "1.21.4",
            LoaderType::Fabric,
            "0.16.9",
            std::path::Path::new("."),
            tx,
            CancellationToken::new(),
            crate::tasks::JobId(0),
        )
        .await
        .unwrap();

        let m = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        assert_eq!(m.loader.as_ref().unwrap().kind, ModloaderKind::Fabric);
        let fabric_version_dir = paths.versions_dir().join("fabric-loader-0.16.9-1.21.4");
        assert!(fabric_version_dir.exists(), "fabric version dir should exist");

        // 2. Remove Fabric
        svc.remove_loader(&paths, slug).await.unwrap();

        let m = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        assert!(m.loader.is_none(), "loader should be cleared after remove");
        assert!(
            !fabric_version_dir.exists(),
            "fabric version dir should be removed"
        );

        // 3. Install Quilt
        let (tx, _rx) = mpsc::channel(64);
        svc.install_loader(
            &paths,
            slug,
            "1.21.4",
            LoaderType::Quilt,
            "0.30.0-beta.7",
            std::path::Path::new("."),
            tx,
            CancellationToken::new(),
            crate::tasks::JobId(1),
        )
        .await
        .unwrap();

        let m = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        let loader = m.loader.unwrap();
        assert_eq!(loader.kind, ModloaderKind::Quilt);
        assert_eq!(loader.version_id, "quilt-loader-0.30.0-beta.7-1.21.4");

        // Quilt version JSON written
        let quilt_version_dir =
            paths.versions_dir().join("quilt-loader-0.30.0-beta.7-1.21.4");
        assert!(
            quilt_version_dir.exists(),
            "quilt version dir should exist"
        );
        // Old Fabric dir still gone
        assert!(
            !fabric_version_dir.exists(),
            "fabric version dir should still be gone"
        );
    }
}

// -----------------------------------------------------------------
// Forge/NeoForge-specific tests
// -----------------------------------------------------------------

#[cfg(test)]
mod forge_neoforge_tests {
    use super::*;
    use crate::loader::forge_meta::ForgeMetaClient;
    use crate::loader::neoforge_meta::NeoForgeMetaClient;
    use tempfile::TempDir;

    fn make_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    async fn write_vanilla_instance(paths: &AppPaths, slug: &str, mc: &str) {
        use crate::domain::instance::InstanceManifest;
        let m = InstanceManifest::new(slug.into(), slug.into(), mc.into());
        crate::instance::store::write_instance_manifest(paths, &m)
            .await
            .unwrap();
    }

    fn make_svc() -> LoaderService {
        let fabric = crate::loader::fabric::FabricMetaClient::new().unwrap();
        let quilt = crate::loader::quilt::QuiltMetaClient::new().unwrap();
        let forge = ForgeMetaClient::new().unwrap();
        let neoforge = NeoForgeMetaClient::new().unwrap();
        LoaderService::with_clients(fabric, quilt, forge, neoforge)
    }

    /// Re-attach: predicted version JSON on disk with empty libraries.
    /// Subprocess must NOT be invoked; manifest gets loader set.
    #[tokio::test]
    async fn test_forge_reattach_skip_writes_manifest_without_subprocess() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let slug = "reattach";
        let mc = "1.20.1";
        let loader_version = "47.4.20";
        let expected_id = format!("{mc}-forge-{loader_version}");

        write_vanilla_instance(&paths, slug, mc).await;

        // Pre-create the predicted version JSON with empty libraries.
        let json_path = paths.version_json(&expected_id);
        tokio::fs::create_dir_all(json_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &json_path,
            format!(r#"{{"id":"{expected_id}","libraries":[]}}"#).as_bytes(),
        )
        .await
        .unwrap();

        let svc = make_svc();
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        // jre_path is irrelevant — re-attach short-circuits before subprocess.
        let bogus_jre = std::path::PathBuf::from("/does/not/exist/java");

        let r = svc
            .install_loader(
                &paths,
                slug,
                mc,
                LoaderType::Forge,
                loader_version,
                &bogus_jre,
                tx,
                token,
                crate::tasks::JobId(0),
            )
            .await;
        assert!(r.is_ok(), "re-attach should succeed: {r:?}");

        // Manifest should now have loader set to the expected id.
        let m = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        assert_eq!(
            m.loader.as_ref().map(|l| l.version_id.clone()),
            Some(expected_id.clone()),
            "version_id must match"
        );
        assert_eq!(
            m.loader.as_ref().map(|l| l.kind),
            Some(ModloaderKind::Forge),
            "kind must be Forge"
        );
    }

    /// Cancel before subprocess: manifest MUST NOT have loader set.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_cancel_does_not_set_manifest_loader() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let slug = "cancel-atomicity";
        let mc = "1.20.1";

        write_vanilla_instance(&paths, slug, mc).await;

        let svc = make_svc();
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        token.cancel(); // cancel immediately — before any network or FS

        let bogus_jre = std::path::PathBuf::from("/bin/sh");
        let r = svc
            .install_loader(
                &paths,
                slug,
                mc,
                LoaderType::Forge,
                "47.4.20",
                &bogus_jre,
                tx,
                token,
                crate::tasks::JobId(0),
            )
            .await;
        // Expect Cancelled or an error (download might fail before cancel is checked)
        assert!(
            r.is_err(),
            "expected error after cancel, got: {r:?}"
        );

        // Atomicity invariant: manifest loader must stay None.
        let m = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        assert!(
            m.loader.is_none(),
            "atomicity broken: loader should be None after cancel, got: {:?}",
            m.loader
        );
    }

    /// Cancel + subprocess path: pre-cache installer JAR to skip download step,
    /// use /bin/sh as JRE (will fail fast), verify manifest stays untouched and
    /// staging dir is cleaned.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_cancel_kills_subprocess_and_cleans_staging() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let slug = "cancel-subproc";
        let mc = "1.20.1";
        let loader_v = "47.4.20";

        write_vanilla_instance(&paths, slug, mc).await;

        // Pre-create vanilla version JSON so staging populate_vanilla doesn't fail.
        let vj_path = paths.version_json(mc);
        tokio::fs::create_dir_all(vj_path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&vj_path, br#"{"id":"1.20.1"}"#).await.unwrap();
        // Pre-create vanilla JAR (also needed by populate_vanilla).
        let vjar_path = paths.version_jar(mc);
        tokio::fs::create_dir_all(vjar_path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&vjar_path, b"FAKE_JAR").await.unwrap();

        // Pre-cache the installer JAR so the download step short-circuits.
        let installer = paths
            .cache_dir
            .join("installers")
            .join(format!("forge-{mc}-{loader_v}-installer.jar"));
        tokio::fs::create_dir_all(installer.parent().unwrap()).await.unwrap();
        tokio::fs::write(&installer, b"PK\x03\x04FAKE").await.unwrap();

        let svc = make_svc();
        let (tx, _rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let token_c = token.clone();
        // Cancel after a short delay to let the subprocess attempt to start.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            token_c.cancel();
        });

        // Using /bin/sh as jre — it will fail when given JVM args (non-zero exit
        // or immediate error). Either way the install fails before writing manifest.
        let bogus_jre = std::path::PathBuf::from("/bin/sh");
        let r = svc
            .install_loader(
                &paths,
                slug,
                mc,
                LoaderType::Forge,
                loader_v,
                &bogus_jre,
                tx,
                token,
                crate::tasks::JobId(0),
            )
            .await;
        assert!(r.is_err(), "expected install failure (sh != java): {r:?}");

        // Atomicity invariant: manifest loader must stay None.
        let m = crate::instance::store::read_instance_manifest(&paths, slug)
            .await
            .unwrap();
        assert!(
            m.loader.is_none(),
            "manifest loader set after failed install: {:?}",
            m.loader
        );

        // Staging dir should be cleaned (best-effort — might linger on some
        // error paths, so tolerate empty dir too).
        let staging_base = paths.data_dir.join("staging");
        if staging_base.exists() {
            let mut entries = tokio::fs::read_dir(&staging_base).await.unwrap();
            let first = entries.next_entry().await.unwrap();
            assert!(
                first.is_none(),
                "staging dir should be cleaned after failed install"
            );
        }
    }
}
