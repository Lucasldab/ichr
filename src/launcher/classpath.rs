//! Classpath string builder — OS-correct separator, rules-filtered library
//! walk, client.jar placed last.
//!
//! PITFALLS.md Pitfall 14: use `cfg!(target_os = "windows")` — NEVER
//! `std::path::MAIN_SEPARATOR`, which is the path component separator (`/`
//! or `\`) and is a DIFFERENT character from the classpath separator (`:` or
//! `;`).

use std::path::PathBuf;

use crate::error::AppError;
use crate::mojang::natives::needs_native_extraction;
use crate::mojang::rules::{evaluate_rules, RuleContext};
use crate::mojang::types::ResolvedVersion;
use crate::persistence::paths::AppPaths;

/// Classpath entry separator for the HOST OS (not the rule-context OS).
///
/// - Linux / macOS: `':'`
/// - Windows: `';'`
///
/// This is the single authoritative source of the separator throughout the
/// launcher. Never use `std::path::MAIN_SEPARATOR` for this purpose.
pub fn classpath_separator() -> char {
    if cfg!(target_os = "windows") {
        ';'
    } else {
        ':'
    }
}

/// Build the colon/semicolon-separated classpath string for `version`.
///
/// Walk order:
/// 1. `version.libraries` — filtered by `evaluate_rules(&lib.rules, ctx)`.
/// 2. Libraries where `needs_native_extraction` is `true` are skipped (those
///    are extracted into the per-instance natives dir, not put on the classpath).
/// 3. `paths.version_jar(&version.root_id)` appended LAST (standard ordering).
///    NOTE: uses `root_id` (vanilla MC id at the inheritsFrom chain root)
///    rather than `id` (loader id post-merge). Phase 6's loader install
///    writes only `{loader-id}.json` — never a `{loader-id}.jar` — so
///    using `id` here would produce a classpath entry pointing at a
///    non-existent file and the JVM would `ClassNotFoundException`.
///
/// Returns an `AppError` only if path construction itself fails (currently
/// infallible, but returns `Result` to allow future validation).
pub fn build_classpath(
    version: &ResolvedVersion,
    ctx: &RuleContext,
    paths: &AppPaths,
) -> Result<String, AppError> {
    let mut entries: Vec<PathBuf> = Vec::new();

    for lib in &version.libraries {
        if !evaluate_rules(&lib.rules, ctx) {
            continue;
        }
        if needs_native_extraction(lib) {
            continue;
        }
        if let Some(artifact) = &lib.downloads.artifact {
            entries.push(paths.library_path(&artifact.path));
        }
    }

    // Client JAR is always last — standard Minecraft launcher ordering.
    // SECONDARY-BUG-FIX (Phase 8.3 GAP-LAUNCH-PARSE-08): the JAR lives at
    // versions/{root-vanilla-id}/{root-vanilla-id}.jar — Phase 6 loader
    // install never writes a loader JAR (only the loader JSON). Pre-8.3
    // code used &version.id which is the loader id post-merge, pointing
    // at a non-existent file and causing ClassNotFoundException at JVM
    // spawn. ResolvedVersion exposes root_id (the inheritsFrom chain's
    // terminal vanilla id) precisely for this.
    entries.push(paths.version_jar(&version.root_id));

    let sep = classpath_separator().to_string();
    let parts: Vec<String> = entries
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    Ok(parts.join(&sep))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::platform::{Arch, OsName};
    use crate::mojang::inherits::resolve_inherits;
    use crate::mojang::types::{ResolvedVersion, VersionJson};
    use crate::persistence::paths::AppPaths;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn paths_for_test() -> AppPaths {
        AppPaths::with_roots(
            PathBuf::from("/data"),
            PathBuf::from("/config"),
            PathBuf::from("/cache"),
        )
    }

    fn load_resolved(path: &str) -> ResolvedVersion {
        let raw = std::fs::read_to_string(path).expect("fixture present");
        let v: VersionJson = serde_json::from_str(&raw).expect("fixture parses");
        resolve_inherits(&v, &HashMap::new())
            .expect("vanilla fixture has all required fields and resolves cleanly")
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_classpath_separator_linux() {
        assert_eq!(classpath_separator(), ':');
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_classpath_separator_windows() {
        assert_eq!(classpath_separator(), ';');
    }

    #[test]
    fn test_client_jar_is_last() {
        let v = load_resolved("tests/fixtures/mojang/version_1_21_4.json");
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cp = build_classpath(&v, &ctx, &paths_for_test()).unwrap();
        let last = cp.rsplit(':').next().unwrap();
        assert!(
            last.ends_with(&format!("{}.jar", v.root_id)),
            "client jar should be last entry; got last entry: {last}"
        );
    }

    #[test]
    fn test_classpath_excludes_native_extraction_libs() {
        // version_1_12_2 has legacy classifier-style natives
        let v = load_resolved("tests/fixtures/mojang/version_1_12_2.json");
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cp = build_classpath(&v, &ctx, &paths_for_test()).unwrap();
        // Classifier-native jars have "natives" somewhere in their path and
        // must NOT appear on the classpath.
        for entry in cp.split(':') {
            assert!(
                !entry.contains("natives-linux"),
                "classifier-native lib must not be on classpath; found: {entry}"
            );
        }
    }

    #[test]
    fn test_classpath_nonempty_for_modern_version() {
        let v = load_resolved("tests/fixtures/mojang/version_1_21_4.json");
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cp = build_classpath(&v, &ctx, &paths_for_test()).unwrap();
        assert!(
            !cp.is_empty(),
            "classpath must not be empty for a real version"
        );
        // Must contain at least the version jar entry (root_id == id for vanilla).
        assert!(
            cp.contains(&v.root_id),
            "classpath must include the version id in the jar path"
        );
    }

    #[test]
    fn test_classpath_nonempty_for_legacy_version() {
        let v = load_resolved("tests/fixtures/mojang/version_1_12_2.json");
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cp = build_classpath(&v, &ctx, &paths_for_test()).unwrap();
        assert!(
            !cp.is_empty(),
            "classpath must not be empty for a legacy version"
        );
        assert!(
            cp.contains(&v.root_id),
            "classpath must include the version id in the jar path"
        );
    }
}
