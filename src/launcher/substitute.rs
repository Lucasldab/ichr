//! `${var}` substitution in Minecraft version JSON argument templates.
//!
//! Provides `SubstitutionContext` (a flat struct of every variable) and
//! `substitute(template, ctx)` which performs a single linear sweep of
//! `String::replace` calls -- no shell escaping, no regex, no nesting.
//!
//! All args are ultimately passed to `tokio::process::Command::args` as
//! separate `OsString` elements, so shell-escaping would be incorrect here.

use std::path::PathBuf;

/// All substitution variables that can appear in Minecraft version JSON
/// argument templates (JVM args and game args blocks).
///
/// Offline defaults:
/// - `auth_xuid`, `clientid`, `auth_xbox_user_hash` → `""`
/// - `auth_access_token` → `"0"`
/// - `user_type` → `"legacy"`
/// - `resolution_width`, `resolution_height` → `""` (feature not active in v1)
#[derive(Debug, Clone)]
pub struct SubstitutionContext {
    // --- Auth ---------------------------------------------------------------
    /// Offline: arbitrary username chosen by the user.
    pub auth_player_name: String,
    /// Offline: deterministic UUID from `offline_uuid(username)`.
    pub auth_uuid: String,
    /// Offline: `"0"` (the safe universal offline placeholder).
    pub auth_access_token: String,
    /// Offline: `""` (no Xbox UID).
    pub auth_xuid: String,
    /// Offline: `""` (no client ID).
    pub clientid: String,
    /// Offline: `""` (no Xbox user hash).
    pub auth_xbox_user_hash: String,
    /// Offline: `"legacy"`. Online MSA auth uses `"msa"`.
    pub user_type: String,

    // --- Version ------------------------------------------------------------
    /// `version.id`, e.g. `"1.21.4"`.
    pub version_name: String,
    /// `version.version_type`, e.g. `"release"` or `"snapshot"`.
    pub version_type: String,

    // --- Paths --------------------------------------------------------------
    /// `AppPaths::instance_minecraft_dir(slug)` -- the `.minecraft` working dir.
    pub game_directory: PathBuf,
    /// `AppPaths::assets_dir()` -- shared Mojang asset objects root.
    pub assets_root: PathBuf,
    /// `version.asset_index.id` -- the asset index name (e.g. `"17"`).
    pub assets_index_name: String,
    /// `AppPaths::instance_natives_dir(slug)` -- per-instance extracted natives.
    pub natives_directory: PathBuf,
    /// `AppPaths::libraries_dir()` -- shared Maven-layout libraries root.
    pub library_directory: PathBuf,

    // --- Classpath (filled during JVM arg composition) ----------------------
    /// The pre-built OS-separator-joined classpath string.
    pub classpath: String,

    // --- Platform -----------------------------------------------------------
    /// `':'` on Linux, `';'` on Windows -- the classpath entry separator.
    pub classpath_separator: char,
    /// Mojang arch string: `"x86_64"` or `"arm64"`.
    pub arch: String,

    // --- Launcher identity --------------------------------------------------
    /// Always `"ichr"`.
    pub launcher_name: String,
    /// `env!("CARGO_PKG_VERSION")` baked in at compile time.
    pub launcher_version: String,

    // --- Optional / unimplemented in v1 -------------------------------------
    /// `""` -- `has_custom_resolution` feature not active in v1.
    pub resolution_width: String,
    /// `""` -- `has_custom_resolution` feature not active in v1.
    pub resolution_height: String,
}

/// Replace every `${key}` occurrence in `template` with the matching field
/// from `ctx`. Unknown placeholders are left as-is (no crash, no empty
/// replacement). Paths are converted via `.to_string_lossy()`.
///
/// This is a single linear sweep -- Mojang templates never nest `${...}`
/// so order of replacement is irrelevant.
pub fn substitute(template: &str, ctx: &SubstitutionContext) -> String {
    // Capture lossy path strings before the borrow checker has to juggle them.
    let game_dir = ctx.game_directory.to_string_lossy().into_owned();
    let assets_root = ctx.assets_root.to_string_lossy().into_owned();
    let natives_dir = ctx.natives_directory.to_string_lossy().into_owned();
    let lib_dir = ctx.library_directory.to_string_lossy().into_owned();
    let sep_str = ctx.classpath_separator.to_string();

    let vars: &[(&str, &str)] = &[
        ("${auth_player_name}", &ctx.auth_player_name),
        ("${auth_uuid}", &ctx.auth_uuid),
        ("${auth_access_token}", &ctx.auth_access_token),
        ("${auth_xuid}", &ctx.auth_xuid),
        ("${clientid}", &ctx.clientid),
        ("${auth_xbox_user_hash}", &ctx.auth_xbox_user_hash),
        ("${user_type}", &ctx.user_type),
        ("${version_name}", &ctx.version_name),
        ("${version_type}", &ctx.version_type),
        ("${game_directory}", &game_dir),
        ("${assets_root}", &assets_root),
        ("${assets_index_name}", &ctx.assets_index_name),
        ("${natives_directory}", &natives_dir),
        ("${library_directory}", &lib_dir),
        ("${classpath}", &ctx.classpath),
        ("${classpath_separator}", &sep_str),
        ("${arch}", &ctx.arch),
        ("${launcher_name}", &ctx.launcher_name),
        ("${launcher_version}", &ctx.launcher_version),
        ("${resolution_width}", &ctx.resolution_width),
        ("${resolution_height}", &ctx.resolution_height),
    ];

    let mut out = template.to_string();
    for (key, val) in vars {
        out = out.replace(key, val);
    }
    out
}

