//! `inheritsFrom` chain resolution.
//!
//! PURE SYNCHRONOUS function. Takes a `&HashMap<String, VersionJson>` that
//! the caller has pre-populated with every parent the chain might reach. No
//! I/O, no network, no runtime blocking of any kind. The caller (02-06) walks
//! the chain in its own concurrent context BEFORE calling this function,
//! fetching each parent's JSON via `MojangClient::fetch_version_json` and
//! populating the map.
//!
//! Recursive merge with cycle detection and a max depth of 3.
//! See 02-RESEARCH.md §7 for the merge rules.
//!
//! Returns a `ResolvedVersion` (post-merge invariant: `asset_index`, `assets`,
//! `downloads` all populated; `root_id` set to the inheritsFrom chain's
//! terminal id). The launcher pipeline reads ONLY `ResolvedVersion`; the type
//! system enforces "resolve before read" at compile time.

use std::collections::{HashMap, HashSet};

use super::natives::maven_group_artifact;
use super::types::{Arguments, ResolvedVersion, VersionJson};
use crate::error::AppError;

/// Maximum recursive depth for inheritsFrom chains. Enforcement prevents
/// infinite loops from malformed modpack JSONs.
pub const MAX_INHERITS_DEPTH: u32 = 3;

/// Resolve a `VersionJson` by merging in its parents via `inheritsFrom`.
///
/// Returns `ResolvedVersion` (post-merge invariant: `asset_index`,
/// `assets`, `downloads` all populated; `root_id` set to the inheritsFrom
/// chain's terminal id).
///
/// Merge semantics:
/// - `id`, `version_type`, `main_class`, `release_time`, `time`: child wins
/// - `asset_index`, `assets`, `downloads`: child wins IF set, else parent IF set,
///   else `Err(InheritsFromMissingRequired { field })`
/// - `libraries`: parent ++ child, deduplicated by `group:artifact`, child's version wins
/// - `arguments.game` / `arguments.jvm`: parent ++ child concatenation
/// - `minecraft_arguments`, `java_version`, `logging`, `compliance_level`,
///   `minimum_launcher_version`: child wins IF Some, else parent IF Some, else None
///
/// `parents` MUST contain every id in the `inheritsFrom` chain (caller
/// pre-populates). Any missing parent yields
/// `AppError::InheritsFromParentMissing`.
pub fn resolve_inherits(
    child: &VersionJson,
    parents: &HashMap<String, VersionJson>,
) -> Result<ResolvedVersion, AppError> {
    let mut seen = HashSet::new();
    seen.insert(child.id.clone());
    let merged = resolve_inner(child.clone(), parents, &mut seen, 0)?;
    // Track root_id: walk the original child's chain explicitly, since
    // `merged.inherits_from` is set to None at the end of recursion.
    let root_id = compute_root_id(child, parents);
    // Promote merged.{asset_index,assets,downloads} from Option to required.
    let asset_index = merged
        .asset_index
        .ok_or_else(|| AppError::InheritsFromMissingRequired {
            field: "asset_index".into(),
            version_id: child.id.clone(),
        })?;
    let assets = merged
        .assets
        .ok_or_else(|| AppError::InheritsFromMissingRequired {
            field: "assets".into(),
            version_id: child.id.clone(),
        })?;
    let downloads = merged
        .downloads
        .ok_or_else(|| AppError::InheritsFromMissingRequired {
            field: "downloads".into(),
            version_id: child.id.clone(),
        })?;
    Ok(ResolvedVersion {
        id: merged.id,
        root_id,
        version_type: merged.version_type,
        main_class: merged.main_class,
        asset_index,
        assets,
        downloads,
        libraries: merged.libraries,
        java_version: merged.java_version,
        logging: merged.logging,
        compliance_level: merged.compliance_level,
        minimum_launcher_version: merged.minimum_launcher_version,
        release_time: merged.release_time,
        time: merged.time,
        arguments: merged.arguments,
        minecraft_arguments: merged.minecraft_arguments,
    })
}

