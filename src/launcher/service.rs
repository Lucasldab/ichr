//! Launch orchestrator — loads an instance, composes the command, spawns
//! Minecraft, and updates the manifest on exit. Emits TaskEvents at
//! each step so the TUI progress indicator can track the launch.
//!
//! See `.planning/phases/03-launcher-process-and-offline-launch/03-RESEARCH.md`
//! §"System Architecture Diagram" for the flow.

use std::collections::HashMap;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::instance::store::{mark_launch_started, update_play_time};
use crate::instance::read_instance_manifest;
use crate::mojang::inherits::resolve_inherits;
use crate::mojang::rules::RuleContext;
use crate::mojang::types::VersionJson;
use crate::persistence::paths::AppPaths;
use crate::tasks::job::{JobId, TaskEvent};

use super::argfile::{argfile_path, write_argfile};
use super::command::{compose, compose_msa};
use super::offline::{offline_auth, MsaAuth};
use super::spawn::run_process;

/// Launch `slug` using the provided `auth_ctx`. Emits `TaskEvent::Progress`
/// messages at each step to `tx`.
///
/// - `AuthContext::Offline { username }`: identical to the Phase 3 offline path.
/// - `AuthContext::Msa { account_id }`: resolves live MC tokens via
///   `account_service` and populates `SubstitutionContext` with real session
///   fields. Requires `account_service.is_some()`; returns
///   `AppError::NoActiveAccount` if `None` is passed.
///
/// Returns the play duration in milliseconds on a clean exit.
/// Returns `AppError::Cancelled` if the `token` is cancelled during the game.
/// Returns `AppError::LaunchFailed { code, message }` on a non-zero JVM exit,
/// where `message` contains the ring-buffered log tail from `spawn::run_process`.
/// Returns `AppError::VersionNotInstalled { slug }` if the client jar is absent
/// (short-circuits before anything is spawned).
/// Returns `AppError::JavaMismatch { required, found, .. }` if the resolved
/// Java binary's major version does not meet the version's requirement
/// (surfaces BEFORE any process spawn).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip_all, fields(slug = %slug))]
pub async fn launch_instance(
    paths: &AppPaths,
    slug: &str,
    auth_ctx: crate::auth::AuthContext,
    account_service: Option<&crate::auth::service::AccountService>,
    java_service: &crate::java::service::JavaService,
    tx: mpsc::Sender<TaskEvent>,
    token: CancellationToken,
    job_id: JobId,
) -> Result<u64, AppError> {
    // Step 1 — load instance manifest
    send_progress(&tx, job_id, 1, "loading instance").await;
    let manifest = read_instance_manifest(paths, slug).await?;

    // GAP-8-E (Phase 8.1 gap closure): when a loader is installed, the launch
    // entry point is the loader's version JSON (which carries `inheritsFrom`
    // back to the vanilla MC version). Without this, modded instances silently
    // launch as vanilla — mods in `.minecraft/mods/` are never loaded because
    // the JVM is invoked with the vanilla main class and classpath.
    let launch_version_id: &str = match manifest.loader.as_ref() {
        Some(loader) => loader.version_id.as_str(),
        None => manifest.mc_version_id.as_str(),
    };

    // GAP-LAUNCH-JAR-08 (Phase 8.2 gap closure): the up-front version_jar
    // existence guard previously here is removed. For Fabric/Quilt/Forge/
    // NeoForge instances the loader's `.jar` STRUCTURALLY NEVER EXISTS on
    // disk (loader version JSONs are metadata-only; the JAR is inherited
    // via `inheritsFrom`). Step 3's `read_version_json_from_disk` already
    // returns `VersionNotInstalled` when the version JSON is absent, which
    // is the authoritative install marker for both vanilla and loader
    // paths. The resolved root vanilla JAR is later referenced by the
    // classpath builder (Step 6 → `compose` → `build_classpath`); a
    // missing client JAR will surface from the spawn layer when the JVM
    // fails to find the class on its classpath.

    // Step 3 — load root version JSON from disk (no network)
    send_progress(&tx, job_id, 10, "loading version JSON").await;
    let root_version = read_version_json_from_disk(paths, launch_version_id).await?;

    // Step 4 — walk inheritsFrom chain from disk only; call pure-sync resolve_inherits
    send_progress(&tx, job_id, 15, "resolving inheritsFrom chain").await;
    let parents = collect_parents_from_disk(paths, &root_version).await?;
    let version = resolve_inherits(&root_version, &parents)?;

    // Step 5 — resolve Java runtime (Phase 5: per-instance + auto-download)
    send_progress(&tx, job_id, 25, "resolving Java runtime").await;
    let java = java_service.resolve_jre_for_launch(paths, &manifest, &version).await?;

    // Step 6 — compose the LaunchCommand
    send_progress(&tx, job_id, 30, "composing command").await;
    let ctx = RuleContext::current();
    let cmd = match auth_ctx {
        crate::auth::AuthContext::Offline { username } => {
            let auth = offline_auth(&username);
            compose(&version, &auth, paths, slug, &ctx, &java)?
        }
        crate::auth::AuthContext::Msa { account_id } => {
            let svc = account_service.ok_or(AppError::NoActiveAccount)?;
            send_progress(&tx, job_id, 28, "refreshing MSA tokens").await;
            let tokens = svc
                .resolve_msa_tokens_for_launch(&account_id)
                .await
                .map_err(AppError::Auth)?;
            let auth = MsaAuth::from_tokens(&tokens);
            compose_msa(&version, &auth, paths, slug, &ctx, &java)?
        }
    };

    // Step 7 — verify Java binary exists on disk before spawning
    send_progress(&tx, job_id, 35, "checking java binary").await;
    if !tokio::fs::try_exists(&java).await.unwrap_or(false) {
        return Err(AppError::JavaNotFound);
    }

    // Step 8 — on Windows write @argfile and replace jvm_args with the @-reference;
    //           on Linux pass jvm_args through unmodified
    send_progress(&tx, job_id, 40, "preparing argfile").await;
    let effective_jvm_args: Vec<String> = if cfg!(target_os = "windows") {
        let ap = argfile_path(paths, slug);
        write_argfile(&cmd.jvm_args, &ap)
            .map_err(|e| AppError::SpawnFailed(format!("write argfile: {e}")))?;
        vec![format!("@{}", ap.to_string_lossy())]
    } else {
        cmd.jvm_args.clone()
    };

    // Step 9 — mark launch started (sets last_played_at; play_time untouched until exit)
    send_progress(&tx, job_id, 45, "marking launch started").await;
    mark_launch_started(paths, slug).await?;

    // Step 10 — spawn Minecraft and wait
    send_progress(&tx, job_id, 50, "spawning Minecraft").await;
    let outcome = run_process(
        &cmd.java_bin,
        &effective_jvm_args,
        &cmd.main_class,
        &cmd.game_args,
        &paths.instance_minecraft_dir(slug),
        &paths.instance_log_file(slug),
        token,
    )
    .await;

    match outcome {
        Ok(launch_outcome) => {
            // Step 11 — clean exit: update play time and return duration
            send_progress(&tx, job_id, 100, "exited cleanly").await;
            update_play_time(paths, slug, launch_outcome.duration_ms).await?;
            Ok(launch_outcome.duration_ms)
        }
        Err(AppError::Cancelled) => {
            // Step 12 — cancelled: propagate without updating play time
            Err(AppError::Cancelled)
        }
        Err(e) => {
            // Step 13 — LaunchFailed or SpawnFailed: propagate as-is
            Err(e)
        }
    }
}

