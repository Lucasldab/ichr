//! Live integration tests for PackService against the real Modrinth API.
//! `#[ignore]`-gated; requires internet access.
//!
//! Pinned projects (verified live 2026-05-08):
//!
//! | slug                     | project_id | version_id | kind         |
//! |--------------------------|------------|------------|--------------|
//! | faithful-32x             | w0TnApzs   | kIpbQNcv   | resourcepack |
//! | complementary-reimagined | HVnmMxH1   | 836bPNGo   | shader       |
//!
//! Run with:
//!   cargo nextest run --test packs_live -- --include-ignored
//! or:
//!   cargo test --test packs_live -- --ignored --nocapture
//!
//! Skip with: `ICHR_SKIP_LIVE=1 cargo nextest run --test packs_live -- --include-ignored`
//!
//! Refresh policy: re-pin if either test fails with 404 or Modrinth ranking shifts.
//! Reference:
//!   curl 'https://api.modrinth.com/v2/project/faithful-32x' | jq '.id,.versions[0]'
//!   curl 'https://api.modrinth.com/v2/project/complementary-reimagined' | jq '.id,.versions[0]'

use tempfile::TempDir;

use ichr::domain::instance::InstanceManifest;
use ichr::instance::store::write_instance_manifest;
use ichr::mods::types::InstalledItemKind;
use ichr::packs::kind::PackKind;
use ichr::packs::service::PackService;
use ichr::persistence::paths::AppPaths;
use ichr::tasks::JobId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

// ─── Pin constants ─────────────────────────────────────────────────────────────

/// Faithful 32x -- resource pack pinned 2026-05-08.
const FAITHFUL_PROJECT_ID: &str = "w0TnApzs";
const FAITHFUL_SLUG: &str = "faithful-32x";
const FAITHFUL_VERSION_ID: &str = "kIpbQNcv";
const FAITHFUL_MC_VERSION: &str = "1.20.4";

/// Complementary Reimagined -- shader pack pinned 2026-05-08.
const COMPLEMENTARY_PROJECT_ID: &str = "HVnmMxH1";
const COMPLEMENTARY_SLUG: &str = "complementary-reimagined";
const COMPLEMENTARY_VERSION_ID: &str = "836bPNGo";
const COMPLEMENTARY_MC_VERSION: &str = "1.20.4";

// ─── Shared helpers ───────────────────────────────────────────────────────────

fn make_paths(td: &TempDir) -> AppPaths {
    AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    )
}

async fn make_instance(paths: &AppPaths, slug: &str) {
    let manifest = InstanceManifest::new(slug.to_string(), slug.to_string(), "1.20.4".to_string());
    write_instance_manifest(paths, &manifest)
        .await
        .expect("write_instance_manifest in test setup");
}

// ─── Live Test 1: Faithful 32x resource pack ─────────────────────────────────

