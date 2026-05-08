//! Per-instance mod ledger — TOML sidecar at `instance_dir/installed-mods.toml`.
//!
//! - 7 functions: read_ledger / write_ledger / upsert_mod / remove_mod /
//!   toggle_enabled / uninstall / per_instance_lock.
//! - All mutation operations atomically update BOTH the disk file (.jar
//!   rename or removal) AND the ledger row, serialized per-instance via a
//!   `tokio::sync::Mutex<()>` map (Pitfall 8 protocol — held only during
//!   read→mutate→write, never across HTTP).
//!
//! ASSUMPTION A4 from 08-RESEARCH.md — sidecar TOML is the right pattern
//! (vs. extending instance.json). Verify in human checkpoint.
//!
//! Mirrors `src/auth/store.rs::{save_accounts, load_accounts}` (TOML
//! instead of JSON) and `src/instance/store.rs` (per-instance read_X /
//! write_X helpers).
//!
//! ## Error wrapping note
//!
//! `ModrinthError::Io(#[from] std::io::Error)` requires a real `io::Error`,
//! not a string. We use `std::io::Error::other(format!(...))` (stable since
//! Rust 1.74) to attach context strings to filesystem failures while still
//! producing a typed `io::Error`. AppError surfaced from `atomic_write` is
//! likewise re-wrapped as `io::Error::other(e.to_string())` so the public
//! signature stays `Result<_, ModrinthError>`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use tokio::sync::Mutex;

use crate::mods::error::ModrinthError;
use crate::mods::filter::{disabled_filename, is_safe_mod_filename, is_safe_pack_filename};
use crate::mods::types::{InstalledModRow, Ledger};
use crate::persistence::paths::AppPaths;

// ============================================================================
// === Per-instance lock map (Pitfall 8)                                    ===
// ============================================================================

/// Per-instance lock map — serializes ledger mutations for one slug.
/// The outer std::sync mutex protects the map; the inner tokio mutex
/// serializes mutation operations for a single instance.
fn instance_locks() -> &'static std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>> {
    static LOCKS: OnceLock<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Get (or create) the per-instance ledger mutation lock for `slug`.
/// Hold this across `read_ledger → mutate → write_ledger` ONLY; never across
/// HTTP (Pitfall 8). Returns the same `Arc<Mutex<()>>` for the same slug
/// across calls so concurrent install tasks for one instance serialize on
/// the same lock.
pub fn per_instance_lock(slug: &str) -> Arc<Mutex<()>> {
    let mut map = instance_locks().lock().expect("per_instance_lock poisoned");
    map.entry(slug.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

// ============================================================================
// === Internal helpers                                                     ===
// ============================================================================

/// Wrap a context string into a `ModrinthError::Io` via `io::Error::other`.
/// Lets callers attach human-readable context to filesystem failures while
/// the `Io` variant continues to take a real `io::Error` (#[from]).
fn io_err(msg: impl Into<String>) -> ModrinthError {
    ModrinthError::Io(std::io::Error::other(msg.into()))
}

// ============================================================================
// === Read / Write                                                         ===
// ============================================================================

#[tracing::instrument(name = "mods::read_ledger", skip_all, fields(slug = %slug))]
pub async fn read_ledger(paths: &AppPaths, slug: &str) -> Result<Ledger, ModrinthError> {
    let path = paths.instance_mod_ledger(slug);
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Ok(Ledger::default());
    }
    let raw = tokio::fs::read(&path)
        .await
        .map_err(|e| io_err(format!("read {path:?}: {e}")))?;
    if raw.is_empty() {
        return Ok(Ledger::default());
    }
    let s = std::str::from_utf8(&raw)
        .map_err(|e| ModrinthError::LedgerParse(format!("non-UTF8 ledger: {e}")))?;
    toml::from_str(s).map_err(|e| ModrinthError::LedgerParse(format!("parse {path:?}: {e}")))
}

#[tracing::instrument(
    name = "mods::write_ledger",
    skip_all,
    fields(slug = %slug, mods_count = ledger.mods.len())
)]
pub async fn write_ledger(
    paths: &AppPaths,
    slug: &str,
    ledger: &Ledger,
) -> Result<(), ModrinthError> {
    let path = paths.instance_mod_ledger(slug);
    let s = toml::to_string_pretty(ledger)
        .map_err(|e| ModrinthError::LedgerParse(format!("serialize: {e}")))?;
    crate::mojang::cache::atomic_write(&path, s.as_bytes())
        .await
        .map_err(|e| io_err(format!("ledger write {path:?}: {e}")))?;
    Ok(())
}

