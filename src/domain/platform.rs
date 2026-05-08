//! Platform detection: architecture and OS identity.
//!
//! These types are consumed by the Mojang library rule evaluator
//! (added in Phase 2) to decide which native classifiers to download.
//! See `.planning/research/PITFALLS.md` Pitfall 3 for the rule semantics.

/// CPU architecture of the running process. Values are compile-time
/// (from `std::env::consts::ARCH`), which is the correct source for
/// Mojang library rule evaluation even when cross-compiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
    Other(&'static str),
}

impl Arch {
    /// Architecture of the running process, resolved at compile time.
    pub const fn current() -> Self {
        let a = std::env::consts::ARCH;
        if str_eq(a, "x86_64") {
            Arch::X86_64
        } else if str_eq(a, "aarch64") {
            Arch::Aarch64
        } else {
            Arch::Other(a)
        }
    }

    /// Mojang-style arch string used in library rule matching
    /// (`rules[].arch` and `${arch}` substitutions).
    ///
    /// - `X86_64` → `"x86_64"`
    /// - `Aarch64` → `"arm64"`  (Mojang uses `arm64`, not `aarch64`)
    /// - `Other(s)` → `s`
    pub fn mojang_str(&self) -> &str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "arm64",
            Arch::Other(s) => s,
        }
    }
}

/// const-context byte-wise string equality -- Rust 1.88 lacks const `str::eq`.
const fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Operating system identity. Only Linux and Windows are v1 targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsName {
    Linux,
    Windows,
}

impl OsName {
    /// Resolve the current OS at compile time.
    ///
    /// Panics on an unsupported OS (macOS, BSD, etc.) -- out of scope for v1.
    pub const fn current() -> Self {
        if cfg!(target_os = "linux") {
            OsName::Linux
        } else if cfg!(target_os = "windows") {
            OsName::Windows
        } else {
            panic!("Unsupported OS -- only Linux and Windows are v1 targets");
        }
    }

    /// Mojang-style OS name string used in library rule matching
    /// (`rules[].os.name`).
    pub fn mojang_str(&self) -> &'static str {
        match self {
            OsName::Linux => "linux",
            OsName::Windows => "windows",
        }
    }
}
