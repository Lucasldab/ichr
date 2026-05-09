//! Live CurseForge install smoke test -- gated by `#[ignore]`.
//!
//! Run with: `cargo nextest run --run-ignored only -E 'test(curseforge_live)'`
//! (or `cargo test --test curseforge_live -- --ignored --nocapture`)
//!
//! Exercises the full Phase 9 chain end-to-end:
//!   search → get_mod → list_files → install_mod_into_instance → ledger assertions.
//!
//! Requires:
//!   - Internet access (api.curseforge.com + edge.forgecdn.net)
//!   - `CURSEFORGE_API_KEY` env var set (build-time
//!     `ICHR_CURSEFORGE_API_KEY_DEFAULT` also works in principle but is
//!     NOT assumed for CI test runs -- the runtime env var is the canonical
//!     test-time secret).
//!
//! Skip behavior: per 09-RESEARCH.md §Open Questions Q10 (line 1346), missing
//! `CURSEFORGE_API_KEY` results in `eprintln!` + early `return` (NOT panic).
//! This protects CI from breaking when secrets rotate or the build host has
//! no API key configured.

use ichr::domain::instance::{InstanceManifest, ModloaderKind};
use ichr::loader::types::LoaderInfo;
use ichr::mods::curseforge::error::CurseForgeError;
use ichr::mods::curseforge::CurseForgeService;
use ichr::mods::types::{HashAlgo, ModSource};
use ichr::persistence::paths::AppPaths;
use ichr::tasks::JobId;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Skip-gate for the CURSEFORGE_API_KEY env var. Returns true when the test
/// MUST be skipped (key absent or empty); prints the skip reason to stderr
/// per Q10. Never echoes the key value.
fn skip_if_no_api_key(test_name: &str) -> bool {
    match std::env::var("CURSEFORGE_API_KEY") {
        Ok(v) if !v.is_empty() => false,
        _ => {
            eprintln!("[curseforge_live] SKIPPED {test_name} -- CURSEFORGE_API_KEY not set");
            true
        }
    }
}

fn fabric_loader() -> LoaderInfo {
    LoaderInfo {
        kind: ModloaderKind::Fabric,
        version: "0.16.9".into(),
        version_id: "fabric-loader-0.16.9-1.20.4".into(),
    }
}

#[tokio::test]
#[ignore = "requires internet + CURSEFORGE_API_KEY env var -- see module docs"]
async fn live_install_downloadable_mod_fabric_1_20_4() {
    if skip_if_no_api_key("live_install_downloadable_mod_fabric_1_20_4") {
        return;
    }

    let td = TempDir::new().expect("TempDir");
    let paths = AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    );
    let slug = "live-cf-sodium";
    let mc = "1.20.4";

    // Pre-populate the manifest with a Fabric loader marker (we don't run
    // Phase 6's install_loader pipeline; we just need the manifest to claim
    // Fabric so CurseForgeService computes modLoaderType=4 for queries).
    let mut m = InstanceManifest::new(slug.into(), slug.into(), mc.into());
    let loader_info = fabric_loader();
    m.loader = Some(loader_info.clone());
    ichr::instance::store::write_instance_manifest(&paths, &m)
        .await
        .expect("write_instance_manifest");

    let svc = CurseForgeService::new().expect("CurseForgeService::new");
    assert!(
        svc.api_key_present(),
        "expected api_key_present == true with CURSEFORGE_API_KEY env var set"
    );

    // 1. Search for Sodium.
    let hits = svc
        .search(
            "sodium",
            Some(mc),
            Some(&loader_info),
            Some(&paths),
            Some(slug),
        )
        .await
        .expect("search");
    assert!(!hits.is_empty(), "search returned zero hits");
    let sodium = hits
        .iter()
        .find(|h| h.slug.contains("sodium") || h.name.to_lowercase().contains("sodium"))
        .expect("sodium not in search results");
    println!(
        "[curseforge_live] sodium id = {}, slug = {}",
        sodium.id, sodium.slug
    );

    // 2. Get mod detail.
    let detail = svc.get_mod(sodium.id).await.expect("get_mod");
    assert_eq!(detail.id, sodium.id);

    // 3. List files for Sodium with MC + loader filters.
    let files = svc
        .list_files(sodium.id, Some(mc), Some(&loader_info))
        .await
        .expect("list_files");
    assert!(!files.is_empty(), "no Sodium files for 1.20.4 + Fabric");
    let chosen = files
        .iter()
        .find(|f| {
            f.download_url
                .as_deref()
                .filter(|u| !u.is_empty())
                .is_some()
        })
        .expect("expected at least one file with non-null download_url for Sodium");
    println!(
        "[curseforge_live] picked file id={}, name={}, sha1_present={}",
        chosen.id,
        chosen.file_name,
        chosen.hashes.iter().any(|h| h.algo == 1),
    );

    // 4. Install -- drain progress events into /dev/null.
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let token = CancellationToken::new();

    svc.install_mod_into_instance(&paths, slug, &detail, chosen, tx, token, JobId(0))
        .await
        .expect("install_mod_into_instance");

    // 5. Assert ledger row.
    let ledger_path = paths.instance_mod_ledger(slug);
    assert!(
        ledger_path.exists(),
        "ledger file missing at {}",
        ledger_path.display()
    );
    let ledger_raw = tokio::fs::read_to_string(&ledger_path).await.unwrap();
    let ledger: ichr::mods::types::Ledger = toml::from_str(&ledger_raw).unwrap();
    assert_eq!(ledger.mods.len(), 1, "expected exactly 1 ledger row");
    let row = &ledger.mods[0];
    assert_eq!(
        row.source,
        ModSource::CurseForge,
        "source must be CurseForge"
    );
    assert_eq!(row.hash_algo, HashAlgo::Sha1, "hash_algo must be Sha1");
    assert_eq!(row.mod_id, sodium.id.to_string());
    assert_eq!(row.version_id, chosen.id.to_string());
    assert_eq!(row.file_name, chosen.file_name);

    // 6. Assert file on disk.
    let file_path = paths.instance_mod_file(slug, &row.file_name);
    assert!(
        file_path.is_file(),
        "mod file missing: {}",
        file_path.display()
    );

    println!(
        "[curseforge_live] SUCCESS -- installed {} (file={}) into ledger with source=CurseForge, hash_algo=Sha1",
        sodium.name, row.file_name,
    );
}