// ============================================================================
// === Mutations (per-instance lock; read → mutate → write)                ===
// ============================================================================

#[tracing::instrument(
    name = "mods::upsert_mod",
    skip_all,
    fields(slug = %slug, mod_id = %row.mod_id)
)]
pub async fn upsert_mod(
    paths: &AppPaths,
    slug: &str,
    row: InstalledModRow,
) -> Result<(), ModrinthError> {
    // V5 input validation: never accept a mod row with an unsafe filename.
    if !is_safe_mod_filename(&row.file_name) {
        return Err(io_err(format!("unsafe ledger filename: {}", row.file_name)));
    }
    let lock = per_instance_lock(slug);
    let _guard = lock.lock().await;

    let mut ledger = read_ledger(paths, slug).await?;
    if let Some(existing) = ledger.mods.iter_mut().find(|m| m.mod_id == row.mod_id) {
        *existing = row;
    } else {
        ledger.mods.push(row);
    }
    write_ledger(paths, slug, &ledger).await
}

/// Insert or replace a pack row in the per-instance ledger.
/// Mirrors `upsert_mod` exactly — same per-instance lock, same read→mutate→write,
/// same replace-on-mod_id semantics — but validates filename via
/// `is_safe_pack_filename` (which accepts `.zip` instead of `.jar`).
/// Per 11-RESEARCH.md §"Upsert Validation Gate" + Researcher Q4 (keep
/// ledger logic together).
#[tracing::instrument(
    name = "packs::upsert_pack",
    skip_all,
    fields(slug = %slug, mod_id = %row.mod_id, kind = ?row.kind)
)]
pub async fn upsert_pack(
    paths: &AppPaths,
    slug: &str,
    row: InstalledModRow,
) -> Result<(), ModrinthError> {
    // V5 input validation — pack flavor (.zip + SPACE allowed).
    if !is_safe_pack_filename(&row.file_name) {
        return Err(io_err(format!("unsafe ledger filename: {}", row.file_name)));
    }
    let lock = per_instance_lock(slug);
    let _guard = lock.lock().await;

    let mut ledger = read_ledger(paths, slug).await?;
    if let Some(existing) = ledger.mods.iter_mut().find(|m| m.mod_id == row.mod_id) {
        *existing = row;
    } else {
        ledger.mods.push(row);
    }
    write_ledger(paths, slug, &ledger).await
}

#[tracing::instrument(name = "mods::remove_mod", skip_all, fields(slug = %slug, mod_id))]
pub async fn remove_mod(paths: &AppPaths, slug: &str, mod_id: &str) -> Result<(), ModrinthError> {
    let lock = per_instance_lock(slug);
    let _guard = lock.lock().await;

    let mut ledger = read_ledger(paths, slug).await?;
    let idx = ledger
        .mods
        .iter()
        .position(|m| m.mod_id == mod_id)
        .ok_or_else(|| ModrinthError::ModNotFound(mod_id.to_string()))?;
    ledger.mods.remove(idx);
    write_ledger(paths, slug, &ledger).await
}

