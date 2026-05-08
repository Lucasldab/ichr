//! Structural pin against the bundled ForgeWrapper JAR.
//!
//! The pin asserts that `Main.class` (the install-time AND launch-time
//! JVM entry point — confirmed by debug session 2026-05-07T20:45 against
//! the upstream `1.6.0` JAR via `javap -public`) contains a
//! `public static void main(String[] args)` method.
//!
//! GAP-7-A history (round 1 → round 2 → round 3, single umbrella):
//!   - Round 1 (07-04): wired FORGE_WRAPPER_MAIN_CLASS without the three
//!     -Dforgewrapper.* properties → NoClassDefFoundError: modlauncher.
//!     Symptom misread as "wrong class".
//!   - Round 2 (07.1-02): swapped install-time class to `Installer.class`
//!     → "Main method not found in .Installer"
//!     because Installer is a library class with no `main()`.
//!   - Round 3 (07.2-01, this commit): revert to Main + add the three
//!     -D properties + this strengthened test. Class presence ≠
//!     entry-point presence; the round-2 misdiagnosis would have been
//!     caught by this bytecode pin.
//!
//! Runs offline; no network, no JVM, no fs. Loads the embedded jar bytes
//! via `mineltui::loader::forgewrapper::FORGE_WRAPPER_JAR` and uses the
//! `zip` crate to extract `Main.class` bytes for byte-substring scanning.

use std::io::{Cursor, Read};
use zip::ZipArchive;

use mineltui::loader::forgewrapper::{FORGE_WRAPPER_JAR, FORGE_WRAPPER_MAIN_CLASS};

/// JVM method descriptor for `void main(String[])`.
/// Constant-pool entries this exact byte sequence in any .class file
/// containing a `main(String[])` method.
const MAIN_DESCRIPTOR_BYTES: &[u8] = b"([Ljava/lang/String;)V";

/// JVM method name as it appears in the constant pool.
const MAIN_NAME_BYTES: &[u8] = b"main";

/// Read a class entry's bytes out of the embedded ForgeWrapper JAR.
fn read_class_bytes(name: &str) -> Vec<u8> {
    let cursor = Cursor::new(FORGE_WRAPPER_JAR);
    let mut zip = ZipArchive::new(cursor).expect("ZipArchive::new on embedded ForgeWrapper jar");
    let mut entry = zip
        .by_name(name)
        .unwrap_or_else(|e| panic!("class entry {name} missing from JAR: {e}"));
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut bytes)
        .expect("read class bytes from JAR entry");
    bytes
}

/// True if `haystack` contains the byte sequence `needle`.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn forgewrapper_main_class_has_main_string_method_signature() {
    // Sanity: jar bytes are non-empty + start with PK (zip magic).
    assert!(
        FORGE_WRAPPER_JAR.len() > 20_000,
        "embedded ForgeWrapper jar suspiciously small: {} bytes",
        FORGE_WRAPPER_JAR.len()
    );
    assert_eq!(&FORGE_WRAPPER_JAR[..2], b"PK", "jar magic missing");

    // Map our Rust-level fully-qualified class name to the JAR-internal
    // ZIP entry path: replace `.` with `/` and append `.class`.
    let zip_path = format!("{}.class", FORGE_WRAPPER_MAIN_CLASS.replace('.', "/"));
    assert_eq!(
        zip_path, "io/github/zekerzhayard/forgewrapper/installer/Main.class",
        "FORGE_WRAPPER_MAIN_CLASS does not map to the expected ZIP entry path"
    );

    let main_bytes = read_class_bytes(&zip_path);

    // Class file magic: 0xCAFEBABE.
    assert_eq!(
        &main_bytes[..4],
        &[0xCA, 0xFE, 0xBA, 0xBE],
        "Main.class missing JVM class-file magic 0xCAFEBABE"
    );

    // Constant pool MUST contain UTF-8 entries `"main"` AND
    // `"([Ljava/lang/String;)V"` for the entry-point method to exist.
    // GAP-7-A-v2 pin: Installer.class contains neither (its only public
    // methods are install(File,File,File), getData(File), getWrapper()).
    assert!(
        contains_bytes(&main_bytes, MAIN_NAME_BYTES),
        "Main.class constant pool missing UTF-8 entry `main` — not a valid \
         JVM entry point. GAP-7-A umbrella regression: re-vendored JAR may \
         be stripped or renamed."
    );
    assert!(
        contains_bytes(&main_bytes, MAIN_DESCRIPTOR_BYTES),
        "Main.class constant pool missing method descriptor \
         `([Ljava/lang/String;)V` — class has no `void main(String[])` \
         method, hence not a valid JVM entry point. GAP-7-A-v2 \
         regression: would re-trigger `Main method not found` at \
         install-time JVM launch."
    );
}

