//! Live Forge install smoke test -- `#[ignore]`-gated.
//!
//! Run with: `cargo test --test loader_forge_live -- --ignored --nocapture`
//!
//! Hits real endpoints:
//!   - https://piston-meta.mojang.com (Mojang version manifest, JRE manifest)
//!   - https://launcher.mojang.com (Mojang JRE binaries)
//!   - https://maven.minecraftforge.net (Forge maven-metadata.xml + installer JAR)
//!   - https://files.minecraftforge.net (Forge promotions_slim.json)
//!   - https://piston-data.mojang.com / libraries.minecraft.net (vanilla libs)
//!   - Forge libraries Maven mirrors
//!
//! Total runtime ~60-180s on a fast connection; downloads ~150MB.

use std::sync::Arc;

use ichr::domain::instance::{InstanceManifest, ModloaderKind};
use ichr::install::version_installer::install_version;
use ichr::java::service::JavaService;
use ichr::loader::service::LoaderService;
use ichr::loader::types::LoaderType;
use ichr::mojang::client::MojangClient;
use ichr::persistence::paths::AppPaths;
use ichr::tasks::{JobId, TaskEvent};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn make_paths(td: &TempDir) -> AppPaths {
    AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    )
}

fn make_progress_drain(label: &'static str) -> mpsc::Sender<TaskEvent> {
    let (tx, mut rx) = mpsc::channel::<TaskEvent>(256);
    tokio::spawn(async move {
        while let Some(evt) = rx.recv().await {
            if let TaskEvent::Progress { pct, msg, .. } = evt {
                println!("[{label}] progress: {pct:>3}%  {msg}");
            }
        }
    });
    tx
}

#[tokio::test]
#[ignore = "requires internet access -- see module docs"]
async fn live_forge_install_1_20_1() {
    let td = TempDir::new().expect("tempdir");
    let paths = make_paths(&td);
    let slug = "forge-live";
    let mc = "1.20.1";

    // 1) Write vanilla InstanceManifest
    let m = InstanceManifest::new(slug.into(), slug.into(), mc.into());
    ichr::instance::store::write_instance_manifest(&paths, &m)
        .await
        .expect("write_instance_manifest");

    // 2) Pre-fetch vanilla MC via the canonical install_version helper.
    //    Mirrors tests/version_install.rs:269 call shape exactly.
    println!("[forge_live] pre-fetching vanilla MC {mc}...");
    let mojang = MojangClient::new().expect("MojangClient::new");
    let manifest = mojang
        .fetch_manifest(&paths.cache_dir.join("manifest_v2.json"))
        .await
        .expect("fetch_manifest");
    let version_entry = manifest
        .versions
        .iter()
        .find(|v| v.id == mc)
        .expect("MC 1.20.1 present in version manifest")
        .clone();

    let (vt_tx, _vt_rx) = mpsc::channel::<TaskEvent>(256);
    let vanilla_token = CancellationToken::new();
    install_version(
        JobId(99),
        &paths,
        &mojang,
        vt_tx,
        vanilla_token,
        slug,
        &version_entry,
    )
    .await
    .expect("install_version vanilla MC 1.20.1");
    assert!(
        paths.version_json(mc).is_file(),
        "vanilla version JSON missing after install_version"
    );
    assert!(
        paths.version_jar(mc).is_file(),
        "vanilla client.jar missing after install_version"
    );

    // 3) Install JRE for MC 1.20.1 (Java 17 = java-runtime-gamma)
    println!("[forge_live] installing Mojang JRE java-runtime-gamma...");
    let java_svc = JavaService::new().expect("JavaService::new");
    let _ = java_svc
        .install_mojang(&paths, "java-runtime-gamma")
        .await
        .expect("install_mojang java-runtime-gamma");

    // 4) Resolve JRE path
    let jre_path = java_svc
        .resolve_jre_for_mc_version_install(&paths, mc)
        .await
        .expect("resolve_jre_for_mc_version_install");
    println!("[forge_live] using JRE: {}", jre_path.display());

    // 5) Pick Forge loader version dynamically
    let svc = Arc::new(LoaderService::new().expect("LoaderService::new"));
    let versions = svc
        .list_loader_versions(LoaderType::Forge, mc)
        .await
        .expect("list_loader_versions Forge");
    assert!(
        !versions.is_empty(),
        "Forge must publish loaders for MC {mc}"
    );
    let loader_version = versions
        .iter()
        .find(|v| v.stable)
        .map(|v| v.version.clone())
        .unwrap_or_else(|| versions[0].version.clone());
    println!("[forge_live] picked Forge {loader_version}");

    // 6) Run install
    let progress = make_progress_drain("forge_live");
    let token = CancellationToken::new();
    svc.install_loader(
        &paths,
        slug,
        mc,
        LoaderType::Forge,
        &loader_version,
        &jre_path,
        progress,
        token,
        JobId(0),
    )
    .await
    .expect("install_loader Forge");

    // 7) Assertions
    let m = ichr::instance::store::read_instance_manifest(&paths, slug)
        .await
        .expect("read manifest");
    let loader = m.loader.expect("manifest.loader must be Some");
    assert_eq!(loader.kind, ModloaderKind::Forge);
    assert_eq!(loader.version, loader_version);
    assert!(
        !loader.version_id.is_empty(),
        "version_id must be non-empty"
    );

    let vj = paths.version_json(&loader.version_id);
    assert!(vj.is_file(), "version JSON missing: {}", vj.display());

    // Staging dir cleaned
    let staging_dir = paths.data_dir.join("staging");
    if staging_dir.is_dir() {
        let mut entries = tokio::fs::read_dir(&staging_dir)
            .await
            .expect("read staging");
        assert!(
            entries.next_entry().await.expect("staging entry").is_none(),
            "staging dir not cleaned: {}",
            staging_dir.display()
        );
    }

    // At least one Forge library present
    let forge_libs_dir = paths.libraries_dir().join("net").join("minecraftforge");
    assert!(
        forge_libs_dir.is_dir(),
        "Forge libraries dir missing: {}",
        forge_libs_dir.display()
    );

    // Per-install log file present
    let instance_dir = paths.instance_dir(slug);
    let mut found_log = false;
    if let Ok(mut entries) = tokio::fs::read_dir(&instance_dir).await {
        while let Some(entry) = entries.next_entry().await.expect("read instance dir") {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("loader-install-")
            {
                found_log = true;
                break;
            }
        }
    }
    assert!(
        found_log,
        "no loader-install-*.log file in {}",
        instance_dir.display()
    );

    println!(
        "[forge_live] SUCCESS -- installed Forge {} (id={})",
        loader_version, loader.version_id
    );
}

