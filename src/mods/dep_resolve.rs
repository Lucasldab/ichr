//! BFS dependency resolution over Modrinth project versions.
//!
//! The algorithm is **pure-async** -- caller injects fetch closures so the
//! algorithm is testable without httpmock (just hand-built ModrinthVersion
//! fixtures). 08-06 ModrinthService passes closures that delegate to
//! ModrinthClient::list_versions / get_version.
//!
//! Edge cases enumerated in 08-RESEARCH.md Section Pattern 5:
//!   1. Circular deps -- `seen` HashSet ensures each project_id is processed once.
//!   2. Missing version for MC+loader -- `fetch_latest` returns None -> NoCompatibleVersion.
//!   3. Required dep already installed -- pre-seeded `seen` from ledger.
//!   4. Embedded dep -- never queued, never downloaded.
//!   5. Optional dep -- collected for UI; never queued.
//!   6. Incompatible dep already installed -- abort with DependencyConflict.
//!   7. Version downgrade -- out of scope for v1 (atomic_write replaces existing JAR).
//!
//! Q2 (08-RESEARCH.md Open Question Q2): if a dep has only `version_id` set
//! (no `project_id`), the algorithm calls `fetch_version_by_id` to resolve the
//! version, then derives `project_id` from the response and dedupes against `seen`.
//!
//! Algorithm shape mirrors `src/mojang/inherits.rs::resolve_inherits` -- iterative
//! `VecDeque` + `HashSet` walk with no recursion (no `Box<dyn Future>` pin needed).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::mods::error::ModrinthError;
use crate::mods::filter::pick_primary_file;
use crate::mods::types::{DepKind, ModrinthDep, ModrinthVersion, ResolvedDep};

/// Result of dependency resolution.
///
/// `deps` is in BFS layer order; the root version is NOT included.
/// `total_new_bytes` / `total_new_files` count only required-and-not-already-satisfied
/// entries (matches the dep-confirm modal "Total: N file(s) to download (~size)" line).
#[derive(Debug, Clone)]
pub struct ResolvedDepGraph {
    pub root: ModrinthVersion,
    pub deps: Vec<ResolvedDep>,
    pub total_new_bytes: u64,
    pub total_new_files: usize,
}

/// BFS resolve of the root version's required dependencies.
///
/// Caller responsibilities:
/// - `installed`: `project_id -> version_id` from the per-instance ledger.
/// - `fetch_latest_for_project`: returns the latest matching ModrinthVersion (or None).
///   For 08-06, this is `client.list_versions(...).map(pick_latest_by_date)`.
/// - `fetch_version_by_id`: resolves a version_id to a ModrinthVersion (Q2 path).
///   For 08-06, this is `client.get_version(version_id)`.
pub async fn resolve_required_deps<F1, Fut1, F2, Fut2>(
    root: ModrinthVersion,
    mc: &str,
    loaders: &[String],
    installed: &HashMap<String, String>,
    fetch_latest_for_project: F1,
    fetch_version_by_id: F2,
) -> Result<ResolvedDepGraph, ModrinthError>
where
    F1: Fn(String, String, Vec<String>) -> Fut1 + Sync,
    Fut1: std::future::Future<Output = Result<Option<ModrinthVersion>, ModrinthError>> + Send,
    F2: Fn(String) -> Fut2 + Sync,
    Fut2: std::future::Future<Output = Result<ModrinthVersion, ModrinthError>> + Send,
{
    let mut deps: Vec<ResolvedDep> = Vec::new();
    let mut seen: HashSet<String> = installed.keys().cloned().collect();
    let mut q: VecDeque<ModrinthVersion> = VecDeque::new();
    q.push_back(root.clone());

    while let Some(v) = q.pop_front() {
        // Iterate by index to avoid borrowing `v` across `.await` (the inner
        // closure may need `v.project_id` but we only read scalar fields).
        for d in v.dependencies.clone() {
            // Resolve project_id -- direct on `dep`, or via Q2 fallback to fetch_version_by_id.
            let project_id_owned: String = match (&d.project_id, &d.version_id) {
                (Some(pid), _) => pid.clone(),
                (None, Some(vid)) => {
                    // Q2 path: dep pins a version_id without project_id. Resolve it.
                    let resolved = fetch_version_by_id(vid.clone()).await?;
                    resolved.project_id.clone()
                }
                (None, None) => continue, // spec-rare; skip silently per 08-RESEARCH.md Pattern 5.
            };

            process_dep(
                &d,
                project_id_owned,
                &v,
                mc,
                loaders,
                installed,
                &mut seen,
                &mut q,
                &mut deps,
                &fetch_latest_for_project,
            )
            .await?;
        }
    }

    let mut total_new_bytes: u64 = 0;
    let mut total_new_files: usize = 0;
    for rd in &deps {
        if matches!(rd.kind, DepKind::Required) && rd.is_new_download {
            if let Some(ver) = &rd.version {
                if let Some(f) = pick_primary_file(&ver.files) {
                    total_new_bytes += f.size;
                }
                total_new_files += 1;
            }
        }
    }

    Ok(ResolvedDepGraph { root, deps, total_new_bytes, total_new_files })
}