#[tokio::test]
#[ignore = "requires internet + CURSEFORGE_API_KEY env var + a known-restricted mod -- see module docs"]
async fn live_restricted_mod_returns_file_not_downloadable() {
    if skip_if_no_api_key("live_restricted_mod_returns_file_not_downloadable") {
        return;
    }

    // OPERATOR MAINTENANCE NOTE: Update the candidate slug below to a current
    // CurseForge mod that has the "Distribution" toggle off (third-party
    // downloads disabled). If the candidate is no longer restricted, the test
    // will skip (rather than fail). Search the CurseForge browser for current
    // examples; historically `enchanted-book-redesign` and similar mods have
    // been restricted, but mod-author preferences change.
    const RESTRICTED_CANDIDATE_SLUG: &str = "enchanted-book-redesign";

    let td = TempDir::new().expect("TempDir");
    let paths = AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    );
    let slug = "live-cf-restricted";
    let mc = "1.20.4";
    let loader_info = fabric_loader();

    let mut m = InstanceManifest::new(slug.into(), slug.into(), mc.into());
    m.loader = Some(loader_info.clone());
    ichr::instance::store::write_instance_manifest(&paths, &m)
        .await
        .expect("write_instance_manifest");

    let svc = CurseForgeService::new().expect("CurseForgeService::new");

    let hits = svc
        .search(
            RESTRICTED_CANDIDATE_SLUG,
            Some(mc),
            Some(&loader_info),
            None,
            None,
        )
        .await
        .expect("search");
    let candidate = match hits.iter().find(|h| h.slug == RESTRICTED_CANDIDATE_SLUG) {
        Some(h) => h,
        None => {
            eprintln!(
                "[curseforge_live] SKIPPED restricted-mod test -- candidate slug '{RESTRICTED_CANDIDATE_SLUG}' not found in current CurseForge data; update RESTRICTED_CANDIDATE_SLUG to a current example"
            );
            return;
        }
    };

    let detail = svc.get_mod(candidate.id).await.expect("get_mod");
    let files = svc
        .list_files(candidate.id, Some(mc), Some(&loader_info))
        .await
        .expect("list_files");
    let restricted = match files.iter().find(|f| f.download_url.is_none()) {
        Some(f) => f,
        None => {
            eprintln!(
                "[curseforge_live] SKIPPED restricted-mod test -- candidate '{RESTRICTED_CANDIDATE_SLUG}' no longer has restricted files; update RESTRICTED_CANDIDATE_SLUG"
            );
            return;
        }
    };

    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let token = CancellationToken::new();

    let res = svc
        .install_mod_into_instance(&paths, slug, &detail, restricted, tx, token, JobId(0))
        .await;

    match res {
        Err(CurseForgeError::FileNotDownloadable {
            web_url,
            mod_slug,
            file_id,
        }) => {
            assert!(!web_url.is_empty(), "web_url must be non-empty");
            assert!(
                web_url.starts_with("https://www.curseforge.com/minecraft/mc-mods/"),
                "web_url must be canonical CurseForge URL, got: {web_url}"
            );
            assert_eq!(mod_slug, RESTRICTED_CANDIDATE_SLUG);
            assert_eq!(file_id, restricted.id);
            println!(
                "[curseforge_live] SUCCESS -- restricted mod surfaced FileNotDownloadable with web_url={web_url}"
            );

            // Atomicity: ledger MUST be empty after a failed install.
            let ledger_path = paths.instance_mod_ledger(slug);
            if ledger_path.exists() {
                let raw = tokio::fs::read_to_string(&ledger_path).await.unwrap();
                let ledger: ichr::mods::types::Ledger = toml::from_str(&raw).unwrap_or_default();
                assert!(
                    ledger.mods.is_empty(),
                    "ledger MUST be empty after failed install"
                );
            }
        }
        other => panic!("expected FileNotDownloadable, got {other:?}"),
    }
}