/// GAP-7-A umbrella regression -- fast-fail live test that guards against
/// any ForgeWrapper install-blocker (class load, entry point, or class
/// resolution failure).
///
/// Three historical signatures detected (any one of which fires the banner):
///   - round-1 (pre-07.1-02): `NoClassDefFoundError: cpw/mods/modlauncher/Launcher`
///     -- Main was invoked without the three -Dforgewrapper.* properties so
///     MultiMCFileDetector.getLibraryDir fell back to introspecting modlauncher.
///   - round-2 (post-07.1-02 / pre-07.2-01): `Main method not found in
///     class io.github.zekerzhayard.forgewrapper.installer.Installer`
///     -- argv class swapped to Installer (a library class with no main()).
///   - any future regression that reverts back to either of the above
///     (or invents a new ForgeWrapper class-related blocker).
///
/// Post-07.2-01: the install proceeds normally; the test passes once the
/// subprocess exits 0 and the manifest's `loader.version_id` is non-empty.
///
/// This test is FASTER than `live_forge_install_1_20_1` because it asserts
/// EARLY behaviour (subprocess class load / entry point) rather than full
/// pipeline, but it is NOT a substitute -- `live_forge_install_1_20_1`
/// exercises full install + harvest + manifest + library copy. Both are
/// `#[ignore]`-gated; both run during HUMAN-UAT.
#[tokio::test]
#[ignore = "requires internet access -- see module docs"]
async fn live_forge_install_does_not_throw_install_blocker() {
    let td = TempDir::new().expect("tempdir");
    let paths = make_paths(&td);
    let slug = "forge-install-blocker-canary";
    let mc = "1.20.1";

    // 1) Write vanilla InstanceManifest
    let m = InstanceManifest::new(slug.into(), slug.into(), mc.into());
    ichr::instance::store::write_instance_manifest(&paths, &m)
        .await
        .expect("write manifest");

    // 2) Pre-fetch vanilla MC
    let mojang = MojangClient::new().expect("MojangClient::new");
    let manifest = mojang
        .fetch_manifest(&paths.cache_dir.join("manifest_v2.json"))
        .await
        .expect("fetch_manifest");
    let version_entry = manifest
        .versions
        .iter()
        .find(|v| v.id == mc)
        .expect("MC 1.20.1 in manifest")
        .clone();
    let (vt_tx, _vt_rx) = mpsc::channel::<TaskEvent>(256);
    install_version(
        JobId(99),
        &paths,
        &mojang,
        vt_tx,
        CancellationToken::new(),
        slug,
        &version_entry,
    )
    .await
    .expect("install_version vanilla");

    // 3) JRE
    let java_svc = JavaService::new().expect("JavaService::new");
    let _ = java_svc
        .install_mojang(&paths, "java-runtime-gamma")
        .await
        .expect("install_mojang java-runtime-gamma");
    let jre_path = java_svc
        .resolve_jre_for_mc_version_install(&paths, mc)
        .await
        .expect("resolve JRE");

    // 4) Pick latest stable Forge
    let svc = Arc::new(LoaderService::new().expect("LoaderService::new"));
    let versions = svc
        .list_loader_versions(LoaderType::Forge, mc)
        .await
        .expect("list_loader_versions Forge");
    assert!(!versions.is_empty(), "Forge must publish loaders for {mc}");
    let loader_version = versions
        .iter()
        .find(|v| v.stable)
        .map(|v| v.version.clone())
        .unwrap_or_else(|| versions[0].version.clone());

    // 5) Run install. Pre-07.2-01 raises SubprocessExit { code: 1 } here
    //    with one of two banners (round-1 or round-2 of GAP-7-A);
    //    post-07.2-01 succeeds.
    let progress = make_progress_drain("forge_install_blocker");
    let r = svc
        .install_loader(
            &paths,
            slug,
            mc,
            LoaderType::Forge,
            &loader_version,
            &jre_path,
            progress,
            CancellationToken::new(),
            JobId(0),
        )
        .await;

    match &r {
        Err(e) => {
            let msg = e.to_string();
            // round-1 signature
            let r1_classdef = msg.contains("NoClassDefFoundError") || msg.contains("modlauncher");
            // round-2 signature
            let r2_no_main = msg.contains("Main method not found");
            // round-3 signature (GAP-7-A-v3): Main reaches its body but throws
            // IndexOutOfBoundsException at line 28 because argv is empty.
            // (`argsList.get(argsList.indexOf("--fml.mcVersion") + 1)`).
            let r3_main_iooobe = msg.contains("IndexOutOfBoundsException")
                && msg.contains("forgewrapper.installer.Main");
            // any forgewrapper class reference in an Err message is suspect
            let any_wrapper_class = msg.contains("forgewrapper.installer.Main")
                || msg.contains("forgewrapper.installer.Installer");
            if r1_classdef || r2_no_main || r3_main_iooobe || any_wrapper_class {
                panic!(
                    "GAP-7-A umbrella regression detected -- install subprocess \
                     threw a ForgeWrapper class-related blocker. Round-1 signal: \
                     NoClassDefFoundError/modlauncher = {r1_classdef}. Round-2 \
                     signal: 'Main method not found' = {r2_no_main}. Round-3 signal: \
                     IndexOutOfBoundsException + forgewrapper.installer.Main = \
                     {r3_main_iooobe}. Any-class signal: forgewrapper.installer.{{Main,Installer}} = \
                     {any_wrapper_class}. The structurally correct fix (07.3-01) is: do \
                     NOT route the install subprocess through ForgeWrapper at all. \
                     Verify (a) install_subprocess_loader Step 4 invokes the installer \
                     JAR directly via `java -Djava.awt.headless=true -jar <installer> \
                     <install_flag> <staging>` where install_flag is `--installClient` \
                     for Forge and `--install-client` for NeoForge, (b) NO -cp / NO \
                     FORGE_WRAPPER_MAIN_CLASS / NO -Dforgewrapper.* in the argv, and \
                     (c) staging.populate_vanilla + staging.write_launcher_profiles \
                     still run before the subprocess. Full error: {msg}"
                );
            }
            panic!("install_loader failed for non-GAP-7-A-v3 reason: {msg}");
        }
        Ok(()) => {
            println!("[forge_install_blocker] install_loader succeeded -- GAP-7-A umbrella closed");
        }
    }

    let m = ichr::instance::store::read_instance_manifest(&paths, slug)
        .await
        .expect("read manifest");
    let loader = m
        .loader
        .expect("manifest.loader must be Some after install");
    assert_eq!(loader.kind, ModloaderKind::Forge);
    assert!(
        !loader.version_id.is_empty(),
        "loader.version_id must be non-empty (GAP-7-A umbrella canary): {loader:?}"
    );
}
