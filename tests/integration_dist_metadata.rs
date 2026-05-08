//! Integration test pinning Cargo.toml [package] metadata for crates.io publish.
//! Covers DIST-01 (cargo install / cargo publish metadata).

use std::process::Command;

fn ichr_package() -> serde_json::Value {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .expect("cargo metadata runs");
    assert!(
        out.status.success(),
        "cargo metadata failed: status={:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("cargo metadata returns valid json");
    let pkgs = v["packages"].as_array().expect("packages array");
    pkgs.iter()
        .find(|p| p["name"] == "ichr")
        .cloned()
        .expect("ichr package present in metadata")
}

#[test]
fn cargo_metadata_declares_required_publish_fields() {
    let pkg = ichr_package();
    for field in ["repository", "homepage", "readme", "description", "license"] {
        assert!(
            !pkg[field].is_null(),
            "Cargo.toml [package].{field} is null — required for crates.io publish"
        );
    }
    assert!(
        pkg["authors"].as_array().is_some_and(|a| !a.is_empty()),
        "Cargo.toml [package].authors must have at least one entry"
    );
    assert!(
        pkg["keywords"].as_array().is_some_and(|k| !k.is_empty()),
        "Cargo.toml [package].keywords must have at least one entry"
    );
    assert!(
        pkg["categories"].as_array().is_some_and(|c| !c.is_empty()),
        "Cargo.toml [package].categories must have at least one entry"
    );
    let rv = pkg["rust_version"].as_str().expect("rust_version set");
    assert_eq!(rv, "1.88", "MSRV must be 1.88 (zip 8.5.1 floor)");
}

#[test]
fn cargo_metadata_repository_points_at_lucasldab_owner() {
    let pkg = ichr_package();
    let repo = pkg["repository"].as_str().expect("repository set");
    assert_eq!(
        repo, "https://github.com/Lucasldab/ichr",
        "repository URL must match the GitHub remote (Lucasldab/ichr); cargo install --git users follow this URL"
    );
}

#[test]
fn cargo_metadata_license_is_dual_licensed() {
    let pkg = ichr_package();
    let license = pkg["license"].as_str().expect("license set");
    assert_eq!(
        license, "MIT OR Apache-2.0",
        "SPDX license expression must stay 'MIT OR Apache-2.0' to match LICENSE-MIT + LICENSE-APACHE files at repo root"
    );
}
