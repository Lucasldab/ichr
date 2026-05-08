// Programmatic `.mrpack` fixture builder for integration tests.
//
// This file is included into test files via `mod fixtures { ... include!(...) }`.
// It does NOT get compiled as a standalone build script.
//
// Provides `build_minimal_mrpack` -- a synchronous helper that constructs a
// minimal but spec-compliant `.mrpack` zip with:
// - `modrinth.index.json` (configurable deps, 1+ mod files)
// - `overrides/config/test.txt` (content: "from overrides")
// - `client-overrides/options.txt` (content: "from client-overrides")
//
// SHA-1 and SHA-512 hashes for mod entries are computed from `mod_payload` so
// the test can serve those exact bytes from httpmock and the hash verification
// passes end-to-end.

use std::io::Write as _;
use std::path::Path;

use sha2::{Digest, Sha512};
use zip::write::SimpleFileOptions;

/// Compute the SHA-512 hex digest of a byte slice.
pub fn sha512_hex_of(data: &[u8]) -> String {
    let mut h = Sha512::new();
    h.update(data);
    h.finalize()
        .iter()
        .fold(String::with_capacity(128), |mut s, b| {
            use std::fmt::Write as _;
            write!(s, "{b:02x}").unwrap();
            s
        })
}

/// Compute a stable (but not cryptographically meaningful) SHA-1 hex string
/// from the SHA-512 digest of `data` (first 20 bytes of SHA-512 → hex).
///
/// The ledger stores SHA-1 but does not verify it for correctness; any 40-char
/// hex is sufficient for test fixtures.
pub fn sha1_hex_of(data: &[u8]) -> String {
    let mut h = Sha512::new();
    h.update(data);
    let digest = h.finalize();
    // Take first 20 bytes of SHA-512 as a proxy for SHA-1 in tests.
    digest[..20]
        .iter()
        .fold(String::with_capacity(40), |mut s, b| {
            use std::fmt::Write as _;
            write!(s, "{b:02x}").unwrap();
            s
        })
}

/// A single mod entry for the manifest's `files[]` array.
pub struct ModEntry<'a> {
    /// Destination path relative to `.minecraft/`, e.g. `"mods/sodium.jar"`.
    pub path: &'a str,
    /// The bytes the mock server will return for this file.
    pub payload: &'a [u8],
    /// The URL where the file can be downloaded (typically a httpmock URL).
    pub download_url: &'a str,
    /// `env.client` value: `"required"`, `"optional"`, or `"unsupported"`.
    pub env_client: &'a str,
    /// `env.server` value: `"required"`, `"optional"`, or `"unsupported"`.
    pub env_server: &'a str,
}

/// Build a minimal `.mrpack` zip at `path`.
///
/// # Parameters
///
/// - `path` -- destination path (typically inside a `TempDir`).
/// - `pack_name` -- modpack display name; used for slug derivation.
/// - `mc_version` -- Minecraft version string (`"1.20.4"`, etc.).
/// - `loader_dep` -- optional `(key, version)` pair added to `dependencies`,
///   e.g. `Some(("fabric-loader", "0.16.9"))`.  Pass `None` for vanilla packs.
/// - `mods` -- slice of `ModEntry` structs describing each file in `files[]`.
/// - `include_overrides` -- when `true`, the zip includes:
///   - `overrides/config/test.txt` containing `"from overrides"`
///   - `client-overrides/options.txt` containing `"from client-overrides"`
///
/// Hashes in the manifest are computed from `mod.payload` so the test mock
/// server can serve the same bytes and SHA-512 verification passes.
pub fn build_minimal_mrpack(
    path: &Path,
    pack_name: &str,
    mc_version: &str,
    loader_dep: Option<(&str, &str)>,
    mods: &[ModEntry<'_>],
    include_overrides: bool,
) {
    let loader_fragment = match loader_dep {
        Some((key, ver)) => format!(r#", "{key}": "{ver}""#),
        None => String::new(),
    };

    let file_entries: Vec<String> = mods
        .iter()
        .map(|m| {
            let sha512 = sha512_hex_of(m.payload);
            let sha1 = sha1_hex_of(m.payload);
            format!(
                r#"{{
                    "path": "{}",
                    "hashes": {{ "sha1": "{sha1}", "sha512": "{sha512}" }},
                    "env": {{ "client": "{}", "server": "{}" }},
                    "downloads": ["{}"],
                    "fileSize": {}
                }}"#,
                m.path,
                m.env_client,
                m.env_server,
                m.download_url,
                m.payload.len(),
            )
        })
        .collect();

    let files_json = file_entries.join(", ");

    let manifest = format!(
        r#"{{
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": "0.1.0",
            "name": "{pack_name}",
            "summary": "Synthetic fixture for tests",
            "dependencies": {{ "minecraft": "{mc_version}"{loader_fragment} }},
            "files": [{files_json}]
        }}"#
    );

    let file = std::fs::File::create(path).expect("create mrpack file");
    let mut writer = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    // modrinth.index.json
    writer.start_file("modrinth.index.json", opts).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();

    if include_overrides {
        // overrides/config/test.txt
        writer.start_file("overrides/config/test.txt", opts).unwrap();
        writer.write_all(b"from overrides").unwrap();

        // client-overrides/options.txt
        writer.start_file("client-overrides/options.txt", opts).unwrap();
        writer.write_all(b"from client-overrides").unwrap();
    }

    writer.finish().unwrap();
}

/// Build a `.mrpack` that includes a path-traversal override entry.
///
/// The zip contains a `modrinth.index.json` with no files (no network needed)
/// and an `overrides/` entry with name `../../etc/passwd`. This is used by
/// `import_with_path_traversal_in_overrides_skips_safely` to prove that
/// `apply_overrides` silently skips malicious entries.
pub fn build_mrpack_with_path_traversal(path: &Path, mc_version: &str) {
    let manifest = format!(
        r#"{{
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": "0.1.0",
            "name": "Path Traversal Pack",
            "summary": "Adversarial fixture",
            "dependencies": {{ "minecraft": "{mc_version}" }},
            "files": []
        }}"#
    );

    let file = std::fs::File::create(path).expect("create traversal mrpack");
    let mut writer = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    writer.start_file("modrinth.index.json", opts).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();

    // Malicious override entry: prefix + traversal in relative path
    writer
        .start_file("overrides/../../etc/passwd", opts)
        .unwrap();
    writer.write_all(b"root:x:0:0:root:/root:/bin/bash\n").unwrap();

    writer.finish().unwrap();
}
