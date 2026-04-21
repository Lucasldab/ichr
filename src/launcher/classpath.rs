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
use crate::mojang::types::VersionJson;
use crate::persistence::paths::AppPaths;

/// Classpath entry separator for the HOST OS (not the rule-context OS).
///
/// - Linux / macOS: `':'`
/// - Windows: `';'`
///
/// This is the single authoritative source of the separator throughout the
/// launcher. Never use `std::path::MAIN_SEPARATOR` for this purpose.
pub fn classpath_separator() -> char {
    if cfg!(target_os = "windows") { ';' } else { ':' }
}

/// Build the colon/semicolon-separated classpath string for `version`.
///
/// Walk order:
/// 1. `version.libraries` — filtered by `evaluate_rules(&lib.rules, ctx)`.
/// 2. Libraries where `needs_native_extraction` is `true` are skipped (those
///    are extracted into the per-instance natives dir, not put on the classpath).
/// 3. `paths.version_jar(&version.id)` appended LAST (standard ordering).
///
/// Returns an `AppError` only if path construction itself fails (currently
/// infallible, but returns `Result` to allow future validation).
pub fn build_classpath(
    version: &VersionJson,
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
    entries.push(paths.version_jar(&version.id));

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
    use crate::mojang::types::VersionJson;
    use crate::persistence::paths::AppPaths;
    use std::path::PathBuf;

    fn paths_for_test() -> AppPaths {
        AppPaths::with_roots(
            PathBuf::from("/data"),
            PathBuf::from("/config"),
            PathBuf::from("/cache"),
        )
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
        let raw =
            std::fs::read_to_string("tests/fixtures/mojang/version_1_21_4.json").unwrap();
        let v: VersionJson = serde_json::from_str(&raw).unwrap();
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cp = build_classpath(&v, &ctx, &paths_for_test()).unwrap();
        let last = cp.rsplit(':').next().unwrap();
        assert!(
            last.ends_with(&format!("{}.jar", v.id)),
            "client jar should be last entry; got last entry: {last}"
        );
    }

    #[test]
    fn test_classpath_excludes_native_extraction_libs() {
        // version_1_12_2 has legacy classifier-style natives
        let raw =
            std::fs::read_to_string("tests/fixtures/mojang/version_1_12_2.json").unwrap();
        let v: VersionJson = serde_json::from_str(&raw).unwrap();
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
        let raw =
            std::fs::read_to_string("tests/fixtures/mojang/version_1_21_4.json").unwrap();
        let v: VersionJson = serde_json::from_str(&raw).unwrap();
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cp = build_classpath(&v, &ctx, &paths_for_test()).unwrap();
        assert!(!cp.is_empty(), "classpath must not be empty for a real version");
        // Must contain at least the version jar entry.
        assert!(cp.contains(&v.id), "classpath must include the version id in the jar path");
    }

    #[test]
    fn test_classpath_nonempty_for_legacy_version() {
        let raw =
            std::fs::read_to_string("tests/fixtures/mojang/version_1_12_2.json").unwrap();
        let v: VersionJson = serde_json::from_str(&raw).unwrap();
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cp = build_classpath(&v, &ctx, &paths_for_test()).unwrap();
        assert!(!cp.is_empty(), "classpath must not be empty for a legacy version");
        assert!(cp.contains(&v.id), "classpath must include the version id in the jar path");
    }
}
