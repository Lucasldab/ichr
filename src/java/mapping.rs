//! Pure platform/version-string mapping helpers shared by the Mojang and Adoptium
//! clients and the resolver service.
//!
//! No I/O, no async, no network. All functions are deterministic pure-Rust.

use std::path::Path;

use crate::domain::platform::{Arch, OsName};
use crate::error::AppError;

// ---------------------------------------------------------------------------
// Platform key mapping -- Mojang JRE all.json
// ---------------------------------------------------------------------------

/// Map `(OsName, Arch)` → Mojang `all.json` top-level platform key, or `None`
/// if Mojang has no entry for the platform (caller falls through to Adoptium).
///
/// Verified against <https://piston-meta.mojang.com/v1/products/java-runtime/…/all.json>
/// (2026-04-20). Mojang has NO `linux-arm64` / `linux-aarch64` key -- Aarch64 Linux
/// must fall back to Adoptium.
pub fn mojang_platform_key(os: OsName, arch: Arch) -> Option<&'static str> {
    match (os, arch) {
        (OsName::Linux, Arch::X86_64) => Some("linux"),
        (OsName::Linux, Arch::Aarch64) => None,
        (OsName::Linux, Arch::Other("x86")) => Some("linux-i386"),
        (OsName::Linux, Arch::Other(_)) => None,
        (OsName::Windows, Arch::X86_64) => Some("windows-x64"),
        (OsName::Windows, Arch::Aarch64) => Some("windows-arm64"),
        (OsName::Windows, Arch::Other("x86")) => Some("windows-x86"),
        (OsName::Windows, Arch::Other(_)) => None,
    }
}

// ---------------------------------------------------------------------------
// Adoptium API parameter strings
// ---------------------------------------------------------------------------

/// Adoptium API `architecture` query-parameter value.
///
/// Used in `GET /v3/assets/latest/{major}/hotspot?architecture={arch}&…`.
pub fn adoptium_arch_str(arch: Arch) -> &'static str {
    match arch {
        Arch::X86_64 => "x64",
        Arch::Aarch64 => "aarch64",
        Arch::Other("x86") => "x86",
        Arch::Other(_) => "x64", // best-effort fallback
    }
}

/// Adoptium API `os` query-parameter value.
///
/// Used in `GET /v3/assets/latest/{major}/hotspot?os={os}&…`.
pub fn adoptium_os_str(os: OsName) -> &'static str {
    match os {
        OsName::Linux => "linux",
        OsName::Windows => "windows",
    }
}

// ---------------------------------------------------------------------------
// java -version output parser
// ---------------------------------------------------------------------------

/// Parse Java major version from the first line of `java -version` stderr output.
///
/// Supported patterns:
/// - `java version "1.8.0_292"`     → `Some(8)`   (legacy 1.x format)
/// - `openjdk version "17.0.2" …`   → `Some(17)`
/// - `java version "11.0.13" …`     → `Some(11)`
/// - `java version "25.0.1" …`      → `Some(25)`
/// - `java version "21"`            → `Some(21)`  (bare major)
/// - garbage / no quotes            → `None`
///
/// Note: `java -version` writes to **stderr**, not stdout.
pub fn parse_java_major(stderr: &str) -> Option<u32> {
    let line = stderr.lines().next()?;
    let start = line.find('"')? + 1;
    let rest = &line[start..];
    let end = rest.find('"')?;
    let ver = &rest[..end];
    let mut parts = ver.split('.');
    let first = parts.next()?;
    if first == "1" {
        // Legacy 1.x format: major is the second component (e.g. "1.8" → 8)
        parts.next()?.parse().ok()
    } else {
        // Modern format: first component is the major (e.g. "17.0.2" → 17)
        first.parse().ok()
    }
}

// ---------------------------------------------------------------------------
// Pre-launch Java version validation
// ---------------------------------------------------------------------------