/// End-to-end live install of Faithful 32x from Modrinth.
///
/// Flow:
///   1. search("faithful", Resource, mc=1.20.4) → locate by slug or project_id
///   2. list_versions(project_id, mc=1.20.4) → locate by version_id or first
///   3. get_version(version_id) → full ModrinthVersion
///   4. install_modrinth → assert dest + ledger shape
///
/// Robustness (Phase 08.1-06 pattern):
///   - Hit located by slug-first, project_id fallback (Modrinth ranking drift resilient)
///   - Version selected by version_id-first, versions.first() fallback (pin drift resilient)
///   - Failure messages include copy-pasteable curl diagnostics for re-pinning
#[tokio::test]
#[ignore = "requires internet -- downloads Faithful 32x from cdn.modrinth.com"]
async fn live_install_faithful_32x() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("ICHR_SKIP_LIVE").is_ok() {
        eprintln!("[packs_live] skipped (ICHR_SKIP_LIVE set)");
        return Ok(());
    }

    let td = TempDir::new()?;
    let paths = make_paths(&td);
    let slug = "live-test-resource";
    make_instance(&paths, slug).await;

    let svc = PackService::new()?;

    // 1. Search -- match by slug first, fall back to project_id.
    let hits = svc
        .search(
            "faithful 32x",
            PackKind::Resource,
            Some(FAITHFUL_MC_VERSION),
            Some(&paths),
            Some(slug),
        )
        .await
        .map_err(|e| format!("search failed: {e:?}"))?;

    assert!(
        !hits.is_empty(),
        "Faithful 32x not found in search results. \
         Probe: curl 'https://api.modrinth.com/v2/search?query=faithful+32x&facets=[[\"project_type:resourcepack\"]]' | jq '.hits[].slug'"
    );

    let hit = hits
        .iter()
        .find(|h| h.slug == FAITHFUL_SLUG)
        .or_else(|| hits.iter().find(|h| h.project_id == FAITHFUL_PROJECT_ID))
        .ok_or_else(|| format!(
            "Faithful 32x not found in search hits by slug '{FAITHFUL_SLUG}' or project_id '{FAITHFUL_PROJECT_ID}'.\n\
             Modrinth ranking may have shifted. Probe:\n\
             curl 'https://api.modrinth.com/v2/search?query=faithful+32x&facets=[[\"project_type:resourcepack\"]]' | jq '.hits[] | {{slug: .slug, project_id: .project_id}}'"
        ))?;

    eprintln!(
        "[packs_live] Faithful 32x project_id={} slug={}",
        hit.project_id, hit.slug
    );

    // 2. List versions -- prefer pinned version_id, fall back to first.
    let version_entries = svc
        .list_versions(
            &hit.project_id,
            Some(FAITHFUL_MC_VERSION),
            PackKind::Resource,
        )
        .await
        .map_err(|e| format!("list_versions failed: {e:?}"))?;

    assert!(
        !version_entries.is_empty(),
        "No versions returned for Faithful 32x (project_id={}). \
         Probe: curl 'https://api.modrinth.com/v2/project/{}/version?game_versions=[\"1.20.4\"]'",
        hit.project_id,
        hit.project_id
    );

    let chosen_entry = version_entries
        .iter()
        .find(|v| v.version_id == FAITHFUL_VERSION_ID)
        .or_else(|| version_entries.first())
        .ok_or("no versions for Faithful 32x")?;

    if chosen_entry.version_id != FAITHFUL_VERSION_ID {
        eprintln!(
            "[packs_live] DRIFT WARNING: pinned version_id '{}' not found; using '{}' instead. \
             Update FAITHFUL_VERSION_ID in this test.\n\
             Probe: curl 'https://api.modrinth.com/v2/project/{}/version?game_versions=[\"1.20.4\"]' | jq '.[0].id'",
            FAITHFUL_VERSION_ID, chosen_entry.version_id, hit.project_id
        );
    }

    // 3. Get full ModrinthVersion.
    let full_version = svc
        .get_version(&chosen_entry.version_id)
        .await
        .map_err(|e| format!("get_version '{}' failed: {e:?}", chosen_entry.version_id))?;

    // 4. Install.
    let (progress_tx, mut rx) = mpsc::channel(256);

    let drain = tokio::spawn(async move {
        while let Some(ichr::tasks::TaskEvent::Progress { pct, msg, .. }) = rx.recv().await {
            eprintln!("[packs_live:faithful] {pct:3}% -- {msg}");
        }
    });

    let token = CancellationToken::new();
    let row = svc
        .install_modrinth(
            &paths,
            slug,
            PackKind::Resource,
            &full_version,
            &hit.slug,
            &hit.project_id,
            &hit.title,
            progress_tx,
            token,
            JobId(1),
        )
        .await
        .map_err(|e| format!("install_modrinth failed: {e:?}"))?;

    let _ = drain.await;

    // Assert dest file exists in resourcepacks/.
    let dest = paths.instance_pack_file(slug, PackKind::Resource, &row.file_name);
    assert!(
        tokio::fs::try_exists(&dest).await?,
        "dest file missing after Faithful 32x install: {}",
        dest.display()
    );

    // Assert ledger has 1 row with kind=ResourcePack + source=Modrinth.
    let ledger = ichr::mods::ledger::read_ledger(&paths, slug)
        .await
        .map_err(|e| format!("read_ledger failed: {e:?}"))?;
    assert_eq!(
        ledger.mods.len(),
        1,
        "ledger must have exactly 1 row; got {}",
        ledger.mods.len()
    );
    let r = &ledger.mods[0];
    assert_eq!(
        r.kind,
        InstalledItemKind::ResourcePack,
        "row kind must be ResourcePack"
    );
    assert_eq!(
        r.source,
        ichr::mods::types::ModSource::Modrinth,
        "row source must be Modrinth"
    );
    assert!(
        !r.sha512.is_empty(),
        "sha512 field (storing SHA-1) must be non-empty"
    );

    eprintln!(
        "[packs_live] SUCCESS -- installed {} (sha1_short={})",
        row.file_name,
        &r.sha512[..16.min(r.sha512.len())]
    );

    Ok(())
}

// ─── Live Test 2: Complementary Reimagined shader pack ───────────────────────

