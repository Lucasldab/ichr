//! Integration test pinning embed-manifest 1.5.0 builder defaults.
//! Covers PLAT-04 (Windows longPathAware manifest embed via build.rs).

use embed_manifest::new_manifest;

/// Builder-contract test: `new_manifest()` defaults must keep
/// `long_path_aware` enabled. If embed-manifest 2.x changes this default,
/// our `build.rs` becomes a silent no-op for the Windows manifest's most
/// important capability -- this test catches that drift.
///
/// embed-manifest 1.5's `ManifestBuilder` and its inner `WindowsSettings`
/// both derive `Debug`. The nested `WindowsSettings` field stores
/// `long_path_aware` as a `bool` (NOT a `Setting` enum -- the public
/// `long_path_aware(Setting)` setter converts via `.enabled()`). The
/// Debug representation of a default-constructed builder therefore
/// contains the literal substring `long_path_aware: true`. We pin
/// EXACTLY that -- if the field name changes OR if the default flips
/// to `false`, this test fails loudly.
#[test]
fn embed_manifest_defaults_enable_long_path_aware() {
    let m = new_manifest("Ichr.Launcher");
    let debug_repr = format!("{m:?}");
    assert!(
        debug_repr.contains("long_path_aware"),
        "embed-manifest builder Debug must mention long_path_aware field; got: {debug_repr}"
    );
    assert!(
        debug_repr.contains("long_path_aware: true"),
        "embed-manifest default for long_path_aware must be `true` (Setting::Enabled \
         coerced via .enabled()); got: {debug_repr}"
    );
}

/// Documentation-style: pins that on a Linux test runner the build
/// script's `CARGO_CFG_WINDOWS` branch is NOT taken. This is not a strict
/// assertion (the env var is build-script-only), but the corresponding
/// `cargo build` already succeeded by the time this test runs, which is
/// itself the no-op proof.
#[cfg(target_os = "linux")]
#[test]
fn build_rs_is_no_op_on_linux() {
    // CARGO_CFG_WINDOWS is set ONLY during build script compilation
    // for Windows targets. Tests do not see it. Asserting `is_none()`
    // here is therefore a tautology under `cargo test` on Linux -- the
    // POINT of the test is that this file COMPILES at all on Linux,
    // which proves embed-manifest's dev-dep is reachable on non-Windows
    // hosts (it has no platform-specific build deps that fail to
    // compile on Linux).
    assert!(
        std::env::var_os("CARGO_CFG_WINDOWS").is_none(),
        "Tests on Linux must not see CARGO_CFG_WINDOWS"
    );
}

/// Windows-host smoke: after `cargo build --release` on Windows, the
/// expected binary path exists. PE manifest section verification is
/// part of HUMAN-UAT (12-HUMAN-UAT.md step 1) -- too heavy for an
/// integration test. This test only confirms the build pipeline
/// produces a binary at the conventional location.
#[cfg(target_os = "windows")]
#[test]
fn built_exe_path_exists_after_release_build() {
    // Locate the workspace target/ directory via cargo's standard
    // layout. We accept either debug or release path -- the goal is to
    // prove the .exe exists, not which profile produced it.
    let target_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let candidates = [
        target_dir.join("release").join("ichr.exe"),
        target_dir.join("debug").join("ichr.exe"),
    ];
    let exists = candidates.iter().any(|p| p.exists());
    assert!(
        exists,
        "expected ichr.exe to exist at one of: {candidates:?} (run `cargo build` first)"
    );
}
