//! Pure LaunchCommand composer — ties classpath, substitution, offline auth,
//! and version-JSON arg resolution into a single `LaunchCommand` struct.
//! No I/O, no async, no process spawning.
//!
//! The spawn layer (`src/launcher/spawn.rs`, plan 03-03) consumes `LaunchCommand`
//! and passes its fields directly to `tokio::process::Command::args`.

use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::mojang::args::{resolve_game_args, resolve_jvm_args};
use crate::mojang::rules::RuleContext;
use crate::mojang::types::ResolvedVersion;
use crate::persistence::paths::AppPaths;

use super::classpath::{build_classpath, classpath_separator};
use super::offline::{MsaAuth, OfflineAuth};
use super::substitute::{substitute_all, SubstitutionContext};

/// Legacy JVM-arg baseline for pre-1.13 versions whose version JSON has no
/// `arguments.jvm` block. These templates still contain `${...}` tokens
/// that `substitute_all` will expand.
const LEGACY_JVM_ARGS: &[&str] = &[
    "-Djava.library.path=${natives_directory}",
    "-cp",
    "${classpath}",
];

/// A fully-resolved JVM launch command — everything needed to spawn Minecraft
/// except stdio wiring and process management.
///
/// Fields are intentionally public so plan 03-03 (`spawn`) can consume them
/// without extra accessors.
#[derive(Debug, Clone)]
pub struct LaunchCommand {
    /// Path to the Java binary (e.g. `PathBuf::from("java")` for PATH lookup).
    pub java_bin: PathBuf,
    /// Fully-substituted JVM arguments (everything before the main class).
    pub jvm_args: Vec<String>,
    /// The `main_class` field from the version JSON.
    pub main_class: String,
    /// Fully-substituted game arguments (everything after the main class).
    pub game_args: Vec<String>,
}

/// Compose a `LaunchCommand` from version metadata, offline auth, paths, and
/// a rule context.
///
/// Steps:
/// 1. Build the classpath via `build_classpath`.
/// 2. Construct a `SubstitutionContext` from all inputs.
/// 3. Resolve JVM arg templates via `resolve_jvm_args`; if empty (pre-1.13),
///    fall back to `LEGACY_JVM_ARGS`.
/// 4. Resolve game arg templates via `resolve_game_args`.
/// 5. Run `substitute_all` over both sets.
/// 6. Return `LaunchCommand`.
pub fn compose(
    version: &ResolvedVersion,
    auth: &OfflineAuth,
    paths: &AppPaths,
    slug: &str,
    ctx: &RuleContext,
    java_bin: &Path,
) -> Result<LaunchCommand, AppError> {
    let classpath = build_classpath(version, ctx, paths)?;

    let sub_ctx = SubstitutionContext {
        auth_player_name: auth.username.clone(),
        auth_uuid: auth.uuid.clone(),
        auth_access_token: auth.access_token.clone(),
        auth_xuid: String::new(),
        clientid: String::new(),
        auth_xbox_user_hash: String::new(),
        user_type: auth.user_type.clone(),
        version_name: version.id.clone(),
        version_type: version.version_type.clone(),
        game_directory: paths.instance_minecraft_dir(slug),
        assets_root: paths.assets_dir(),
        assets_index_name: version.asset_index.id.clone(),
        natives_directory: paths.instance_natives_dir(slug),
        library_directory: paths.libraries_dir(),
        classpath,
        classpath_separator: classpath_separator(),
        arch: ctx.arch.mojang_str().to_string(),
        launcher_name: "mineltui".to_string(),
        launcher_version: env!("CARGO_PKG_VERSION").to_string(),
        resolution_width: String::new(),
        resolution_height: String::new(),
    };

    // JVM args: use structured arguments.jvm when present; fall back to
    // the legacy baseline for pre-1.13 versions that have no jvm block.
    let jvm_templates: Vec<String> = {
        let resolved = resolve_jvm_args(version, ctx);
        if resolved.is_empty() {
            LEGACY_JVM_ARGS.iter().map(|s| (*s).to_string()).collect()
        } else {
            resolved
        }
    };

    let jvm_args = substitute_all(&jvm_templates, &sub_ctx);
    let game_args = substitute_all(&resolve_game_args(version, ctx), &sub_ctx);

    Ok(LaunchCommand {
        java_bin: java_bin.to_path_buf(),
        jvm_args,
        main_class: version.main_class.clone(),
        game_args,
    })
}