/// Accept when `found_major >= required_major`; reject with [`AppError::JavaMismatch`]
/// when `found_major < required_major`.
///
/// Note on forward-compatibility (Assumption A4 from research): newer Java is
/// accepted for all vanilla MC versions. Modded 1.12.x on Java 21 may need
/// additional `--add-opens` JVM flags -- that is handled in a future modloader
/// phase, not here.
pub fn validate_java_major(found: u32, required: u32, java_path: &Path) -> Result<(), AppError> {
    if found < required {
        return Err(AppError::JavaMismatch {
            required,
            found,
            path: java_path.to_path_buf(),
            hint: format!(
                "Install Java {required} or set ICHR_JAVA to a Java {required}+ executable"
            ),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // --- mojang_platform_key ---

    #[test]
    fn mojang_linux_x86_64() {
        assert_eq!(
            mojang_platform_key(OsName::Linux, Arch::X86_64),
            Some("linux")
        );
    }

    #[test]
    fn mojang_linux_aarch64_is_none() {
        // Mojang has no linux-arm64 key; caller must fall through to Adoptium
        assert_eq!(mojang_platform_key(OsName::Linux, Arch::Aarch64), None);
    }

    #[test]
    fn mojang_linux_x86_other() {
        assert_eq!(
            mojang_platform_key(OsName::Linux, Arch::Other("x86")),
            Some("linux-i386")
        );
    }

    #[test]
    fn mojang_linux_unknown_other_is_none() {
        assert_eq!(
            mojang_platform_key(OsName::Linux, Arch::Other("riscv64")),
            None
        );
    }

    #[test]
    fn mojang_windows_x86_64() {
        assert_eq!(
            mojang_platform_key(OsName::Windows, Arch::X86_64),
            Some("windows-x64")
        );
    }

    #[test]
    fn mojang_windows_aarch64() {
        assert_eq!(
            mojang_platform_key(OsName::Windows, Arch::Aarch64),
            Some("windows-arm64")
        );
    }

    #[test]
    fn mojang_windows_x86_other() {
        assert_eq!(
            mojang_platform_key(OsName::Windows, Arch::Other("x86")),
            Some("windows-x86")
        );
    }

    #[test]
    fn mojang_windows_unknown_other_is_none() {
        assert_eq!(
            mojang_platform_key(OsName::Windows, Arch::Other("riscv64")),
            None
        );
    }

    // --- adoptium_arch_str ---

    #[test]
    fn adoptium_arch_x86_64() {
        assert_eq!(adoptium_arch_str(Arch::X86_64), "x64");
    }

    #[test]
    fn adoptium_arch_aarch64() {
        assert_eq!(adoptium_arch_str(Arch::Aarch64), "aarch64");
    }

    #[test]
    fn adoptium_arch_x86_other() {
        assert_eq!(adoptium_arch_str(Arch::Other("x86")), "x86");
    }

    // --- adoptium_os_str ---

    #[test]
    fn adoptium_os_linux() {
        assert_eq!(adoptium_os_str(OsName::Linux), "linux");
    }

    #[test]
    fn adoptium_os_windows() {
        assert_eq!(adoptium_os_str(OsName::Windows), "windows");
    }

    // --- parse_java_major ---

    #[test]
    fn parse_legacy_1_8() {
        assert_eq!(
            parse_java_major(r#"java version "1.8.0_292" 2021-07-20"#),
            Some(8)
        );
    }

    #[test]
    fn parse_openjdk_17() {
        assert_eq!(
            parse_java_major(r#"openjdk version "17.0.2" 2022-01-18"#),
            Some(17)
        );
    }

    #[test]
    fn parse_java_11() {
        assert_eq!(
            parse_java_major(r#"java version "11.0.13" 2021-10-19 LTS"#),
            Some(11)
        );
    }

    #[test]
    fn parse_java_21() {
        assert_eq!(
            parse_java_major(r#"java version "21.0.1" 2023-10-17"#),
            Some(21)
        );
    }

    #[test]
    fn parse_java_25() {
        assert_eq!(
            parse_java_major(r#"openjdk version "25.0.1" 2026-04-15"#),
            Some(25)
        );
    }

    #[test]
    fn parse_bare_major_21() {
        // Some JVM builds emit only the major with no dot components
        assert_eq!(parse_java_major(r#"java version "21""#), Some(21));
    }

    #[test]
    fn parse_empty_string_is_none() {
        assert_eq!(parse_java_major(""), None);
    }

    #[test]
    fn parse_no_quotes_is_none() {
        assert_eq!(parse_java_major("garbage with no quotes at all"), None);
    }

    #[test]
    fn parse_multiline_uses_first_line() {
        // java -version emits several lines; only the first is meaningful
        let stderr = "openjdk version \"17.0.2\" 2022-01-18\nOpenJDK Runtime Environment\n";
        assert_eq!(parse_java_major(stderr), Some(17));
    }

    // --- validate_java_major ---

    #[test]
    fn validate_exact_match_is_ok() {
        let path = PathBuf::from("/usr/bin/java");
        assert!(validate_java_major(21, 21, &path).is_ok());
    }

    #[test]
    fn validate_newer_is_ok() {
        let path = PathBuf::from("/usr/bin/java");
        assert!(validate_java_major(25, 21, &path).is_ok());
    }

    #[test]
    fn validate_older_is_err() {
        let path = PathBuf::from("/usr/bin/java");
        let result = validate_java_major(17, 21, &path);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::JavaMismatch {
                required,
                found,
                path: p,
                hint,
            } => {
                assert_eq!(required, 21);
                assert_eq!(found, 17);
                assert_eq!(p, path);
                assert!(hint.contains("21"));
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn validate_java_8_required_8_is_ok() {
        let path = PathBuf::from("/usr/bin/java");
        assert!(validate_java_major(8, 8, &path).is_ok());
    }

    #[test]
    fn validate_java_8_required_17_is_err() {
        let path = PathBuf::from("/usr/bin/java");
        let result = validate_java_major(8, 17, &path);
        assert!(result.is_err());
    }
}
