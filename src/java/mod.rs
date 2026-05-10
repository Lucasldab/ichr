//! Java runtime management -- download (Mojang + Adoptium), system detection,
//! per-instance override, and launch-time resolution.
//! See `.planning/phases/05-java-runtime-management/05-RESEARCH.md`.

pub mod adoptium;
pub mod detect;
pub mod mapping;
pub mod mojang_jre;
pub mod service;
pub mod types;

/// Process-global async mutex serialising any test that mutates the
/// `ICHR_JAVA` environment variable. Tests in `java::service` and
/// `launcher::service` both touch this env var; without a shared lock
/// they race because `std::env::set_var` mutates process state outside
/// any per-runtime mutex. See the failure mode that surfaced after the
/// MSRV 1.90 bump (different test scheduler ordering).
#[cfg(test)]
pub(crate) fn ichr_java_env_lock() -> &'static tokio::sync::Mutex<()> {
    use std::sync::OnceLock;
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}