#[test]
fn forgewrapper_installer_class_lacks_main_string_method_signature() {
    // Inverse pin: confirm the bytecode-level discrimination actually
    // works. The Installer class — present in the JAR but not an entry
    // point — must NOT contain the main(String[]) descriptor. If a future
    // re-vendoring adds a main() to Installer.class, this test will fail
    // (loud signal) and the developer can decide whether to widen the
    // entry-point class set.
    let installer_bytes =
        read_class_bytes("io/github/zekerzhayard/forgewrapper/installer/Installer.class");
    assert_eq!(
        &installer_bytes[..4],
        &[0xCA, 0xFE, 0xBA, 0xBE],
        "Installer.class missing JVM class-file magic"
    );
    assert!(
        !contains_bytes(&installer_bytes, MAIN_DESCRIPTOR_BYTES),
        "Installer.class unexpectedly contains main(String[]) descriptor — \
         upstream may have added a main() entry point. Re-evaluate whether \
         FORGE_WRAPPER_MAIN_CLASS is still the unique entry point."
    );
}

#[test]
fn forgewrapper_main_class_constant_ends_with_dot_main() {
    // Single-constant invariant (replaces the round-2
    // `forgewrapper_class_constants_are_distinct` test which compared
    // two constants — the second constant is deleted in this plan).
    assert!(
        FORGE_WRAPPER_MAIN_CLASS.ends_with(".Main"),
        "FORGE_WRAPPER_MAIN_CLASS must end with `.Main`: {FORGE_WRAPPER_MAIN_CLASS}"
    );
    assert!(
        FORGE_WRAPPER_MAIN_CLASS.starts_with("io.github.zekerzhayard.forgewrapper"),
        "FORGE_WRAPPER_MAIN_CLASS must be in the upstream package: \
         {FORGE_WRAPPER_MAIN_CLASS}"
    );
}

/// Pins Main.class as the LAUNCH-time entry point — its bytecode constant pool
/// MUST contain the literal UTF-8 sequence `--fml.mcVersion`. This is the FML
/// argv flag Main parses at line 28 of the upstream source
/// (https://raw.githubusercontent.com/ZekerZhayard/ForgeWrapper/3c6712d64a42e4ec200909912e72749499aaca79/src/main/java/io/github/zekerzhayard/forgewrapper/installer/Main.java):
///     String mcVersion = argsList.get(argsList.indexOf("--fml.mcVersion") + 1);
///
/// GAP-7-A-v3 (round 3) regression history: invoking Main at install-time with
/// empty argv produces `IndexOutOfBoundsException: Index 0 out of bounds for
/// length 0` because `indexOf` returns -1, +1 = 0, `get(0)` on length-0 list
/// throws. The structurally correct fix (07.3-01) is to NOT invoke Main at
/// install time — invoke the installer JAR directly via `java -jar <installer>
/// --installClient <staging>`. This pin locks the structural reason install-time
/// invocation is wrong: Main reads Mojang LAUNCH argv, period.
///
/// If a future re-vendoring removes `--fml.mcVersion` from the constant pool
/// (would only happen if upstream rewrites Main to not parse FML argv — extremely
/// unlikely), this test fails loudly and the developer can decide whether the
/// new shape changes the launch-vs-install-time distinction. The Phase 12 launch
/// wiring will still need this constant-pool fact to compose the version JSON
/// argument template.
#[test]
fn forgewrapper_main_class_is_launch_time_entry_point() {
    let main_bytes = read_class_bytes("io/github/zekerzhayard/forgewrapper/installer/Main.class");
    assert_eq!(
        &main_bytes[..4],
        &[0xCA, 0xFE, 0xBA, 0xBE],
        "Main.class missing JVM class-file magic"
    );
    // The literal `--fml.mcVersion` is loaded via ldc at offset 27 of Main.main
    // (verified via `javap -v Main.class` LineNumberTable line 28: 25). Constant-
    // pool UTF-8 entries are stored as length-prefixed bytes, so a byte-substring
    // scan over the full .class bytes is sound.
    const FML_MC_VERSION: &[u8] = b"--fml.mcVersion";
    assert!(
        contains_bytes(&main_bytes, FML_MC_VERSION),
        "Main.class constant pool missing UTF-8 entry `--fml.mcVersion` — \
         Main is no longer the FML-argv-parsing launch-time entry point. \
         GAP-7-A-v3 structural pin: re-vendored ForgeWrapper may have moved \
         FML argv parsing elsewhere (or removed it). Re-evaluate whether \
         FORGE_WRAPPER_MAIN_CLASS is still launch-time-only and whether the \
         07.3-01 invariant (install does not use ForgeWrapper) still holds."
    );
}
