// Programmatic resource pack + shader pack fixture builder for integration tests.
//
// This file is included into test files via `mod fixtures { ... include!(...) }`.
// It does NOT get compiled as a standalone build script.
//
// Provides:
//   - `build_minimal_resource_pack` — synchronous helper that constructs a
//     minimal spec-compliant resource pack `.zip` with pack.mcmeta + an
//     empty-marker asset entry.
//   - `build_minimal_shader_pack` — synchronous helper that constructs a
//     minimal shader pack `.zip` with shaders/composite.fsh + shaders.properties.
//   - `sha1_hex_of` — compute real SHA-1 hex from bytes (for hash verification).
//
// Uses the synchronous `zip` crate per CLAUDE.md: "synchronous — modpack
// extract is not the hot path".

use std::io::Write as _;
use std::path::Path;

use sha1::Digest as _;
use sha1::Sha1;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// Compute the SHA-1 hex digest of a byte slice.
///
/// Used by tests to compute the expected hash the service will store in the
/// ledger (`sha512` field with `hash_algo = Sha1`).
pub fn sha1_hex_of(data: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(data);
    h.finalize()
        .iter()
        .fold(String::with_capacity(40), |mut s, b| {
            use std::fmt::Write as _;
            write!(s, "{b:02x}").unwrap();
            s
        })
}

/// Write a minimal valid resource pack zip to `path`.
///
/// Per minecraft.wiki/w/Resource_pack:
/// - `pack.mcmeta` REQUIRED at root (JSON `{"pack":{"pack_format":N,"description":"..."}}`)
/// - `pack.png` OPTIONAL (omitted — not needed for a functional fixture)
/// - At least one entry under `assets/` so the directory is non-empty
///
/// Returns the raw bytes of the zip (re-read from disk) so callers can compute
/// the SHA-1 hash that the service will verify during install.
pub fn build_minimal_resource_pack(path: &Path, pack_format: u32) -> Vec<u8> {
    let f = std::fs::File::create(path).expect("create resource pack zip");
    let mut w = zip::ZipWriter::new(f);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    // Root manifest — required by Minecraft to recognise as a resource pack.
    w.start_file("pack.mcmeta", opts).unwrap();
    w.write_all(
        format!(
            r#"{{"pack":{{"pack_format":{pack_format},"description":"test fixture"}}}}"#
        )
        .as_bytes(),
    )
    .unwrap();

    // Non-empty asset marker — some scanners require assets/ to be non-empty.
    w.start_file("assets/minecraft/.gitkeep", opts).unwrap();
    w.write_all(b"").unwrap();

    w.finish().unwrap();

    // Re-read from disk so callers can compute the SHA-1 the service stores.
    std::fs::read(path).expect("read back resource pack zip")
}

/// Write a minimal shader pack zip to `path`.
///
/// Per the Iris/OptiFine shader pack spec:
/// - `shaders/<name>.fsh` — at least one fragment shader
/// - `shaders.properties` — optional but expected by Iris discovery
///
/// Returns the raw bytes of the zip (re-read from disk) so callers can compute
/// the SHA-1 hash that the service will verify during install.
pub fn build_minimal_shader_pack(path: &Path) -> Vec<u8> {
    let f = std::fs::File::create(path).expect("create shader pack zip");
    let mut w = zip::ZipWriter::new(f);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    // Stub fragment shader — Iris requires at least one .fsh file.
    w.start_file("shaders/composite.fsh", opts).unwrap();
    w.write_all(b"// stub fragment shader\nvoid main() { gl_FragColor = vec4(1.0); }\n")
        .unwrap();

    // shaders.properties — optional but Iris-discoverable marker.
    w.start_file("shaders.properties", opts).unwrap();
    w.write_all(b"").unwrap();

    w.finish().unwrap();

    // Re-read from disk so callers can compute the SHA-1 the service stores.
    std::fs::read(path).expect("read back shader pack zip")
}