/// Walk the `inheritsFrom` chain and return the TERMINAL id (the leaf
/// parent that does NOT declare `inheritsFrom`). For a vanilla input
/// (`child.inherits_from == None`), returns `child.id` verbatim.
///
/// This is a best-effort walk: it caps iterations at `MAX_INHERITS_DEPTH + 1`
/// as a safety belt; `resolve_inner` already validates depth + cycles and
/// errors before this function is called.
fn compute_root_id(child: &VersionJson, parents: &HashMap<String, VersionJson>) -> String {
    let mut current = child;
    for _ in 0..(MAX_INHERITS_DEPTH as usize + 1) {
        match &current.inherits_from {
            None => return current.id.clone(),
            Some(parent_id) => match parents.get(parent_id) {
                Some(p) => current = p,
                None => return current.id.clone(), // best-effort
            },
        }
    }
    current.id.clone()
}

fn resolve_inner(
    child: VersionJson,
    parents: &HashMap<String, VersionJson>,
    seen: &mut HashSet<String>,
    depth: u32,
) -> Result<VersionJson, AppError> {
    let Some(parent_id) = child.inherits_from.clone() else {
        return Ok(child);
    };
    if !seen.insert(parent_id.clone()) {
        return Err(AppError::InheritsFromCycle(parent_id));
    }
    // depth counts how many parent-hops have been made so far to reach
    // the current node. `child` itself is at hop `depth`. If `child` has
    // another parent, that would become hop `depth + 1`. Allow up to
    // MAX_INHERITS_DEPTH hops total (child + MAX parents). The 4-node
    // chain (A→B→C→D) requires 3 hops; with MAX=3 we allow at most
    // MAX-1 additional hops from the root, so we reject when
    // depth >= MAX_INHERITS_DEPTH - 1 and a further parent exists.
    // Equivalently: reject when the NEXT hop index equals MAX_INHERITS_DEPTH.
    if depth + 1 >= MAX_INHERITS_DEPTH {
        return Err(AppError::InheritsFromDepthExceeded {
            current: parent_id,
            max: MAX_INHERITS_DEPTH,
        });
    }
    let parent = parents
        .get(&parent_id)
        .ok_or_else(|| AppError::InheritsFromParentMissing(parent_id.clone()))?
        .clone();
    let resolved_parent = resolve_inner(parent, parents, seen, depth + 1)?;
    Ok(merge(resolved_parent, child))
}

/// Merge `child` onto `parent`. Returns a still-Optional `VersionJson`; the
/// outer `resolve_inherits` is what promotes Option<asset_index/assets/
/// downloads> into the required ResolvedVersion fields.
///
/// Semantics:
/// - `main_class`, `id`, `version_type`, `release_time`, `time`: child wins
/// - `asset_index`, `assets`, `downloads`: child wins IF set, else parent IF set,
///   else None (resolve_inherits will then convert None into
///   `InheritsFromMissingRequired`)
/// - `libraries`: parent ++ child, deduplicated by `group:artifact`, child's version wins
/// - `arguments.game` / `arguments.jvm`: parent ++ child (concatenation, parent first)
/// - `minecraft_arguments`, `java_version`, `logging`, `compliance_level`,
///   `minimum_launcher_version`: child wins IF Some, else parent IF Some, else None
fn merge(parent: VersionJson, child: VersionJson) -> VersionJson {
    // Library dedup: parent entries first; child overrides by group:artifact
    let parent_keys: Vec<String> = parent
        .libraries
        .iter()
        .map(|l| maven_group_artifact(&l.name).to_string())
        .collect();
    let mut libs = parent.libraries.clone();
    for cl in &child.libraries {
        let key = maven_group_artifact(&cl.name);
        if let Some(idx) = parent_keys.iter().position(|k| k == key) {
            libs[idx] = cl.clone(); // child version wins
        } else {
            libs.push(cl.clone());
        }
    }

    // Arguments: parent ++ child concatenation
    let arguments = match (parent.arguments.clone(), child.arguments.clone()) {
        (Some(p), Some(c)) => {
            let mut game = p.game.clone();
            game.extend(c.game.clone());
            let mut jvm = p.jvm.clone();
            jvm.extend(c.jvm.clone());
            Some(Arguments { game, jvm })
        }
        (None, Some(c)) => Some(c),
        (Some(p), None) => Some(p),
        (None, None) => None,
    };

    VersionJson {
        id: child.id,
        version_type: child.version_type,
        main_class: child.main_class,
        // child wins IF Some, else parent IF Some, else None (resolve_inherits
        // will then convert None into InheritsFromMissingRequired):
        asset_index: child.asset_index.or(parent.asset_index),
        assets: child.assets.or(parent.assets),
        downloads: child.downloads.or(parent.downloads),
        libraries: libs,
        java_version: child.java_version.or(parent.java_version),
        logging: child.logging.or(parent.logging),
        compliance_level: child.compliance_level.or(parent.compliance_level),
        minimum_launcher_version: child
            .minimum_launcher_version
            .or(parent.minimum_launcher_version),
        release_time: child.release_time,
        time: child.time,
        arguments,
        minecraft_arguments: child.minecraft_arguments.or(parent.minecraft_arguments),
        inherits_from: None, // resolved chain — no further parent
    }
}

