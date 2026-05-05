//! Live Modrinth install smoke test — gated by `#[ignore]`.
//!
//! Run with: `cargo nextest run --run-ignored only -E 'test(modrinth_live)'`
//! (or `cargo test --test modrinth_live -- --ignored --nocapture`)
//!
//! Exercises the full Phase 8 chain end-to-end:
//!   search → list_versions → resolve_dependencies → install_mod_into_instance
//!   → ledger assertions → toggle disable → toggle re-enable → uninstall.
//!
//! Requires internet access (api.modrinth.com + cdn.modrinth.com).
//!
//! ASSUMPTION A2 from 08-RESEARCH.md verified live: the project User-Agent
//! does not get blocked by Modrinth (test fails with 403 if A2 is wrong).
//! ASSUMPTION A3 verified: resolve_dependencies returns at least one Required
//! dep for Sodium on Fabric 1.20.4 (Fabric API).

use mineltui::domain::instance::{InstanceManifest, ModloaderKind};
use mineltui::loader::types::LoaderInfo;
use mineltui::mods::service::ModrinthService;
use mineltui::mods::types::DepKind;
use mineltui::persistence::paths::AppPaths;
use mineltui::tasks::JobId;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires internet access — see module docs"]
async fn live_install_sodium_with_deps_fabric_1_20_4() {
    let td = TempDir::new().expect("TempDir");
    let paths = AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    );
    let slug = "live-sodium";
    let mc = "1.20.4";

    // Pre-populate the manifest with Fabric loader marker (we don't run Phase 6's
    // install_loader pipeline; we just need the manifest to claim Fabric so the
    // ModrinthService computes loaders=["fabric"] for Modrinth queries).
    let mut m = InstanceManifest::new(slug.into(), slug.into(), mc.into());
    let loader_info = LoaderInfo {
        kind: ModloaderKind::Fabric,
        version: "0.16.9".into(),
        version_id: "fabric-loader-0.16.9-1.20.4".into(),
    };
    m.loader = Some(loader_info.clone());
    mineltui::instance::store::write_instance_manifest(&paths, &m)
        .await
        .expect("write_instance_manifest");

    let svc = ModrinthService::new().expect("ModrinthService::new");

    // 1. Search for Sodium.
    let hits = svc
        .search("sodium", Some(mc), Some(&loader_info), Some(&paths), Some(slug))
        .await
        .expect("search");
    assert!(!hits.is_empty(), "search returned zero hits");
    let sodium = hits
        .iter()
        .find(|h| h.slug == "sodium")
        .expect("sodium not in search results");
    println!(
        "[modrinth_live] sodium project_id = {}",
        sodium.project_id
    );

    // 2. List versions for Sodium.
    let versions = svc
        .list_versions(&sodium.project_id, Some(mc), Some(&loader_info))
        .await
        .expect("list_versions");
    assert!(
        !versions.is_empty(),
        "no Sodium versions for 1.20.4 + fabric"
    );
    let chosen = versions.first().expect("at least one version");
    println!(
        "[modrinth_live] picked Sodium version_id = {}",
        chosen.version_id
    );

    // 3. Resolve dependencies.
    let graph = svc
        .resolve_dependencies(&paths, slug, &chosen.version_id, mc, Some(&loader_info))
        .await
        .expect("resolve_dependencies");
    let new_required: Vec<_> = graph
        .deps
        .iter()
        .filter(|d| matches!(d.kind, DepKind::Required) && d.is_new_download)
        .collect();
    assert!(
        !new_required.is_empty(),
        "Sodium should require at least one new dep (Fabric API) on 1.20.4 + fabric"
    );
    println!(
        "[modrinth_live] resolved {} new required deps (graph total {} bytes / {} files)",
        new_required.len(),
        graph.total_new_bytes,
        graph.total_new_files,
    );

    // 4. Install — drain progress events into /dev/null.
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let token = CancellationToken::new();

    svc.install_mod_into_instance(
        &paths,
        slug,
        &sodium.slug,
        &sodium.title,
        &graph.root,
        &graph,
        tx,
        token,
        JobId(0),
    )
    .await
    .expect("install_mod_into_instance");

    // 5. Assert ledger has both Sodium and Fabric API.
    let mods = svc
        .list_installed_mods(&paths, slug)
        .await
        .expect("list_installed_mods");
    println!("[modrinth_live] ledger has {} mods", mods.len());
    assert!(
        mods.iter()
            .any(|m| m.project_slug == "sodium" || m.mod_id == sodium.project_id),
        "Sodium missing from ledger"
    );
    assert!(
        mods.iter().any(|m| m.project_slug == "fabric-api"
            || m.display_name.to_lowercase().contains("fabric api")),
        "Fabric API missing from ledger"
    );

    // 6. Assert files exist on disk.
    for row in &mods {
        let path = paths.instance_mod_file(slug, &row.file_name);
        assert!(path.is_file(), "mod file missing: {}", path.display());
        println!("[modrinth_live] verified file: {}", path.display());
    }

    // 7. Toggle Sodium disabled → re-enabled.
    let sodium_id = mods
        .iter()
        .find(|m| m.project_slug == "sodium" || m.mod_id == sodium.project_id)
        .map(|m| m.mod_id.clone())
        .expect("sodium ledger row");
    svc.disable_mod(&paths, slug, &sodium_id)
        .await
        .expect("disable_mod");
    let post_disable = svc.list_installed_mods(&paths, slug).await.unwrap();
    let sodium_row = post_disable
        .iter()
        .find(|m| m.mod_id == sodium_id)
        .expect("sodium row after disable");
    assert!(!sodium_row.enabled, "sodium should be disabled");
    let dot_disabled =
        paths.instance_mod_file(slug, &format!("{}.disabled", sodium_row.file_name));
    assert!(
        dot_disabled.is_file(),
        "sodium .jar.disabled file missing at {}",
        dot_disabled.display()
    );

    svc.enable_mod(&paths, slug, &sodium_id)
        .await
        .expect("enable_mod");
    let post_enable = svc.list_installed_mods(&paths, slug).await.unwrap();
    let sodium_row = post_enable
        .iter()
        .find(|m| m.mod_id == sodium_id)
        .expect("sodium row after enable");
    assert!(sodium_row.enabled, "sodium should be re-enabled");
    assert!(
        paths
            .instance_mod_file(slug, &sodium_row.file_name)
            .is_file(),
        "sodium .jar should be back at original path"
    );

    // Capture file_name BEFORE uninstall (we need it to assert on disk after the
    // ledger row is gone).
    let sodium_file_name = sodium_row.file_name.clone();

    // 8. Uninstall Sodium → file gone, row removed, Fabric API still present.
    svc.uninstall_mod(&paths, slug, &sodium_id)
        .await
        .expect("uninstall_mod");
    let final_mods = svc.list_installed_mods(&paths, slug).await.unwrap();
    assert!(
        !final_mods.iter().any(|m| m.mod_id == sodium_id),
        "sodium row should be gone after uninstall"
    );
    assert!(
        final_mods.iter().any(|m| m.project_slug == "fabric-api"
            || m.display_name.to_lowercase().contains("fabric api")),
        "Fabric API should still be present after Sodium uninstall"
    );
    let removed_path = paths.instance_mod_file(slug, &sodium_file_name);
    assert!(
        !removed_path.is_file(),
        "sodium .jar should be gone after uninstall: {}",
        removed_path.display()
    );

    println!(
        "[modrinth_live] SUCCESS — installed {} mods, toggled, uninstalled cleanly",
        mods.len()
    );
}