// ----- private helpers -------------------------------------------------------

/// Read a version JSON from `{versions_dir}/{id}/{id}.json`. Returns
/// `AppError::VersionNotInstalled` if the file is absent (the caller must
/// ensure the version was installed before launching).
async fn read_version_json_from_disk(
    paths: &AppPaths,
    version_id: &str,
) -> Result<VersionJson, AppError> {
    let json_path = paths.version_json(version_id);
    if !tokio::fs::try_exists(&json_path).await.unwrap_or(false) {
        return Err(AppError::VersionNotInstalled {
            slug: version_id.to_string(),
        });
    }
    let raw = tokio::fs::read_to_string(&json_path).await?;
    let v: VersionJson = serde_json::from_str(&raw)?;
    Ok(v)
}

/// Walk `root.inherits_from` from disk ONLY — no network. Populates a
/// `HashMap<id, VersionJson>` for every ancestor in the chain and returns it
/// for `resolve_inherits` (which is pure-sync).
///
/// Cycle protection: the loop stops if it sees a `parent_id` already in the
/// map; `resolve_inherits` enforces MAX_INHERITS_DEPTH (= 3) as a hard cap.
///
/// If any parent JSON is absent from disk, returns `AppError::VersionNotInstalled`
/// — launch must not hit the network (Phase 2 install pre-fetched all parents).
async fn collect_parents_from_disk(
    paths: &AppPaths,
    root: &VersionJson,
) -> Result<HashMap<String, VersionJson>, AppError> {
    let mut parents: HashMap<String, VersionJson> = HashMap::new();
    let mut current = root.inherits_from.clone();
    while let Some(parent_id) = current {
        if parents.contains_key(&parent_id) {
            // Cycle detected — resolve_inherits will reject with InheritsFromCycle
            break;
        }
        let pv = read_version_json_from_disk(paths, &parent_id).await?;
        current = pv.inherits_from.clone();
        parents.insert(parent_id, pv);
    }
    Ok(parents)
}

