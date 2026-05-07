//! Structural pin against the bundled ForgeWrapper JAR.
//!
//! Asserts that BOTH `Main.class` (launch-time) AND `Installer.class`
//! (install-time) are present in the embedded jar bytes. Catches accidental
//! re-vendoring with a stripped or replaced jar that would silently break
//! Phase 7 install (regression of GAP-7-A) or Phase 12 launch wiring.
//!
//! Runs offline; no network, no JVM, no fs. Loads the embedded jar bytes via
//! `mineltui::loader::forgewrapper::FORGE_WRAPPER_JAR` and parses the central
//! directory using the `zip` crate.

use std::io::Cursor;
use zip::ZipArchive;

use mineltui::loader::forgewrapper::{
    FORGE_WRAPPER_INSTALLER_CLASS, FORGE_WRAPPER_JAR, FORGE_WRAPPER_MAIN_CLASS,
};

#[test]
fn forgewrapper_jar_contains_both_main_and_installer_classes() {
    // Sanity: jar bytes are non-empty + start with PK (zip magic).
    assert!(
        FORGE_WRAPPER_JAR.len() > 20_000,
        "embedded ForgeWrapper jar suspiciously small: {} bytes",
        FORGE_WRAPPER_JAR.len()
    );
    assert_eq!(&FORGE_WRAPPER_JAR[..2], b"PK", "jar magic missing");

    let cursor = Cursor::new(FORGE_WRAPPER_JAR);
    let mut zip = ZipArchive::new(cursor).expect("ZipArchive::new on embedded jar");

    let entries: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();

    let expect_install = "io/github/zekerzhayard/forgewrapper/installer/Installer.class";
    let expect_main = "io/github/zekerzhayard/forgewrapper/installer/Main.class";

    assert!(
        entries.iter().any(|e| e == expect_install),
        "Installer.class missing from bundled jar — GAP-7-A regression. \
         FORGE_WRAPPER_INSTALLER_CLASS={} expects path {}. Entries: {:?}",
        FORGE_WRAPPER_INSTALLER_CLASS,
        expect_install,
        entries
            .iter()
            .filter(|e| e.ends_with(".class"))
            .collect::<Vec<_>>()
    );
    assert!(
        entries.iter().any(|e| e == expect_main),
        "Main.class missing from bundled jar — Phase 12 launch-time class \
         missing. FORGE_WRAPPER_MAIN_CLASS={} expects path {}. Entries: {:?}",
        FORGE_WRAPPER_MAIN_CLASS,
        expect_main,
        entries
            .iter()
            .filter(|e| e.ends_with(".class"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn forgewrapper_class_constants_are_distinct() {
    // GAP-7-A root cause was conflating these two; pin the distinction.
    assert_ne!(
        FORGE_WRAPPER_MAIN_CLASS, FORGE_WRAPPER_INSTALLER_CLASS,
        "MAIN and INSTALLER class constants must be distinct fully-qualified \
         names; if they're equal, the gap reopens."
    );
    assert!(FORGE_WRAPPER_MAIN_CLASS.ends_with(".Main"));
    assert!(FORGE_WRAPPER_INSTALLER_CLASS.ends_with(".Installer"));
}