/// Atomically toggle a mod between `.jar` and `.jar.disabled`.
/// File rename happens BEFORE ledger write — if rename fails, ledger is unchanged.
/// Returns the NEW enabled state.
#[tracing::instrument(name = "mods::toggle_enabled", skip_all, fields(slug = %slug, mod_id))]
pub async fn toggle_enabled(
    paths: &AppPaths,
    slug: &str,
    mod_id: &str,
) -> Result<bool, ModrinthError> {
    let lock = per_instance_lock(slug);
    let _guard = lock.lock().await;

    let mut ledger = read_ledger(paths, slug).await?;
    let row = ledger
        .mods
        .iter_mut()
        .find(|m| m.mod_id == mod_id)
        .ok_or_else(|| ModrinthError::ModNotFound(mod_id.to_string()))?;

    if !is_safe_mod_filename(&row.file_name) {
        return Err(io_err(format!("unsafe ledger filename: {}", row.file_name)));
    }

    let mods_dir = paths.instance_minecraft_dir(slug).join("mods");
    let (current_path, new_path) = if row.enabled {
        (
            mods_dir.join(&row.file_name),
            mods_dir.join(disabled_filename(&row.file_name)),
        )
    } else {
        (
            mods_dir.join(disabled_filename(&row.file_name)),
            mods_dir.join(&row.file_name),
        )
    };

    tokio::fs::rename(&current_path, &new_path).await.map_err(|e| {
        io_err(format!(
            "rename {} -> {}: {e}",
            current_path.display(),
            new_path.display()
        ))
    })?;

    row.enabled = !row.enabled;
    let new_state = row.enabled;
    write_ledger(paths, slug, &ledger).await?;
    Ok(new_state)
}

/// Remove a mod's file AND its ledger row. File removal happens BEFORE ledger
/// write — if file removal fails (other than NotFound), the ledger is unchanged.
///
/// Defensive behavior: if the on-disk file is already missing (user manually
/// deleted), we still drop the ledger row. This keeps the ledger consistent
/// with the user's apparent intent.
#[tracing::instrument(name = "mods::uninstall", skip_all, fields(slug = %slug, mod_id))]
pub async fn uninstall(paths: &AppPaths, slug: &str, mod_id: &str) -> Result<(), ModrinthError> {
    let lock = per_instance_lock(slug);
    let _guard = lock.lock().await;

    let ledger_pre = read_ledger(paths, slug).await?;
    let row = ledger_pre
        .mods
        .iter()
        .find(|m| m.mod_id == mod_id)
        .ok_or_else(|| ModrinthError::ModNotFound(mod_id.to_string()))?
        .clone();

    if !is_safe_mod_filename(&row.file_name) {
        return Err(io_err(format!("unsafe ledger filename: {}", row.file_name)));
    }

    let mods_dir = paths.instance_minecraft_dir(slug).join("mods");
    let target = if row.enabled {
        mods_dir.join(&row.file_name)
    } else {
        mods_dir.join(disabled_filename(&row.file_name))
    };

    // Best-effort remove — file may already be missing (user manually deleted);
    // we still drop the ledger row in that case. Surface only non-NotFound errors.
    match tokio::fs::remove_file(&target).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(
                "uninstall: mod file {} already missing, dropping ledger row anyway",
                target.display()
            );
        }
        Err(e) => {
            return Err(io_err(format!("remove_file {}: {e}", target.display())));
        }
    }

    let mut ledger = ledger_pre;
    let idx = ledger
        .mods
        .iter()
        .position(|m| m.mod_id == mod_id)
        .ok_or_else(|| ModrinthError::ModNotFound(mod_id.to_string()))?;
    ledger.mods.remove(idx);
    write_ledger(paths, slug, &ledger).await
}