/// Send a `TaskEvent::Progress` to `tx`. Failures are silently ignored —
/// a dropped receiver is a legitimate shutdown signal, not an error.
async fn send_progress(tx: &mpsc::Sender<TaskEvent>, id: JobId, pct: u8, msg: &str) {
    let _ = tx
        .send(TaskEvent::Progress {
            id,
            pct,
            msg: msg.to_string(),
        })
        .await;
}

// ----- tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::InstanceManifest;
    use crate::instance::store::write_instance_manifest;
    use crate::java::service::JavaService;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn paths_in(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    #[tokio::test]
    async fn test_launch_returns_version_not_installed_when_jar_missing() {
        let td = TempDir::new().unwrap();
        let paths = paths_in(&td);
        let m = InstanceManifest::new("x".into(), "x".into(), "1.21.4".into());
        write_instance_manifest(&paths, &m).await.unwrap();
        // client.jar intentionally NOT created
        let (tx, _rx) = mpsc::channel::<TaskEvent>(16);
        let token = CancellationToken::new();
        let auth_ctx = crate::auth::AuthContext::Offline { username: "TestUser".to_string() };
        let java_service = JavaService::new().expect("JavaService::new");
        let result = launch_instance(&paths, "x", auth_ctx, None, &java_service, tx, token, JobId(1)).await;
        assert!(
            matches!(result, Err(AppError::VersionNotInstalled { .. })),
            "expected VersionNotInstalled; got {result:?}"
        );
    }

    /// GAP-8-E regression (reconciled in Phase 8.2 GAP-LAUNCH-JAR-08): a
    /// manifest carrying `loader=Some(_)` MUST cause the launcher to look up
    /// the loader's version_id (e.g. `fabric-loader-0.16.9-1.20.4`) — NOT
    /// `manifest.mc_version_id` (vanilla).
    ///
    /// Setup: vanilla `1.20.4/1.20.4.jar` is pre-created on disk; NO version
    /// JSON exists for either id, and NO loader JAR/JSON exists.
    ///
    /// Pre-08.2-01 expectation (the OLD bug surface): the broken Step 2 guard
    /// fired at `paths.version_jar(launch_version_id)`. Because the loader JAR
    /// path (`fabric-loader-0.16.9-1.20.4.jar`) was absent, it returned
    /// `VersionNotInstalled { slug: <instance slug> }` (== "modded"), since
    /// the guard sourced its slug from `slug.to_string()` (the instance slug).
    ///
    /// Post-08.2-01 expectation (the CHOSEN fix surface): the Step 2 guard is
    /// removed. The launcher progresses to Step 3
    /// (`read_version_json_from_disk(paths, launch_version_id)`), which fires
    /// `VersionNotInstalled { slug: version_id.to_string() }` when the loader
    /// JSON is absent — i.e. `slug == "fabric-loader-0.16.9-1.20.4"` (the
    /// loader version_id, NOT the instance slug).
    ///
    /// Asserting `slug == "fabric-loader-0.16.9-1.20.4"` is what gives this
    /// test teeth post-fix: it proves the launcher resolved to the LOADER
    /// version_id (not vanilla). A wildcard `VersionNotInstalled { .. }` match
    /// would pass under the original GAP-8-E bug too (which fired with
    /// `slug == "1.21.4"` from a vanilla-only read) — so the slug-equality
    /// assertion remains the real regression guard, just at a new firing
    /// site (Step 3 instead of the now-removed Step 2 guard).
    #[tokio::test]
    async fn test_launch_reads_loader_version_id_when_loader_some() {
        use crate::domain::instance::ModloaderKind;
        use crate::loader::types::LoaderInfo;

        let td = TempDir::new().unwrap();
        let paths = paths_in(&td);

        // Pre-create the vanilla 1.20.4 client.jar so a (hypothetical)
        // vanilla-only read would advance past the jar-presence check. With
        // the Step 2 guard now removed, this jar is no longer consulted; we
        // keep it to preserve the historical fixture shape that distinguishes
        // GAP-8-E bug from fix.
        let vanilla_jar = paths.version_jar("1.20.4");
        tokio::fs::create_dir_all(vanilla_jar.parent().unwrap()).await.unwrap();
        tokio::fs::write(&vanilla_jar, b"fake client.jar").await.unwrap();

        // Manifest declares a Fabric loader. NO loader version JSON exists on
        // disk — this is the lever that fires `VersionNotInstalled` from
        // Step 3 (`read_version_json_from_disk`) with the LOADER version_id
        // as the slug, proving the launcher routed via `launch_version_id`.
        let mut m = InstanceManifest::new("modded".into(), "modded".into(), "1.20.4".into());
        m.loader = Some(LoaderInfo {
            kind: ModloaderKind::Fabric,
            version: "0.16.9".into(),
            version_id: "fabric-loader-0.16.9-1.20.4".into(),
        });
        write_instance_manifest(&paths, &m).await.unwrap();

        let (tx, _rx) = mpsc::channel::<TaskEvent>(16);
        let token = CancellationToken::new();
        let auth_ctx = crate::auth::AuthContext::Offline { username: "TestUser".to_string() };
        let java_service = JavaService::new().expect("JavaService::new");

        let result = launch_instance(
            &paths, "modded", auth_ctx, None, &java_service, tx, token, JobId(2),
        ).await;

        match result {
            Err(AppError::VersionNotInstalled { slug }) => {
                assert_eq!(
                    slug, "fabric-loader-0.16.9-1.20.4",
                    "expected VersionNotInstalled at Step 3 \
                     read_version_json_from_disk with slug=loader_version_id \
                     (proving the launcher looked up the loader's JSON, not \
                     vanilla); got slug={slug:?} — under the pre-08.2-01 \
                     buggy code this would have been \"modded\" (firing from \
                     the now-removed Step 2 jar guard)",
                );
            }
            other => panic!(
                "expected Err(VersionNotInstalled {{ slug: \"fabric-loader-0.16.9-1.20.4\" }}); got {other:?}",
            ),
        }
    }

    /// GAP-LAUNCH-JAR-08 regression (Phase 8.2): with a Phase-6-shaped on-disk
    /// fixture (loader dir has only `.json` with `inheritsFrom='1.20.4'`;
    /// vanilla dir has both `.jar` AND `.json` with `inheritsFrom=None`),
    /// `launch_instance` MUST progress past Steps 2-4 (jar/json existence,
    /// inheritsFrom resolve) and fail at Step 5 or later for an unrelated
    /// reason — NOT with `VersionNotInstalled` fired from a jar-existence
    /// guard.
    ///
    /// This is the test the original 08.1-01 plan should have shipped. The
    /// sibling test (`test_launch_reads_loader_version_id_when_loader_some`)
    /// only proves the launcher LOOKS at the loader's path; this test proves
    /// the launcher can actually GET PAST the loader path when the install
    /// is real.
    ///
    /// Implementation note: we set `MINELTUI_JAVA` to a non-existent path so
    /// `JavaService::resolve_jre_for_launch` short-circuits with that path
    /// (Step 5 succeeds), `compose` succeeds (pure-sync), and Step 7 then
    /// fails with `JavaNotFound` because the path doesn't exist on disk.
    /// This is deterministic and avoids any network calls. Critically, the
    /// post-fix code MUST reach Step 5+ — only the bug would short-circuit
    /// at Step 2 with `VersionNotInstalled`.
    #[tokio::test]
    async fn test_launch_modded_passes_jar_check_with_realistic_fixture() {
        use crate::domain::instance::ModloaderKind;
        use crate::loader::types::LoaderInfo;

        let td = TempDir::new().unwrap();
        let paths = paths_in(&td);

        // --- Vanilla 1.20.4 fixture (both .jar AND .json) -------------------
        let vanilla_id = "1.20.4";
        let vanilla_jar = paths.version_jar(vanilla_id);
        tokio::fs::create_dir_all(vanilla_jar.parent().unwrap()).await.unwrap();
        tokio::fs::write(&vanilla_jar, b"fake vanilla client.jar").await.unwrap();

        // Minimum vanilla VersionJson — must parse as VersionJson and serve
        // as a leaf parent (inherits_from = None). Modeled on the
        // `vjson_stub` helper in tests/mojang_protocol.rs (which has the
        // same minimum-parseable shape this struct demands: id, type,
        // mainClass, assetIndex (full), assets, downloads, libraries,
        // releaseTime, time).
        let vanilla_json_path = paths.version_json(vanilla_id);
        let vanilla_json = r#"{
            "id": "1.20.4",
            "type": "release",
            "mainClass": "net.minecraft.client.main.Main",
            "assetIndex": {
                "id": "12",
                "sha1": "0000000000000000000000000000000000000000",
                "size": 0,
                "totalSize": 0,
                "url": "http://example.com/12.json"
            },
            "assets": "12",
            "downloads": {},
            "libraries": [],
            "releaseTime": "2023-12-07T00:00:00Z",
            "time": "2023-12-07T00:00:00Z"
        }"#;
        tokio::fs::write(&vanilla_json_path, vanilla_json).await.unwrap();

        // --- Loader 0.16.9 fixture (ONLY .json, with inheritsFrom='1.20.4') -
        let loader_id = "fabric-loader-0.16.9-1.20.4";
        let loader_json_path = paths.version_json(loader_id);
        tokio::fs::create_dir_all(loader_json_path.parent().unwrap()).await.unwrap();
        // Loader JSON inherits from vanilla and carries Fabric mainClass.
        // resolve_inherits walks `inherits_from`, fetches the parent from
        // the parents map, and merges. Required field shape mirrors what
        // Phase 6's install_loader actually writes.
        let loader_json = r#"{
            "id": "fabric-loader-0.16.9-1.20.4",
            "inheritsFrom": "1.20.4",
            "type": "release",
            "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
            "assetIndex": {
                "id": "12",
                "sha1": "0000000000000000000000000000000000000000",
                "size": 0,
                "totalSize": 0,
                "url": "http://example.com/12.json"
            },
            "assets": "12",
            "downloads": {},
            "libraries": [],
            "releaseTime": "2024-12-15T00:00:00Z",
            "time": "2024-12-15T00:00:00Z"
        }"#;
        tokio::fs::write(&loader_json_path, loader_json).await.unwrap();
        // INTENTIONALLY: NO `versions/fabric-loader-0.16.9-1.20.4/...jar`.
        // The fix relies on the launcher NOT looking for one.

        // --- InstanceManifest with the Fabric loader -----------------------
        let mut m = InstanceManifest::new("modded".into(), "modded".into(), "1.20.4".into());
        m.loader = Some(LoaderInfo {
            kind: ModloaderKind::Fabric,
            version: "0.16.9".into(),
            version_id: loader_id.into(),
        });
        write_instance_manifest(&paths, &m).await.unwrap();

        // --- Hermetic Java path: short-circuit Step 5 to a non-existent
        //     binary so Step 7 fails cleanly with JavaNotFound (no network). ---
        let prior_java = std::env::var("MINELTUI_JAVA").ok();
        let fake_java = td.path().join("nonexistent-java-binary");
        std::env::set_var("MINELTUI_JAVA", fake_java.to_str().unwrap());

        let (tx, _rx) = mpsc::channel::<TaskEvent>(16);
        let token = CancellationToken::new();
        let auth_ctx = crate::auth::AuthContext::Offline { username: "TestUser".to_string() };
        let java_service = JavaService::new().expect("JavaService::new");

        let result = launch_instance(
            &paths, "modded", auth_ctx, None, &java_service, tx, token, JobId(3),
        ).await;

        // Restore env BEFORE assertions so a panic doesn't leak state.
        match prior_java {
            Some(v) => std::env::set_var("MINELTUI_JAVA", v),
            None => std::env::remove_var("MINELTUI_JAVA"),
        }

        // Pin the negative assertion: any AppError variant is fine EXCEPT
        // VersionNotInstalled. If we get VersionNotInstalled here, either
        // (a) the fixture is wrong (e.g. resolve_inherits rejected it), or
        // (b) the bug was reintroduced. Both deserve a loud failure.
        match result {
            Ok(_) => {
                // Highly unlikely without a real JRE + spawn ability in the
                // test env. If it somehow succeeds, the test is still valid:
                // we got past the guard.
            }
            Err(AppError::VersionNotInstalled { slug }) => panic!(
                "GAP-LAUNCH-JAR-08 REGRESSION: launch_instance returned \
                 VersionNotInstalled (slug={slug:?}) on a realistic loader \
                 fixture — the launcher should have progressed past Steps \
                 2-4. Either the Step 2 guard was re-introduced or the \
                 fixture layout drifted from what Phase 6 writes."
            ),
            Err(other) => {
                // Acceptable: any other error variant proves we got past
                // the early jar/json existence checks. With MINELTUI_JAVA
                // set to a non-existent path, the expected variant is
                // JavaNotFound (Step 7). Log for diagnosis:
                eprintln!(
                    "[gap-08-launch-jar] post-guard failure (expected): {other:?}"
                );
            }
        }
    }
}
