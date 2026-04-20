//! Tests for `mineltui::domain::platform`.
//! Covers PLAT-03 (arch / OS detection for Mojang library rules).

use mineltui::domain::platform::{Arch, OsName};
use mineltui::AppError;

#[test]
fn arch_current_is_known_variant() {
    let a = Arch::current();
    match a {
        Arch::X86_64 | Arch::Aarch64 | Arch::Other(_) => {}
    }
}

#[test]
fn arch_current_matches_std_env_consts() {
    let arch = Arch::current();
    let mojang = arch.mojang_str();
    match std::env::consts::ARCH {
        "x86_64" => assert_eq!(mojang, "x86_64"),
        "aarch64" => assert_eq!(mojang, "arm64"),
        other => assert_eq!(mojang, other),
    }
}

#[test]
fn arch_mojang_str_format() {
    assert_eq!(Arch::X86_64.mojang_str(), "x86_64");
    assert_eq!(Arch::Aarch64.mojang_str(), "arm64");
    assert_eq!(Arch::Other("ppc64le").mojang_str(), "ppc64le");
}

#[test]
fn arch_is_copy_and_eq() {
    let a = Arch::X86_64;
    let b = a;           // compiles because Arch: Copy
    assert_eq!(a, b);
}

#[cfg(target_os = "linux")]
#[test]
fn os_name_linux() {
    assert_eq!(OsName::current(), OsName::Linux);
    assert_eq!(OsName::Linux.mojang_str(), "linux");
}

#[cfg(target_os = "windows")]
#[test]
fn os_name_windows() {
    assert_eq!(OsName::current(), OsName::Windows);
    assert_eq!(OsName::Windows.mojang_str(), "windows");
}

#[test]
fn app_error_display() {
    assert_eq!(AppError::Cancelled.to_string(), "Operation cancelled");
    assert!(AppError::PathResolution.to_string().contains("Path resolution failed"));
}

#[test]
fn app_error_from_io() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
    let app_err: AppError = io_err.into();
    assert!(app_err.to_string().contains("I/O error"));
}