/// Inner per-dep processing -- extracted to keep the BFS loop readable.
#[allow(clippy::too_many_arguments)]
async fn process_dep<F1, Fut1>(
    d: &ModrinthDep,
    project_id: String,
    parent_version: &ModrinthVersion,
    mc: &str,
    loaders: &[String],
    installed: &HashMap<String, String>,
    seen: &mut HashSet<String>,
    q: &mut VecDeque<ModrinthVersion>,
    deps: &mut Vec<ResolvedDep>,
    fetch_latest_for_project: &F1,
) -> Result<(), ModrinthError>
where
    F1: Fn(String, String, Vec<String>) -> Fut1 + Sync,
    Fut1: std::future::Future<Output = Result<Option<ModrinthVersion>, ModrinthError>> + Send,
{
    match d.dependency_type {
        DepKind::Embedded => {
            deps.push(ResolvedDep {
                kind: DepKind::Embedded,
                project_id,
                project_title: String::new(),
                version: None,
                already_satisfied: true,
                is_new_download: false,
            });
        }
        DepKind::Incompatible => {
            if installed.contains_key(&project_id) {
                return Err(ModrinthError::DependencyConflict {
                    conflicting_project_id: project_id,
                    requested_by: parent_version.project_id.clone(),
                });
            }
            deps.push(ResolvedDep {
                kind: DepKind::Incompatible,
                project_id,
                project_title: String::new(),
                version: None,
                already_satisfied: false,
                is_new_download: false,
            });
        }
        DepKind::Optional => {
            let satisfied = installed.contains_key(&project_id);
            deps.push(ResolvedDep {
                kind: DepKind::Optional,
                project_id,
                project_title: String::new(),
                version: None,
                already_satisfied: satisfied,
                is_new_download: false,
            });
        }
        DepKind::Required => {
            if seen.contains(&project_id) {
                deps.push(ResolvedDep {
                    kind: DepKind::Required,
                    project_id,
                    project_title: String::new(),
                    version: None,
                    already_satisfied: true,
                    is_new_download: false,
                });
                return Ok(());
            }
            seen.insert(project_id.clone());
            let chosen = fetch_latest_for_project(
                project_id.clone(),
                mc.to_string(),
                loaders.to_vec(),
            )
            .await?;
            let Some(version) = chosen else {
                return Err(ModrinthError::NoCompatibleVersion {
                    project_id,
                    mc: mc.to_string(),
                    loaders: loaders.to_vec(),
                });
            };
            q.push_back(version.clone());
            deps.push(ResolvedDep {
                kind: DepKind::Required,
                project_id,
                project_title: String::new(),
                version: Some(version),
                already_satisfied: false,
                is_new_download: true,
            });
        }
    }
    Ok(())
}