/// Compose a `LaunchCommand` for an MSA-authenticated launch.
///
/// Identical structure to `compose` but populates `SubstitutionContext` with
/// live Minecraft session fields from `MsaAuth` instead of offline placeholders.
pub fn compose_msa(
    version: &ResolvedVersion,
    auth: &MsaAuth,
    paths: &AppPaths,
    slug: &str,
    ctx: &RuleContext,
    java_bin: &Path,
) -> Result<LaunchCommand, AppError> {
    let classpath = build_classpath(version, ctx, paths)?;

    let sub_ctx = SubstitutionContext {
        auth_player_name: auth.username.clone(),
        auth_uuid: auth.uuid.clone(),
        auth_access_token: auth.access_token.clone(),
        auth_xuid: auth.xuid.clone(),
        clientid: auth.clientid.clone(),
        auth_xbox_user_hash: auth.xbox_user_hash.clone(),
        user_type: auth.user_type.clone(),
        version_name: version.id.clone(),
        version_type: version.version_type.clone(),
        game_directory: paths.instance_minecraft_dir(slug),
        assets_root: paths.assets_dir(),
        assets_index_name: version.asset_index.id.clone(),
        natives_directory: paths.instance_natives_dir(slug),
        library_directory: paths.libraries_dir(),
        classpath,
        classpath_separator: classpath_separator(),
        arch: ctx.arch.mojang_str().to_string(),
        launcher_name: "mineltui".to_string(),
        launcher_version: env!("CARGO_PKG_VERSION").to_string(),
        resolution_width: String::new(),
        resolution_height: String::new(),
    };

    let jvm_templates: Vec<String> = {
        let resolved = resolve_jvm_args(version, ctx);
        if resolved.is_empty() {
            LEGACY_JVM_ARGS.iter().map(|s| (*s).to_string()).collect()
        } else {
            resolved
        }
    };

    let jvm_args = substitute_all(&jvm_templates, &sub_ctx);
    let game_args = substitute_all(&resolve_game_args(version, ctx), &sub_ctx);

    Ok(LaunchCommand {
        java_bin: java_bin.to_path_buf(),
        jvm_args,
        main_class: version.main_class.clone(),
        game_args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::platform::{Arch, OsName};
    use crate::launcher::offline::offline_auth;
    use crate::mojang::inherits::resolve_inherits;
    use crate::mojang::rules::RuleContext;
    use crate::mojang::types::{ResolvedVersion, VersionJson};
    use crate::persistence::paths::AppPaths;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn fixture_paths() -> AppPaths {
        AppPaths::with_roots(
            PathBuf::from("/data"),
            PathBuf::from("/config"),
            PathBuf::from("/cache"),
        )
    }

    fn load(path: &str) -> ResolvedVersion {
        let raw = std::fs::read_to_string(path).expect("fixture must be present");
        let raw_v: VersionJson = serde_json::from_str(&raw).expect("fixture must parse");
        // Vanilla fixtures declare asset_index/assets/downloads inline; an empty
        // parents map is correct because vanilla has no inheritsFrom.
        resolve_inherits(&raw_v, &HashMap::new())
            .expect("vanilla fixture has all required fields and resolves to ResolvedVersion")
    }

    #[test]
    fn test_compose_modern_1_21_4() {
        let v = load("tests/fixtures/mojang/version_1_21_4.json");
        let auth = offline_auth("TestUser");
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cmd = compose(
            &v,
            &auth,
            &fixture_paths(),
            "myslug",
            &ctx,
            Path::new("java"),
        )
        .unwrap();

        // main_class must match the version JSON field
        assert_eq!(cmd.main_class, v.main_class);

        // JVM args must contain -cp
        assert!(
            cmd.jvm_args.iter().any(|a| a == "-cp"),
            "jvm_args must contain -cp; got {:?}",
            cmd.jvm_args
        );

        // The arg after -cp must contain the version jar
        let cp_idx = cmd.jvm_args.iter().position(|a| a == "-cp").unwrap();
        let cp_val = &cmd.jvm_args[cp_idx + 1];
        assert!(
            cp_val.contains(&v.id),
            "classpath entry must contain version id; got {cp_val}"
        );

        // game args: --username TestUser
        assert!(
            cmd.game_args
                .windows(2)
                .any(|w| w[0] == "--username" && w[1] == "TestUser"),
            "game_args must contain --username TestUser; got {:?}",
            cmd.game_args
        );

        // game args: --uuid <offline uuid>
        assert!(
            cmd.game_args
                .windows(2)
                .any(|w| w[0] == "--uuid" && w[1] == auth.uuid),
            "game_args must contain --uuid {}; got {:?}",
            auth.uuid,
            cmd.game_args
        );

        // No unsubstituted ${var} tokens anywhere
        for arg in cmd.jvm_args.iter().chain(cmd.game_args.iter()) {
            assert!(
                !arg.contains("${"),
                "no unsubstituted ${{var}} tokens allowed; found in: {arg}"
            );
        }
    }

    #[test]
    fn test_compose_legacy_1_12_2() {
        let v = load("tests/fixtures/mojang/version_1_12_2.json");
        let auth = offline_auth("TestUser");
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cmd = compose(
            &v,
            &auth,
            &fixture_paths(),
            "myslug",
            &ctx,
            Path::new("java"),
        )
        .unwrap();

        // Pre-1.13: arguments.jvm absent → LEGACY_JVM_ARGS baseline used
        assert!(
            cmd.jvm_args
                .iter()
                .any(|a| a.starts_with("-Djava.library.path=")),
            "legacy versions need -Djava.library.path; got {:?}",
            cmd.jvm_args
        );
        assert!(
            cmd.jvm_args.iter().any(|a| a == "-cp"),
            "jvm_args must contain -cp; got {:?}",
            cmd.jvm_args
        );

        // No unsubstituted ${var} tokens anywhere
        for arg in cmd.jvm_args.iter().chain(cmd.game_args.iter()) {
            assert!(
                !arg.contains("${"),
                "no unsubstituted ${{var}} tokens allowed; found in: {arg}"
            );
        }
    }

    #[test]
    fn test_compose_access_token_is_offline_placeholder() {
        let v = load("tests/fixtures/mojang/version_1_21_4.json");
        let auth = offline_auth("TestUser");
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cmd = compose(
            &v,
            &auth,
            &fixture_paths(),
            "myslug",
            &ctx,
            Path::new("java"),
        )
        .unwrap();

        // offline_auth always has access_token = "0"
        let has_token = cmd
            .game_args
            .windows(2)
            .any(|w| w[0] == "--accessToken" && w[1] == "0");
        assert!(
            has_token,
            "offline launches must pass --accessToken 0; got {:?}",
            cmd.game_args
        );
    }

    #[test]
    fn test_compose_java_bin_preserved() {
        let v = load("tests/fixtures/mojang/version_1_21_4.json");
        let auth = offline_auth("TestUser");
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let java = Path::new("/usr/bin/java");
        let cmd = compose(&v, &auth, &fixture_paths(), "myslug", &ctx, java).unwrap();
        assert_eq!(cmd.java_bin, PathBuf::from("/usr/bin/java"));
    }

    #[test]
    fn test_compose_main_class_legacy() {
        let v = load("tests/fixtures/mojang/version_1_12_2.json");
        let auth = offline_auth("TestUser");
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cmd = compose(
            &v,
            &auth,
            &fixture_paths(),
            "myslug",
            &ctx,
            Path::new("java"),
        )
        .unwrap();
        assert_eq!(cmd.main_class, v.main_class);
    }

    // ---- MSA compose tests --------------------------------------------------

    fn msa_auth_fixture() -> MsaAuth {
        MsaAuth {
            username: "PlayerOne".into(),
            uuid: "11111111-1111-4111-8111-111111111111".into(),
            access_token: "mc-tok".into(),
            xuid: "uhs-1".into(),
            xbox_user_hash: "uhs-1".into(),
            clientid: "00000000402b5328".into(),
            user_type: "msa".into(),
        }
    }

    #[test]
    fn test_compose_msa_access_token_in_game_args() {
        let v = load("tests/fixtures/mojang/version_1_21_4.json");
        let auth = msa_auth_fixture();
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cmd = compose_msa(
            &v,
            &auth,
            &fixture_paths(),
            "myslug",
            &ctx,
            Path::new("java"),
        )
        .unwrap();

        assert!(
            cmd.game_args.iter().any(|a| a == "mc-tok"),
            "game_args must contain the real MC access token; got {:?}",
            cmd.game_args
        );
    }

    #[test]
    fn test_compose_msa_user_type_is_msa() {
        let v = load("tests/fixtures/mojang/version_1_21_4.json");
        let auth = msa_auth_fixture();
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cmd = compose_msa(
            &v,
            &auth,
            &fixture_paths(),
            "myslug",
            &ctx,
            Path::new("java"),
        )
        .unwrap();

        assert!(
            cmd.game_args.iter().any(|a| a == "msa"),
            "game_args must contain user_type=\"msa\"; got {:?}",
            cmd.game_args
        );
    }

    #[test]
    fn test_compose_msa_player_name() {
        let v = load("tests/fixtures/mojang/version_1_21_4.json");
        let auth = msa_auth_fixture();
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cmd = compose_msa(
            &v,
            &auth,
            &fixture_paths(),
            "myslug",
            &ctx,
            Path::new("java"),
        )
        .unwrap();

        assert!(
            cmd.game_args.iter().any(|a| a == "PlayerOne"),
            "game_args must contain player name; got {:?}",
            cmd.game_args
        );
    }

    #[test]
    fn test_compose_msa_uuid() {
        let v = load("tests/fixtures/mojang/version_1_21_4.json");
        let auth = msa_auth_fixture();
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cmd = compose_msa(
            &v,
            &auth,
            &fixture_paths(),
            "myslug",
            &ctx,
            Path::new("java"),
        )
        .unwrap();

        assert!(
            cmd.game_args
                .iter()
                .any(|a| a == "11111111-1111-4111-8111-111111111111"),
            "game_args must contain the MSA UUID; got {:?}",
            cmd.game_args
        );
    }

    #[test]
    fn test_compose_msa_xuid_present() {
        let v = load("tests/fixtures/mojang/version_1_21_4.json");
        let auth = msa_auth_fixture();
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cmd = compose_msa(
            &v,
            &auth,
            &fixture_paths(),
            "myslug",
            &ctx,
            Path::new("java"),
        )
        .unwrap();

        assert!(
            cmd.game_args.iter().any(|a| a == "uhs-1")
                || cmd.jvm_args.iter().any(|a| a.contains("uhs-1")),
            "args must contain xuid/xbox_user_hash value; game={:?} jvm={:?}",
            cmd.game_args,
            cmd.jvm_args
        );
    }

    #[test]
    fn test_compose_msa_no_unsubstituted_tokens() {
        let v = load("tests/fixtures/mojang/version_1_21_4.json");
        let auth = msa_auth_fixture();
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let cmd = compose_msa(
            &v,
            &auth,
            &fixture_paths(),
            "myslug",
            &ctx,
            Path::new("java"),
        )
        .unwrap();

        for arg in cmd.jvm_args.iter().chain(cmd.game_args.iter()) {
            assert!(
                !arg.contains("${"),
                "no unsubstituted ${{var}} tokens allowed; found in: {arg}"
            );
        }
    }

    #[test]
    fn test_compose_msa_java_bin_preserved() {
        let v = load("tests/fixtures/mojang/version_1_21_4.json");
        let auth = msa_auth_fixture();
        let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
        let java = Path::new("/usr/bin/java");
        let cmd = compose_msa(&v, &auth, &fixture_paths(), "myslug", &ctx, java).unwrap();
        assert_eq!(cmd.java_bin, PathBuf::from("/usr/bin/java"));
    }
}