// ----- tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::types::AssetIndex;
    use super::*;
    use crate::error::AppError;
    use std::collections::HashMap;

    /// Build a VersionJson for tests via JSON parsing — mirrors the pattern
    /// in `tests/mojang_protocol.rs::vjson_stub` but local to this file (no
    /// cross-crate test-helper coupling). Includes the demoted fields by
    /// default; helpers below override them to None as needed for loader
    /// shapes.
    fn vjson(id: &str) -> VersionJson {
        serde_json::from_str(&format!(
            r#"{{
                "id": "{id}",
                "type": "release",
                "mainClass": "net.minecraft.client.main.Main",
                "assetIndex": {{
                    "id": "x",
                    "sha1": "0000000000000000000000000000000000000000",
                    "size": 0,
                    "totalSize": 0,
                    "url": "http://example.com/"
                }},
                "assets": "x",
                "downloads": {{}},
                "libraries": [],
                "releaseTime": "2020-01-01T00:00:00Z",
                "time": "2020-01-01T00:00:00Z"
            }}"#
        ))
        .unwrap()
    }

    /// Real Fabric/Quilt loader on-disk JSON shape: NO assetIndex, NO assets,
    /// NO downloads, NO javaVersion, NO logging. Has inheritsFrom set.
    /// This is the production-shape fixture — DO NOT add the demoted fields
    /// to it. If a test fails because parse rejects this shape, the fix is
    /// the type system (Option-demote those fields), NOT fattening the
    /// fixture. See plan 08.3-01 explicit forbid for the rationale.
    fn loader_vjson(id: &str, parent_id: &str) -> VersionJson {
        serde_json::from_str(&format!(
            r#"{{
                "id": "{id}",
                "inheritsFrom": "{parent_id}",
                "type": "release",
                "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
                "arguments": {{ "game": [], "jvm": [] }},
                "libraries": [],
                "releaseTime": "2025-08-01T12:00:00Z",
                "time": "2025-08-01T12:00:00Z"
            }}"#
        ))
        .unwrap()
    }

    /// GAP-LAUNCH-PARSE-08 regression (Phase 8.3 round-3 BLOCKER):
    /// production-shaped loader JSON (NO assetIndex/assets/downloads — those
    /// are inherited from vanilla via `inheritsFrom`) MUST resolve cleanly
    /// when the vanilla parent is in the parents map.
    #[test]
    fn test_resolve_inherits_loader_shape_hydrates_from_vanilla_parent() {
        // Loader child — production shape: no assetIndex/assets/downloads.
        let loader = loader_vjson("fabric-loader-0.19.2-1.20.4", "1.20.4");
        assert!(
            loader.asset_index.is_none(),
            "loader_vjson fixture must lack asset_index — this proves the test fixture matches \
             production loader JSONs on disk (NOT the 8.2-01 fattened shape)"
        );
        assert!(loader.assets.is_none());
        assert!(loader.downloads.is_none());

        // Vanilla parent — has the demoted fields.
        let vanilla = vjson("1.20.4");
        assert!(
            vanilla.asset_index.is_some(),
            "vanilla fixture must declare asset_index"
        );

        let mut parents = HashMap::new();
        parents.insert("1.20.4".to_string(), vanilla);

        let resolved = resolve_inherits(&loader, &parents)
            .expect("loader+vanilla merge must produce ResolvedVersion");

        // ResolvedVersion fields are non-Option — they're hydrated from the parent.
        assert_eq!(
            resolved.asset_index.id, "x",
            "asset_index hydrated from vanilla parent"
        );
        assert_eq!(resolved.assets, "x", "assets hydrated from vanilla parent");

        // id remains the loader id (MultiMC convention — used in ${version_name})
        assert_eq!(resolved.id, "fabric-loader-0.19.2-1.20.4");
        // root_id is the vanilla MC id (where the JAR lives on disk —
        // SECONDARY-BUG-FIX surface area)
        assert_eq!(resolved.root_id, "1.20.4");
        // mainClass: child wins
        assert_eq!(
            resolved.main_class,
            "net.fabricmc.loader.impl.launch.knot.KnotClient"
        );
    }

    /// Existing semantic must be preserved: when both child and parent have
    /// asset_index, the child's value wins (not silently overridden by parent).
    #[test]
    fn test_resolve_inherits_child_asset_index_wins_when_both_set() {
        let mut child = vjson("child");
        // child gets a SPECIFIC asset_index id we'll assert on
        child.asset_index = Some(AssetIndex {
            id: "child-aid".into(),
            sha1: "1111111111111111111111111111111111111111".into(),
            size: 0,
            total_size: 0,
            url: "http://example.com/child".into(),
        });
        child.assets = Some("child-assets".into());
        child.inherits_from = Some("parent".into());

        let mut parent = vjson("parent");
        parent.asset_index = Some(AssetIndex {
            id: "parent-aid".into(),
            sha1: "2222222222222222222222222222222222222222".into(),
            size: 0,
            total_size: 0,
            url: "http://example.com/parent".into(),
        });
        parent.assets = Some("parent-assets".into());

        let mut parents = HashMap::new();
        parents.insert("parent".to_string(), parent);

        let resolved = resolve_inherits(&child, &parents).expect("must resolve");
        assert_eq!(
            resolved.asset_index.id, "child-aid",
            "child wins when both child and parent declare asset_index"
        );
        assert_eq!(
            resolved.assets, "child-assets",
            "child wins when both child and parent declare assets"
        );
    }

    /// When neither child nor any ancestor declares asset_index, resolve_inherits
    /// must surface InheritsFromMissingRequired (NOT panic on a missing field).
    #[test]
    fn test_resolve_inherits_errors_when_required_field_missing_from_chain() {
        // A vanilla-id parent with `inherits_from: None` but ALSO missing
        // asset_index/assets/downloads — the degenerate case (truncated
        // install / hand-rolled JSON) the typed error variant exists for.
        let mut bare_parent = vjson("bare-parent");
        bare_parent.asset_index = None;
        bare_parent.assets = None;
        bare_parent.downloads = None;
        bare_parent.inherits_from = None;

        // Child references bare_parent as its inheritsFrom (loader shape:
        // also missing the demoted fields).
        let bare_child = loader_vjson("bare-child", "bare-parent");

        let mut parents = HashMap::new();
        parents.insert("bare-parent".to_string(), bare_parent);

        let result = resolve_inherits(&bare_child, &parents);
        match result {
            Err(AppError::InheritsFromMissingRequired { field, version_id }) => {
                assert_eq!(
                    field, "asset_index",
                    "first missing required field reported: asset_index"
                );
                assert_eq!(
                    version_id, "bare-child",
                    "version_id in error is the originating child"
                );
            }
            other => panic!(
                "expected Err(InheritsFromMissingRequired {{ field: \"asset_index\", .. }}); got {other:?}"
            ),
        }
    }
}