/// Map `substitute` over a slice of template strings.
pub fn substitute_all(templates: &[String], ctx: &SubstitutionContext) -> Vec<String> {
    templates.iter().map(|t| substitute(t, ctx)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn minimal_ctx() -> SubstitutionContext {
        SubstitutionContext {
            auth_player_name: "TestUser".to_string(),
            auth_uuid: "xxxxxxxx-xxxx-3xxx-yxxx-xxxxxxxxxxxx".to_string(),
            auth_access_token: "0".to_string(),
            auth_xuid: String::new(),
            clientid: String::new(),
            auth_xbox_user_hash: String::new(),
            user_type: "legacy".to_string(),
            version_name: "1.21.4".to_string(),
            version_type: "release".to_string(),
            game_directory: PathBuf::from("/data/instances/test/.minecraft"),
            assets_root: PathBuf::from("/data/assets"),
            assets_index_name: "17".to_string(),
            natives_directory: PathBuf::from("/data/instances/test/natives"),
            library_directory: PathBuf::from("/data/libraries"),
            classpath: "/data/libraries/a.jar:/data/versions/1.21.4/1.21.4.jar".to_string(),
            classpath_separator: ':',
            arch: "x86_64".to_string(),
            launcher_name: "ichr".to_string(),
            launcher_version: "0.1.0".to_string(),
            resolution_width: String::new(),
            resolution_height: String::new(),
        }
    }

    #[test]
    fn test_substitute_auth_player_name() {
        let ctx = minimal_ctx();
        let result = substitute("--username ${auth_player_name}", &ctx);
        assert_eq!(result, "--username TestUser");
    }

    #[test]
    fn test_substitute_classpath() {
        let ctx = minimal_ctx();
        let result = substitute("-cp ${classpath}", &ctx);
        assert!(result.starts_with("-cp /data/libraries/a.jar"));
    }

    #[test]
    fn test_substitute_missing_var_passthrough() {
        let ctx = minimal_ctx();
        let result = substitute("${unknown_var}", &ctx);
        // Unknown variables must be left as-is, not panicked or emptied.
        assert_eq!(result, "${unknown_var}");
    }

    #[test]
    fn test_substitute_all_empties_paths() {
        // PathBuf::new() should substitute as "" not "PathBuf" or panic.
        let mut ctx = minimal_ctx();
        ctx.game_directory = PathBuf::new();
        ctx.assets_root = PathBuf::new();
        ctx.natives_directory = PathBuf::new();
        ctx.library_directory = PathBuf::new();
        let result = substitute(
            "${game_directory}|${assets_root}|${natives_directory}|${library_directory}",
            &ctx,
        );
        assert_eq!(result, "|||");
    }

    #[test]
    fn test_substitute_version_name() {
        let ctx = minimal_ctx();
        let result = substitute("--version ${version_name}", &ctx);
        assert_eq!(result, "--version 1.21.4");
    }

    #[test]
    fn test_substitute_user_type() {
        let ctx = minimal_ctx();
        let result = substitute("--userType ${user_type}", &ctx);
        assert_eq!(result, "--userType legacy");
    }

    #[test]
    fn test_substitute_all() {
        let ctx = minimal_ctx();
        let templates = vec![
            "--username ${auth_player_name}".to_string(),
            "--uuid ${auth_uuid}".to_string(),
        ];
        let results = substitute_all(&templates, &ctx);
        assert_eq!(results[0], "--username TestUser");
        assert_eq!(results[1], "--uuid xxxxxxxx-xxxx-3xxx-yxxx-xxxxxxxxxxxx");
    }

    #[test]
    fn test_substitute_classpath_separator() {
        let ctx = minimal_ctx();
        let result = substitute("${classpath_separator}", &ctx);
        assert_eq!(result, ":");
    }

    #[test]
    fn test_substitute_launcher_identity() {
        let ctx = minimal_ctx();
        let result = substitute("${launcher_name}/${launcher_version}", &ctx);
        assert_eq!(result, "ichr/0.1.0");
    }
}
