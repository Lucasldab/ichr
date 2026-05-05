//! BFS dependency resolution over Modrinth project versions.
//!
//! TDD RED: stub signature; full implementation lands in the GREEN commit.

use std::collections::HashMap;

use crate::mods::error::ModrinthError;
use crate::mods::types::{ModrinthVersion, ResolvedDep};

/// Result of dependency resolution.
#[derive(Debug, Clone)]
pub struct ResolvedDepGraph {
    pub root: ModrinthVersion,
    pub deps: Vec<ResolvedDep>,
    pub total_new_bytes: u64,
    pub total_new_files: usize,
}

/// BFS resolve of the root version's required dependencies.
///
/// RED-phase stub: returns an empty graph. GREEN replaces with the BFS body.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_required_deps<F1, Fut1, F2, Fut2>(
    root: ModrinthVersion,
    _mc: &str,
    _loaders: &[String],
    _installed: &HashMap<String, String>,
    _fetch_latest_for_project: F1,
    _fetch_version_by_id: F2,
) -> Result<ResolvedDepGraph, ModrinthError>
where
    F1: Fn(String, String, Vec<String>) -> Fut1 + Sync,
    Fut1: std::future::Future<Output = Result<Option<ModrinthVersion>, ModrinthError>> + Send,
    F2: Fn(String) -> Fut2 + Sync,
    Fut2: std::future::Future<Output = Result<ModrinthVersion, ModrinthError>> + Send,
{
    Ok(ResolvedDepGraph {
        root,
        deps: Vec::new(),
        total_new_bytes: 0,
        total_new_files: 0,
    })
}

// ============================================================================
// === Tests — synthetic dep graphs, no HTTP                               ===
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::types::{DepKind, ModrinthDep, ModrinthFile, ModrinthHashes};

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

    // 5. Optional collected, never queued. Root A -> optional(C). Algorithm should NOT call fetch_latest for C.
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
