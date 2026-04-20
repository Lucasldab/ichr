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

use std::collections::{HashMap, HashSet};

use super::natives::maven_group_artifact;
use super::types::{Arguments, VersionJson};
use crate::error::AppError;

/// Maximum recursive depth for inheritsFrom chains. Enforcement prevents
/// infinite loops from malformed modpack JSONs.
pub const MAX_INHERITS_DEPTH: u32 = 3;

/// Resolve a `VersionJson` by merging in its parents via `inheritsFrom`.
///
/// `parents` MUST contain every id that appears in the `inherits_from` chain
/// starting from `child`. Any missing parent yields
/// `AppError::InheritsFromParentMissing`. Callers are expected to pre-populate
/// the map (fetching each parent's JSON upstream) before calling this function.
pub fn resolve_inherits(
    child: &VersionJson,
    parents: &HashMap<String, VersionJson>,
) -> Result<VersionJson, AppError> {
    let mut seen = HashSet::new();
    seen.insert(child.id.clone());
    resolve_inner(child.clone(), parents, &mut seen, 0)
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

/// Merge `child` onto `parent` following the semantic rules from 02-RESEARCH.md §7:
/// - `main_class`: child wins
/// - `asset_index`, `assets`, `downloads`: child wins (struct fields are non-Optional)
/// - `libraries`: parent ++ child, deduplicated by `group:artifact`, child's version wins
/// - `arguments.game` / `arguments.jvm`: parent ++ child (concatenation, parent first)
/// - `minecraft_arguments`: child wins IF set
/// - `java_version`, `logging`: child wins IF set
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
        asset_index: child.asset_index,
        assets: child.assets,
        downloads: child.downloads,
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
