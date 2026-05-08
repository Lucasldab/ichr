//! Integration tests for `mineltui::observability::logging`.
//!
//! NOTE: `tracing_subscriber::try_init()` installs a process-global subscriber.
//! Multiple `#[test]` functions in this binary share that state, so we keep
//! the primary verification in ONE test that exercises init + write + ANSI
//! check. The "double init returns Err" assertion is made inline in the same
//! test to avoid racing with separate threads.

use std::io::Read;
use std::time::Duration;

use mineltui::observability::logging;
use mineltui::persistence::AppPaths;

#[test]
fn init_writes_info_to_data_dir_log_file_without_ansi() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();

    // Point all three roots at the tempdir; logging only reads `data_dir`.
    let paths = AppPaths::with_roots(root.clone(), root.clone(), root.clone());

    // Act — init, emit, drop guard to flush.
    let guard = logging::init(&paths).expect("logging::init should succeed");

    tracing::info!("test-message-abc");
    tracing::warn!("caution-xyz");

    // Dropping the guard flushes the non-blocking appender's background thread.
    drop(guard);

    // Give the filesystem a moment to settle (tracing-appender flushes on drop,
    // but some platforms buffer at the OS level).
    std::thread::sleep(Duration::from_millis(50));

    let log_path = paths.log_file();
    assert!(
        log_path.exists(),
        "log file {} should exist",
        log_path.display()
    );

    let mut contents = String::new();
    std::fs::File::open(&log_path)
        .expect("open log file")
        .read_to_string(&mut contents)
        .expect("read log file");

    assert!(
        contents.contains("test-message-abc"),
        "log should contain the info message; got: {contents:?}"
    );
    assert!(
        contents.contains("INFO"),
        "log should have INFO level tag; got: {contents:?}"
    );
    assert!(
        contents.contains("caution-xyz"),
        "log should contain the warn message; got: {contents:?}"
    );
    assert!(
        contents.contains("WARN"),
        "log should have WARN level tag; got: {contents:?}"
    );

    // ANSI check: the byte sequence ESC (0x1B) + '[' must NOT appear.
    let ansi_csi = &[0x1Bu8, b'['];
    assert!(
        !contents.as_bytes().windows(2).any(|w| w == ansi_csi),
        "log file must not contain ANSI CSI escapes (0x1B 0x5B); got: {contents:?}"
    );

    // Double-init: must return Err (global subscriber already set).
    let second = logging::init(&paths);
    assert!(
        second.is_err(),
        "second logging::init should return Err; got Ok"
    );
}

#[test]
fn log_path_matches_app_paths_log_file() {
    // This test does NOT call logging::init (to avoid global-subscriber conflict).
    // It just asserts the contract we rely on: log_file == data_dir/mineltui.log.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    let paths = AppPaths::with_roots(root.clone(), root.clone(), root.clone());

    let expected = root.join("mineltui.log");
    assert_eq!(paths.log_file(), expected);
}
