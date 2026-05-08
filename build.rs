//! Build script: embed Windows side-by-side application manifest.
//!
//! Declares `longPathAware=true` so the launcher binary AND its child
//! JVM processes can operate on paths longer than 260 chars on Windows
//! 10/11. No-op on non-Windows targets via the `CARGO_CFG_WINDOWS`
//! guard. Required for PLAT-04 (Phase 12).
//!
//! `new_manifest()` defaults already enable:
//!   long_path_aware     = Enabled
//!   active_code_page    = Utf8
//!   dpi_awareness       = PerMonitorV2
//!   supported_os        = Windows7..=Windows11
//!   requested_execution_level = AsInvoker
//!
//! We use the defaults verbatim -- no custom builder calls. The manifest
//! is purely necessary for child-process inheritance: Rust std's `\\?\`
//! auto-prefixing already handles the launcher's own file ops since
//! Rust 1.58, but the JVM spawned via tokio::process::Command does NOT
//! have its own long-path handling and inherits the parent process's
//! manifest setting.
//!
//! Source: https://docs.rs/embed-manifest/1.5.0/

use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest(new_manifest("Ichr.Launcher"))
            .expect("unable to embed Windows manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
