//! Integration tests for `mineltui::persistence::paths`.
//!
//! Covers PLAT-01 (XDG paths on Linux) and PLAT-02 (AppData on Windows).

use std::path::PathBuf;

use mineltui::persistence::AppPaths;

fn assert_path_contains_segment(p: &std::path::Path, segment: &str) {
    let found = p
        .components()
        .any(|c| c.as_os_str().to_string_lossy() == segment);
    assert!(
        found,
        "expected path {p:?} to contain segment {segment:?}"
    );
}

#[test]
fn resolve_returns_some() {
    let paths = AppPaths::resolve().expect("home directory should resolve on CI");
    assert!(!paths.data_dir.as_os_str().is_empty());
    assert!(!paths.config_dir.as_os_str().is_empty());
    assert!(!paths.cache_dir.as_os_str().is_empty());
}

#[test]
fn data_config_cache_all_contain_app_name() {
    let paths = AppPaths::resolve().expect("paths resolve");
    assert_path_contains_segment(&paths.data_dir, "mineltui");
    assert_path_contains_segment(&paths.config_dir, "mineltui");
    assert_path_contains_segment(&paths.cache_dir, "mineltui");
}

#[test]
fn derived_paths_descend_from_roots() {
    let tmp_data = PathBuf::from("/tmp/mineltui-test-data");
    let tmp_config = PathBuf::from("/tmp/mineltui-test-config");
    let tmp_cache = PathBuf::from("/tmp/mineltui-test-cache");
    let paths = AppPaths::with_roots(tmp_data.clone(), tmp_config.clone(), tmp_cache.clone());

    assert_eq!(paths.log_file(), tmp_data.join("mineltui.log"));
    assert_eq!(paths.app_config_file(), tmp_config.join("config.toml"));
    assert_eq!(paths.instances_dir(), tmp_data.join("instances"));
    assert_eq!(paths.assets_dir(), tmp_data.join("assets"));
    assert_eq!(paths.libraries_dir(), tmp_data.join("libraries"));
    assert_eq!(paths.versions_dir(), tmp_data.join("versions"));
    assert_eq!(paths.runtime_dir(), tmp_data.join("runtime"));
}

#[test]
fn log_file_lives_under_data_dir() {
    let paths = AppPaths::resolve().expect("paths resolve");
    let log = paths.log_file();
    assert!(
        log.starts_with(&paths.data_dir),
        "log file {:?} should descend from data_dir {:?}",
        log, paths.data_dir
    );
    assert_eq!(log.file_name().and_then(|f| f.to_str()), Some("mineltui.log"));
}

#[test]
fn app_config_file_lives_under_config_dir() {
    let paths = AppPaths::resolve().expect("paths resolve");
    let cfg = paths.app_config_file();
    assert!(
        cfg.starts_with(&paths.config_dir),
        "config file {:?} should descend from config_dir {:?}",
        cfg, paths.config_dir
    );
    assert_eq!(cfg.file_name().and_then(|f| f.to_str()), Some("config.toml"));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_xdg_paths() {
    // PLAT-01: XDG-compliant paths on Linux.
    let paths = AppPaths::resolve().expect("paths resolve");
    let data_str = paths.data_dir.to_string_lossy();
    assert!(
        data_str.contains(".local/share") || std::env::var("XDG_DATA_HOME").is_ok_and(|v| data_str.starts_with(&v)),
        "Linux data_dir {:?} should be under ~/.local/share or $XDG_DATA_HOME",
        paths.data_dir
    );
    let config_str = paths.config_dir.to_string_lossy();
    assert!(
        config_str.contains(".config") || std::env::var("XDG_CONFIG_HOME").is_ok_and(|v| config_str.starts_with(&v)),
        "Linux config_dir {:?} should be under ~/.config or $XDG_CONFIG_HOME",
        paths.config_dir
    );
    assert_path_contains_segment(&paths.data_dir, "mineltui");
}

#[cfg(target_os = "windows")]
#[test]
fn windows_appdata_paths() {
    // PLAT-02: %APPDATA%-based paths on Windows.
    let paths = AppPaths::resolve().expect("paths resolve");
    let data_str = paths.data_dir.to_string_lossy().to_lowercase();
    assert!(
        data_str.contains("appdata"),
        "Windows data_dir {:?} should descend from %APPDATA%",
        paths.data_dir
    );
    assert_path_contains_segment(&paths.data_dir, "mineltui");
}