/// End-to-end live install of Complementary Reimagined from Modrinth.
///
/// Same slug-first / version_id-first fallback pattern as Faithful 32x.
/// Asserts dest in shaderpacks/ + ledger row with kind=Shader.
#[tokio::test]
#[ignore = "requires internet -- downloads Complementary Reimagined from cdn.modrinth.com"]
async fn live_install_complementary_reimagined() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("ICHR_SKIP_LIVE").is_ok() {
        eprintln!("[packs_live] skipped (ICHR_SKIP_LIVE set)");
        return Ok(());
    }

    let td = TempDir::new()?;
    let paths = make_paths(&td);
    let slug = "live-test-shader";
    make_instance(&paths, slug).await;

    let svc = PackService::new()?;

    // 1. Search -- match by slug first, fall back to project_id.
    let hits = svc
        .search(
            "complementary reimagined",
            PackKind::Shader,
            Some(COMPLEMENTARY_MC_VERSION),
            Some(&paths),
            Some(slug),
        )
        .await
        .map_err(|e| format!("search failed: {e:?}"))?;

    assert!(
        !hits.is_empty(),
        "Complementary Reimagined not found in search results. \
         Probe: curl 'https://api.modrinth.com/v2/search?query=complementary+reimagined&facets=[[\"project_type:shader\"]]' | jq '.hits[].slug'"
    );

    let hit = hits
        .iter()
        .find(|h| h.slug == COMPLEMENTARY_SLUG)
        .or_else(|| hits.iter().find(|h| h.project_id == COMPLEMENTARY_PROJECT_ID))
        .ok_or_else(|| format!(
            "Complementary Reimagined not found in search hits by slug '{COMPLEMENTARY_SLUG}' or project_id '{COMPLEMENTARY_PROJECT_ID}'.\n\
             Modrinth ranking may have shifted. Probe:\n\
             curl 'https://api.modrinth.com/v2/search?query=complementary+reimagined&facets=[[\"project_type:shader\"]]' | jq '.hits[] | {{slug: .slug, project_id: .project_id}}'"
        ))?;

    eprintln!(
        "[packs_live] Complementary Reimagined project_id={} slug={}",
        hit.project_id, hit.slug
    );

    // 2. List versions -- prefer pinned version_id, fall back to first.
    let version_entries = svc
        .list_versions(
            &hit.project_id,
            Some(COMPLEMENTARY_MC_VERSION),
            PackKind::Shader,
        )
        .await
        .map_err(|e| format!("list_versions failed: {e:?}"))?;

    assert!(
        !version_entries.is_empty(),
        "No versions returned for Complementary Reimagined (project_id={}). \
         Probe: curl 'https://api.modrinth.com/v2/project/{}/version'",
        hit.project_id,
        hit.project_id
    );

    let chosen_entry = version_entries
        .iter()
        .find(|v| v.version_id == COMPLEMENTARY_VERSION_ID)
        .or_else(|| version_entries.first())
        .ok_or("no versions for Complementary Reimagined")?;

    if chosen_entry.version_id != COMPLEMENTARY_VERSION_ID {
        eprintln!(
            "[packs_live] DRIFT WARNING: pinned version_id '{}' not found; using '{}' instead. \
             Update COMPLEMENTARY_VERSION_ID in this test.\n\
             Probe: curl 'https://api.modrinth.com/v2/project/{}/version' | jq '.[0].id'",
            COMPLEMENTARY_VERSION_ID, chosen_entry.version_id, hit.project_id
        );
    }

    // 3. Get full ModrinthVersion.
    let full_version = svc
        .get_version(&chosen_entry.version_id)
        .await
        .map_err(|e| format!("get_version '{}' failed: {e:?}", chosen_entry.version_id))?;

    // 4. Install.
    let (progress_tx, mut rx) = mpsc::channel(256);

    let drain = tokio::spawn(async move {
        while let Some(ichr::tasks::TaskEvent::Progress { pct, msg, .. }) = rx.recv().await {
            eprintln!("[packs_live:complementary] {pct:3}% -- {msg}");
        }
    });

    let token = CancellationToken::new();
    let row = svc
        .install_modrinth(
            &paths,
            slug,
            PackKind::Shader,
            &full_version,
            &hit.slug,
            &hit.project_id,
            &hit.title,
            progress_tx,
            token,
            JobId(2),
        )
        .await
        .map_err(|e| format!("install_modrinth failed: {e:?}"))?;

    let _ = drain.await;

    // Assert dest file exists in shaderpacks/.
    let dest = paths.instance_pack_file(slug, PackKind::Shader, &row.file_name);
    assert!(
        tokio::fs::try_exists(&dest).await?,
        "dest file missing after Complementary Reimagined install: {}",
        dest.display()
    );

    // Assert ledger has 1 row with kind=Shader + source=Modrinth.
    let ledger = ichr::mods::ledger::read_ledger(&paths, slug)
        .await
        .map_err(|e| format!("read_ledger failed: {e:?}"))?;
    assert_eq!(
        ledger.mods.len(),
        1,
        "ledger must have exactly 1 row; got {}",
        ledger.mods.len()
    );
    let r = &ledger.mods[0];
    assert_eq!(r.kind, InstalledItemKind::Shader, "row kind must be Shader");
    assert_eq!(
        r.source,
        ichr::mods::types::ModSource::Modrinth,
        "row source must be Modrinth"
    );
    assert!(
        !r.sha512.is_empty(),
        "sha512 field (storing SHA-1) must be non-empty"
    );

    eprintln!(
        "[packs_live] SUCCESS -- installed {} (sha1_short={})",
        row.file_name,
        &r.sha512[..16.min(r.sha512.len())]
    );

    Ok(())
}
