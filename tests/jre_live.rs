//! Live JRE download smoke test — gated by `#[ignore]`.
//! Run with: `cargo test --test jre_live -- --ignored --nocapture`
//!
//! Downloads a real Mojang-blessed JRE (`java-runtime-delta` for Java 21) into a
//! temporary directory and asserts the final `bin/java[.exe]` executable exists.
//! Requires internet access and ~150 MB free on the tempdir filesystem.
//!
//! Phase 5 sign-off on `05-VALIDATION.md` requires a successful run of this test
//! (see VALIDATION.md Manual-Only Verifications and nyquist gate).

use mineltui::domain::platform::{Arch, OsName};
use mineltui::java::mapping::mojang_platform_key;
use mineltui::java::service::JavaService;
use mineltui::persistence::paths::AppPaths;
use tempfile::TempDir;

#[tokio::test]
#[ignore = "requires internet access, ~150 MB download, and a Mojang-supported platform — see module docs"]
async fn live_mojang_jre_download_java_runtime_delta() {
    let td = TempDir::new().unwrap();
    let paths = AppPaths::with_roots(
        td.path().to_path_buf(),
        td.path().to_path_buf(),
        td.path().to_path_buf(),
    );

    let (arch, os) = (Arch::current(), OsName::current());
    let plat = mojang_platform_key(os, arch);
    println!("[jre_live] platform key: {plat:?}");

    if plat.is_none() {
        println!(
            "[jre_live] skipping — no Mojang platform key for this host \
             (arch={arch:?}, os={os:?}); use Adoptium fallback test on this platform"
        );
        return;
    }

    let svc = JavaService::new().expect("JavaService::new should succeed");

    println!("[jre_live] starting download of java-runtime-delta via Mojang endpoints…");
    let exe = svc
        .install_mojang(&paths, "java-runtime-delta")
        .await
        .expect("install_mojang should succeed with internet access");

    println!("[jre_live] install_mojang returned: {}", exe.display());

    assert!(
        exe.is_file(),
        "expected executable file at {}",
        exe.display()
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&exe).expect("metadata of java executable must be readable");
        assert!(
            meta.permissions().mode() & 0o111 != 0,
            "java executable must have execute permission; path={}",
            exe.display()
        );
    }

    println!("[jre_live] SUCCESS — downloaded JRE to {}", exe.display());
}