// ============================================================================
// === Tests -- synthetic dep graphs, no HTTP                              ===
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::types::{ModrinthFile, ModrinthHashes};

    fn mk_file(name: &str, size: u64) -> ModrinthFile {
        ModrinthFile {
            url: format!("https://cdn.modrinth.com/{name}"),
            filename: name.to_string(),
            primary: true,
            size,
            hashes: ModrinthHashes { sha1: "a".into(), sha512: "b".into() },
        }
    }

    fn mk_version(id: &str, project_id: &str, file_size: u64, deps: Vec<ModrinthDep>) -> ModrinthVersion {
        ModrinthVersion {
            id: id.into(),
            project_id: project_id.into(),
            name: format!("{project_id}-{id}"),
            version_number: "0.0.1".into(),
            version_type: "release".into(),
            game_versions: vec!["1.20.4".into()],
            loaders: vec!["fabric".into()],
            downloads: 0,
            date_published: "2026-01-01T00:00:00Z".into(),
            dependencies: deps,
            files: vec![mk_file(&format!("{project_id}.jar"), file_size)],
        }
    }

    fn dep(pid: &str, kind: DepKind) -> ModrinthDep {
        ModrinthDep {
            project_id: Some(pid.to_string()),
            version_id: None,
            file_name: None,
            dependency_type: kind,
        }
    }

    async fn never_fetch_by_id(_v: String) -> Result<ModrinthVersion, ModrinthError> {
        panic!("fetch_version_by_id should not be called in this test")
    }

    // 1. Linear chain: root(A) -> required(B). Result: 1 dep, 1 file new.
    #[tokio::test]
    async fn test_linear_chain() {
        let b = mk_version("b1", "B", 100, vec![]);
        let root = mk_version("a1", "A", 200, vec![dep("B", DepKind::Required)]);

        let installed = HashMap::<String, String>::new();
        let g = resolve_required_deps(
            root, "1.20.4", &["fabric".into()], &installed,
            |pid, _mc, _l| {
                let b = b.clone();
                async move {
                    if pid == "B" { Ok(Some(b)) } else { Ok(None) }
                }
            },
            never_fetch_by_id,
        ).await.unwrap();

        assert_eq!(g.deps.len(), 1);
        assert_eq!(g.deps[0].project_id, "B");
        assert!(g.deps[0].is_new_download);
        assert_eq!(g.total_new_files, 1);
        assert_eq!(g.total_new_bytes, 100);
    }

    // 2. Diamond: root(A) -> required(B), required(C); B->D required, C->D required.
    // Result: B, C, D unique downloads — D appears twice in deps but is_new_download=true once.
    #[tokio::test]
    async fn test_diamond_dedupe() {
        let d = mk_version("d1", "D", 50, vec![]);
        let b = mk_version("b1", "B", 100, vec![dep("D", DepKind::Required)]);
        let c = mk_version("c1", "C", 200, vec![dep("D", DepKind::Required)]);
        let root = mk_version("a1", "A", 300, vec![dep("B", DepKind::Required), dep("C", DepKind::Required)]);

        let installed = HashMap::<String, String>::new();
        let g = resolve_required_deps(
            root, "1.20.4", &["fabric".into()], &installed,
            |pid, _mc, _l| {
                let (b, c, d) = (b.clone(), c.clone(), d.clone());
                async move {
                    match pid.as_str() {
                        "B" => Ok(Some(b)),
                        "C" => Ok(Some(c)),
                        "D" => Ok(Some(d)),
                        _ => Ok(None),
                    }
                }
            },
            never_fetch_by_id,
        ).await.unwrap();

        // BFS layer order: root visits B and C; then B visits D (new) then C visits D (already seen).
        let new_pids: Vec<&str> = g.deps.iter()
            .filter(|d| d.is_new_download)
            .map(|d| d.project_id.as_str())
            .collect();
        assert_eq!(new_pids, vec!["B", "C", "D"]);
        assert_eq!(g.total_new_files, 3);
        assert_eq!(g.total_new_bytes, 100 + 200 + 50);
    }

    // 3. Circular: A -> B required; B -> A required. Algorithm terminates via `seen`.
    #[tokio::test]
    async fn test_circular_terminates() {
        let b = mk_version("b1", "B", 100, vec![dep("A", DepKind::Required)]);
        let root = mk_version("a1", "A", 200, vec![dep("B", DepKind::Required)]);

        let installed = HashMap::<String, String>::new();
        let g = resolve_required_deps(
            root.clone(), "1.20.4", &["fabric".into()], &installed,
            |pid, _mc, _l| {
                let (b, root_) = (b.clone(), root.clone());
                async move {
                    match pid.as_str() {
                        "B" => Ok(Some(b)),
                        "A" => Ok(Some(root_)),
                        _ => Ok(None),
                    }
                }
            },
            never_fetch_by_id,
        ).await.unwrap();

        // Critical invariant: the function returns. Bounded by `seen` so cycles cannot diverge.
        assert!(g.deps.len() <= 4, "should terminate without exploding: got {} deps", g.deps.len());
    }

    // 4. Incompatible-with-installed: A -> incompatible(B), B in ledger -> DependencyConflict.
    #[tokio::test]
    async fn test_incompatible_with_installed() {
        let root = mk_version("a1", "A", 100, vec![dep("B", DepKind::Incompatible)]);
        let mut installed = HashMap::new();
        installed.insert("B".to_string(), "old-version".to_string());

        let r = resolve_required_deps(
            root, "1.20.4", &["fabric".into()], &installed,
            |_pid, _mc, _l| async { Ok(None) },
            never_fetch_by_id,
        ).await;

        match r {
            Err(ModrinthError::DependencyConflict { conflicting_project_id, requested_by }) => {
                assert_eq!(conflicting_project_id, "B");
                assert_eq!(requested_by, "A");
            }
            other => panic!("expected DependencyConflict, got {other:?}"),
        }
    }

    // 5. Optional collected, never queued. Root A -> optional(C). Algorithm should NOT call fetch_latest.
    #[tokio::test]
    async fn test_optional_collected_not_installed() {
        let root = mk_version("a1", "A", 100, vec![dep("C", DepKind::Optional)]);
        let installed = HashMap::<String, String>::new();
        let g = resolve_required_deps(
            root, "1.20.4", &["fabric".into()], &installed,
            |_pid, _mc, _l| async { panic!("fetch_latest must not be called for optional deps") },
            never_fetch_by_id,
        ).await.unwrap();

        assert_eq!(g.deps.len(), 1);
        assert_eq!(g.deps[0].kind, DepKind::Optional);
        assert!(!g.deps[0].is_new_download);
        assert_eq!(g.total_new_files, 0);
    }

    // 6. Embedded skipped. Root A -> embedded(E).
    #[tokio::test]
    async fn test_embedded_skipped() {
        let root = mk_version("a1", "A", 100, vec![dep("E", DepKind::Embedded)]);
        let installed = HashMap::<String, String>::new();
        let g = resolve_required_deps(
            root, "1.20.4", &["fabric".into()], &installed,
            |_pid, _mc, _l| async { panic!("fetch_latest must not be called for embedded deps") },
            never_fetch_by_id,
        ).await.unwrap();

        assert_eq!(g.deps.len(), 1);
        assert_eq!(g.deps[0].kind, DepKind::Embedded);
        assert!(g.deps[0].already_satisfied);
        assert_eq!(g.total_new_files, 0);
    }

    // 7. Required already in ledger -> marked already_satisfied, not enqueued.
    #[tokio::test]
    async fn test_already_satisfied_skipped() {
        let root = mk_version("a1", "A", 100, vec![dep("B", DepKind::Required)]);
        let mut installed = HashMap::new();
        installed.insert("B".to_string(), "v-old".to_string());

        let g = resolve_required_deps(
            root, "1.20.4", &["fabric".into()], &installed,
            |_pid, _mc, _l| async { panic!("must not fetch already-installed dep") },
            never_fetch_by_id,
        ).await.unwrap();

        assert_eq!(g.deps.len(), 1);
        assert_eq!(g.deps[0].project_id, "B");
        assert!(g.deps[0].already_satisfied);
        assert!(!g.deps[0].is_new_download);
        assert_eq!(g.total_new_files, 0);
    }

    // 8. Required dep with NO compatible version -> NoCompatibleVersion.
    #[tokio::test]
    async fn test_no_compatible_version() {
        let root = mk_version("a1", "A", 100, vec![dep("B", DepKind::Required)]);
        let installed = HashMap::<String, String>::new();
        let r = resolve_required_deps(
            root, "1.20.4", &["fabric".into()], &installed,
            |_pid, _mc, _l| async { Ok(None) },
            never_fetch_by_id,
        ).await;
        assert!(matches!(r, Err(ModrinthError::NoCompatibleVersion { .. })), "got {r:?}");
    }

    // Q2: Dep with version_id only (no project_id) -- fetch_version_by_id called and resolved project_id deduped.
    #[tokio::test]
    async fn test_q2_version_id_only_dep() {
        let b_version = mk_version("v-pinned", "B", 100, vec![]);
        let root = mk_version("a1", "A", 200, vec![ModrinthDep {
            project_id: None,
            version_id: Some("v-pinned".into()),
            file_name: None,
            dependency_type: DepKind::Required,
        }]);
        let installed = HashMap::<String, String>::new();
        let g = resolve_required_deps(
            root, "1.20.4", &["fabric".into()], &installed,
            |pid, _mc, _l| {
                let b = b_version.clone();
                async move { if pid == "B" { Ok(Some(b)) } else { Ok(None) } }
            },
            |vid| {
                let b = b_version.clone();
                async move {
                    assert_eq!(vid, "v-pinned");
                    Ok(b)
                }
            },
        ).await.unwrap();

        assert_eq!(g.deps.len(), 1);
        assert_eq!(g.deps[0].project_id, "B");
        assert!(g.deps[0].is_new_download);
    }
}
