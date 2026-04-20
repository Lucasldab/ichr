//! Install orchestrator + natives extraction tests.

use std::io::Write;

use mineltui::install::natives_extract::extract_native_jar;
use tempfile::tempdir;

fn build_test_jar(dest: &std::path::Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(dest).unwrap();
    let mut zw = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        zw.start_file(*name, opts).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap();
}

#[tokio::test]
async fn test_extract_native_jar_basic() {
    let td = tempdir().unwrap();
    let jar = td.path().join("test.jar");
    let dest = td.path().join("extracted");

    let dll_bytes = b"fake dll content";
    build_test_jar(
        &jar,
        &[
            ("lwjgl.dll", dll_bytes),
            ("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\n"),
        ],
    );

    extract_native_jar(&jar, &dest, &["META-INF/".to_string()])
        .await
        .unwrap();

    // lwjgl.dll should be extracted
    assert!(dest.join("lwjgl.dll").exists(), "lwjgl.dll should be extracted");
    let extracted = std::fs::read(dest.join("lwjgl.dll")).unwrap();
    assert_eq!(&extracted, dll_bytes, "extracted bytes should match");

    // META-INF entries should be excluded
    assert!(
        !dest.join("META-INF").exists(),
        "META-INF dir should NOT be created"
    );
    assert!(
        !dest.join("META-INF/MANIFEST.MF").exists(),
        "META-INF/MANIFEST.MF should NOT be extracted"
    );
}

#[tokio::test]
async fn test_extract_native_jar_rejects_path_traversal() {
    let td = tempdir().unwrap();
    let jar = td.path().join("traversal.jar");
    let dest = td.path().join("safe_dest");
    std::fs::create_dir_all(&dest).unwrap();

    // Entry that would traverse above dest
    build_test_jar(&jar, &[("../../../etc/passwd", b"root:x:0:0")]);

    // Should succeed (skips the traversal entry silently)
    extract_native_jar(&jar, &dest, &[]).await.unwrap();

    // dest dir should be empty — traversal entry was skipped
    let entries: Vec<_> = std::fs::read_dir(&dest).unwrap().collect();
    assert!(entries.is_empty(), "dest should remain clean after traversal attempt");

    // The passwd file should NOT exist outside dest
    let victim = std::path::PathBuf::from("/etc/passwd_mineltui_test");
    assert!(!victim.exists());
}

#[tokio::test]
async fn test_extract_native_jar_skips_directories() {
    let td = tempdir().unwrap();
    let jar = td.path().join("dirs.jar");
    let dest = td.path().join("out");

    // Directory entry (trailing slash) plus a real file inside it
    build_test_jar(
        &jar,
        &[
            ("subdir/", b""),
            ("subdir/real.so", b"elf binary"),
        ],
    );

    extract_native_jar(&jar, &dest, &[]).await.unwrap();

    // The file inside subdir should be extracted
    assert!(dest.join("subdir").join("real.so").exists(), "real.so should be extracted");

    // The directory entry itself did not create a phantom entry
    assert!(dest.join("subdir").is_dir(), "subdir should exist as a directory");
}