// ============================================================================
// === Tests                                                               ===
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::types::{HashAlgo, InstalledItemKind, InstalledModRow, ModSource};
    use super::upsert_pack;
    use tempfile::TempDir;

    fn test_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    fn mk_row(mod_id: &str, file_name: &str, enabled: bool) -> InstalledModRow {
        InstalledModRow {
            mod_id: mod_id.into(),
            project_slug: "sodium".into(),
            display_name: "Sodium".into(),
            version_id: "v1".into(),
            version_label: "0.5.8".into(),
            file_name: file_name.into(),
            sha512: "deadbeef".into(),
            size: 1024,
            hash_algo: HashAlgo::Sha512,
            kind: InstalledItemKind::Mod,
            source: ModSource::Modrinth,
            enabled,
            installed_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    async fn ensure_mods_dir(paths: &AppPaths, slug: &str) {
        tokio::fs::create_dir_all(paths.instance_minecraft_dir(slug).join("mods"))
            .await
            .unwrap();
    }

    async fn touch(p: &std::path::Path) {
        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent).await.unwrap();
        }
        tokio::fs::write(p, b"jar bytes").await.unwrap();
    }

    // --- read_ledger ----------------------------------------------------------

    #[tokio::test]
    async fn test_read_missing_returns_empty_ledger() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let l = read_ledger(&paths, "fresh").await.unwrap();
        assert_eq!(l.schema_version, 1);
        assert!(l.mods.is_empty());
    }

    #[tokio::test]
    async fn test_read_empty_file_returns_empty_ledger() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let p = paths.instance_mod_ledger("fresh");
        tokio::fs::create_dir_all(p.parent().unwrap()).await.unwrap();
        tokio::fs::write(&p, b"").await.unwrap();
        let l = read_ledger(&paths, "fresh").await.unwrap();
        assert!(l.mods.is_empty());
    }

    #[tokio::test]
    async fn test_round_trip_one_mod() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let l = Ledger {
            schema_version: 1,
            mods: vec![mk_row("m1", "sodium.jar", true)],
        };
        write_ledger(&paths, "rt", &l).await.unwrap();
        let parsed = read_ledger(&paths, "rt").await.unwrap();
        assert_eq!(parsed, l);
    }

    #[tokio::test]
    async fn test_corrupted_toml_returns_ledger_parse() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let p = paths.instance_mod_ledger("bad");
        tokio::fs::create_dir_all(p.parent().unwrap()).await.unwrap();
        tokio::fs::write(&p, b"this is { not toml").await.unwrap();
        let r = read_ledger(&paths, "bad").await;
        assert!(matches!(r, Err(ModrinthError::LedgerParse(_))), "got {r:?}");
    }

    // --- upsert_mod -----------------------------------------------------------

    #[tokio::test]
    async fn test_upsert_appends_new_then_replaces_existing() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        upsert_mod(&paths, "x", mk_row("m1", "sodium.jar", true))
            .await
            .unwrap();
        upsert_mod(&paths, "x", mk_row("m2", "fabric-api.jar", true))
            .await
            .unwrap();
        let l = read_ledger(&paths, "x").await.unwrap();
        assert_eq!(l.mods.len(), 2);

        // Replace m1
        let mut updated = mk_row("m1", "sodium.jar", false);
        updated.version_label = "0.5.9".into();
        upsert_mod(&paths, "x", updated).await.unwrap();
        let l = read_ledger(&paths, "x").await.unwrap();
        assert_eq!(l.mods.len(), 2, "still 2 rows");
        let m1 = l.mods.iter().find(|m| m.mod_id == "m1").unwrap();
        assert_eq!(m1.version_label, "0.5.9");
        assert!(!m1.enabled);
    }

    #[tokio::test]
    async fn test_upsert_rejects_unsafe_filename() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let bad = mk_row("m1", "../etc/passwd.jar", true);
        let r = upsert_mod(&paths, "x", bad).await;
        assert!(matches!(r, Err(ModrinthError::Io(_))), "got {r:?}");
    }

    // --- remove_mod -----------------------------------------------------------

    #[tokio::test]
    async fn test_remove_unknown_returns_mod_not_found() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        upsert_mod(&paths, "x", mk_row("m1", "sodium.jar", true))
            .await
            .unwrap();
        let r = remove_mod(&paths, "x", "nope").await;
        assert!(
            matches!(r, Err(ModrinthError::ModNotFound(ref id)) if id == "nope"),
            "got {r:?}"
        );
    }

    // --- toggle_enabled -------------------------------------------------------

    #[tokio::test]
    async fn test_toggle_renames_jar_to_jar_disabled_and_flips_ledger() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        ensure_mods_dir(&paths, "x").await;
        let row = mk_row("m1", "sodium.jar", true);
        upsert_mod(&paths, "x", row.clone()).await.unwrap();
        touch(&paths.instance_mod_file("x", "sodium.jar")).await;

        let new_state = toggle_enabled(&paths, "x", "m1").await.unwrap();
        assert!(!new_state);
        assert!(
            !paths.instance_mod_file("x", "sodium.jar").exists(),
            "jar should be gone"
        );
        assert!(
            paths.instance_mod_file("x", "sodium.jar.disabled").exists(),
            "disabled file should be present"
        );
        let l = read_ledger(&paths, "x").await.unwrap();
        assert!(!l.mods[0].enabled);
    }

    #[tokio::test]
    async fn test_toggle_idempotent_returns_to_enabled() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        ensure_mods_dir(&paths, "x").await;
        upsert_mod(&paths, "x", mk_row("m1", "sodium.jar", true))
            .await
            .unwrap();
        touch(&paths.instance_mod_file("x", "sodium.jar")).await;

        assert!(!toggle_enabled(&paths, "x", "m1").await.unwrap());
        assert!(toggle_enabled(&paths, "x", "m1").await.unwrap());
        assert!(paths.instance_mod_file("x", "sodium.jar").exists());
    }

    #[tokio::test]
    async fn test_toggle_unknown_mod_id() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let r = toggle_enabled(&paths, "x", "ghost").await;
        assert!(matches!(r, Err(ModrinthError::ModNotFound(_))), "got {r:?}");
    }

    #[tokio::test]
    async fn test_toggle_when_file_missing_does_not_update_ledger() {
        // Pitfall 8 atomicity invariant: rename failure must NOT update ledger.
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        ensure_mods_dir(&paths, "x").await;
        upsert_mod(&paths, "x", mk_row("m1", "sodium.jar", true))
            .await
            .unwrap();
        // Do NOT touch the .jar file.
        let r = toggle_enabled(&paths, "x", "m1").await;
        assert!(matches!(r, Err(ModrinthError::Io(_))), "got {r:?}");
        // Ledger still says enabled=true.
        let l = read_ledger(&paths, "x").await.unwrap();
        assert!(
            l.mods[0].enabled,
            "ledger must NOT have been updated when rename failed"
        );
    }

    // --- uninstall ------------------------------------------------------------

    #[tokio::test]
    async fn test_uninstall_removes_jar_and_row() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        ensure_mods_dir(&paths, "x").await;
        upsert_mod(&paths, "x", mk_row("m1", "sodium.jar", true))
            .await
            .unwrap();
        upsert_mod(&paths, "x", mk_row("m2", "fabric-api.jar", true))
            .await
            .unwrap();
        touch(&paths.instance_mod_file("x", "sodium.jar")).await;
        touch(&paths.instance_mod_file("x", "fabric-api.jar")).await;

        uninstall(&paths, "x", "m1").await.unwrap();
        assert!(!paths.instance_mod_file("x", "sodium.jar").exists());
        assert!(
            paths.instance_mod_file("x", "fabric-api.jar").exists(),
            "other mod untouched"
        );
        let l = read_ledger(&paths, "x").await.unwrap();
        assert_eq!(l.mods.len(), 1);
        assert_eq!(l.mods[0].mod_id, "m2");
    }

    #[tokio::test]
    async fn test_uninstall_disabled_mod_removes_dot_disabled_file() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        ensure_mods_dir(&paths, "x").await;
        let row = mk_row("m1", "sodium.jar", false); // already disabled
        upsert_mod(&paths, "x", row).await.unwrap();
        touch(&paths.instance_mod_file("x", "sodium.jar.disabled")).await;

        uninstall(&paths, "x", "m1").await.unwrap();
        assert!(!paths.instance_mod_file("x", "sodium.jar.disabled").exists());
        let l = read_ledger(&paths, "x").await.unwrap();
        assert!(l.mods.is_empty());
    }

    #[tokio::test]
    async fn test_uninstall_when_file_missing_still_drops_row() {
        // Defensive: user may have manually deleted the JAR. Drop the row.
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        upsert_mod(&paths, "x", mk_row("m1", "sodium.jar", true))
            .await
            .unwrap();
        uninstall(&paths, "x", "m1").await.unwrap();
        let l = read_ledger(&paths, "x").await.unwrap();
        assert!(l.mods.is_empty());
    }

    // --- per_instance_lock ----------------------------------------------------

    #[tokio::test]
    async fn test_per_instance_lock_returns_same_arc_per_slug() {
        let a1 = per_instance_lock("foo-08-05");
        let a2 = per_instance_lock("foo-08-05");
        assert!(Arc::ptr_eq(&a1, &a2), "same slug must yield same Arc");

        let b = per_instance_lock("bar-08-05");
        assert!(
            !Arc::ptr_eq(&a1, &b),
            "different slugs must yield different Arcs"
        );
    }

    // --- upsert_pack ---------------------------------------------------------

    fn mk_pack_row(mod_id: &str, file_name: &str, kind: InstalledItemKind) -> InstalledModRow {
        InstalledModRow {
            mod_id: mod_id.into(),
            project_slug: "faithful-32x".into(),
            display_name: "Faithful 32x".into(),
            version_id: "v1".into(),
            version_label: "1.0".into(),
            file_name: file_name.into(),
            sha512: "deadbeef".into(),
            size: 10 * 1024 * 1024,
            hash_algo: HashAlgo::Sha1,
            kind,
            source: ModSource::Local,
            enabled: true,
            installed_at: "2026-05-08T00:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn test_upsert_pack_writes_zip_filename_row() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let row = mk_pack_row("pack:abc", "Faithful 32x.zip", InstalledItemKind::ResourcePack);
        upsert_pack(&paths, "x", row).await.unwrap();
        let l = read_ledger(&paths, "x").await.unwrap();
        assert_eq!(l.mods.len(), 1);
        assert_eq!(l.mods[0].file_name, "Faithful 32x.zip");
        assert_eq!(l.mods[0].kind, InstalledItemKind::ResourcePack);
    }

    #[tokio::test]
    async fn test_upsert_pack_rejects_jar_filename() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let bad = mk_pack_row("pack:abc", "sodium.jar", InstalledItemKind::ResourcePack);
        let r = upsert_pack(&paths, "x", bad).await;
        assert!(matches!(r, Err(ModrinthError::Io(_))), "got {r:?}");
        // Error message must mention "unsafe ledger filename".
        if let Err(ModrinthError::Io(e)) = r {
            assert!(
                e.to_string().contains("unsafe ledger filename"),
                "message: {e}"
            );
        }
    }

    #[tokio::test]
    async fn test_upsert_pack_rejects_path_traversal() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let bad = mk_pack_row("pack:abc", "../escape.zip", InstalledItemKind::Shader);
        let r = upsert_pack(&paths, "x", bad).await;
        assert!(matches!(r, Err(ModrinthError::Io(_))), "got {r:?}");
    }

    #[tokio::test]
    async fn test_upsert_pack_replaces_row_on_same_mod_id() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);
        let row1 = mk_pack_row("pack:abc", "OldPack.zip", InstalledItemKind::ResourcePack);
        let mut row2 = mk_pack_row("pack:abc", "NewPack.zip", InstalledItemKind::ResourcePack);
        row2.display_name = "New Faithful".into();

        upsert_pack(&paths, "x", row1).await.unwrap();
        upsert_pack(&paths, "x", row2).await.unwrap();

        let l = read_ledger(&paths, "x").await.unwrap();
        // Same mod_id → replace, NOT append.
        assert_eq!(l.mods.len(), 1, "must replace, not append");
        assert_eq!(l.mods[0].file_name, "NewPack.zip");
    }

    #[tokio::test]
    async fn test_upsert_pack_coexists_with_upsert_mod() {
        let td = TempDir::new().unwrap();
        let paths = test_paths(&td);

        // Insert a mod row via upsert_mod.
        upsert_mod(&paths, "x", mk_row("mod:m1", "sodium.jar", true))
            .await
            .unwrap();

        // Insert a pack row via upsert_pack.
        let pack =
            mk_pack_row("pack:p1", "Faithful 32x.zip", InstalledItemKind::ResourcePack);
        upsert_pack(&paths, "x", pack).await.unwrap();

        let l = read_ledger(&paths, "x").await.unwrap();
        assert_eq!(l.mods.len(), 2, "both rows present");

        let mod_row = l.mods.iter().find(|m| m.mod_id == "mod:m1").unwrap();
        let pack_row = l.mods.iter().find(|m| m.mod_id == "pack:p1").unwrap();
        assert_eq!(mod_row.kind, InstalledItemKind::Mod);
        assert_eq!(pack_row.kind, InstalledItemKind::ResourcePack);
        assert!(
            mod_row.file_name.ends_with(".jar"),
            "mod row still .jar"
        );
        assert!(
            pack_row.file_name.ends_with(".zip"),
            "pack row is .zip"
        );
    }
}
