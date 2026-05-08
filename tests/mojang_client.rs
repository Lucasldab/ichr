//! Mojang cache + client tests.
//! Unit tests are network-free. Integration tests gated by #[ignore]:
//! run with `cargo test --test mojang_client -- --ignored`.

use std::time::Duration;

use ichr::mojang::cache::{
    atomic_write, cache_is_fresh, sha1_hex_of_bytes, verify_sha1, MANIFEST_CACHE_TTL,
};

// ---------------------------------------------------------------------------
// Task 2-03-01: Cache primitives
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_atomic_write_creates_file_and_parent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("deep").join("file.bin");
    atomic_write(&path, b"hello").await.unwrap();
    assert!(path.exists(), "file should exist after atomic_write");
    let contents = tokio::fs::read(&path).await.unwrap();
    assert_eq!(contents, b"hello");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atomic_write_overwrites_existing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    atomic_write(&path, b"old").await.unwrap();
    atomic_write(&path, b"new").await.unwrap();
    let contents = tokio::fs::read(&path).await.unwrap();
    assert_eq!(contents, b"new");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atomic_write_leaves_no_tmp_on_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    atomic_write(&path, b"hello").await.unwrap();
    let mut read_dir = tokio::fs::read_dir(dir.path()).await.unwrap();
    while let Some(entry) = read_dir.next_entry().await.unwrap() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        assert!(
            !name_str.ends_with(".tmp"),
            "no .tmp files should be left after successful atomic_write, found: {name_str}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sha1_hex_matches_rustcrypto_known_value() {
    let hash = sha1_hex_of_bytes(b"hello world");
    assert_eq!(hash, "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_verify_sha1_passes_on_match() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    let data = b"hello world";
    let expected = sha1_hex_of_bytes(data);
    tokio::fs::write(&path, data).await.unwrap();
    let result = verify_sha1(&path, &expected).await.unwrap();
    assert!(
        result,
        "verify_sha1 should return Ok(true) when hash matches"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_verify_sha1_fails_on_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    let data = b"hello world";
    tokio::fs::write(&path, data).await.unwrap();
    let wrong_hash = "0000000000000000000000000000000000000000";
    let result = verify_sha1(&path, wrong_hash).await.unwrap();
    assert!(
        !result,
        "verify_sha1 should return Ok(false) on hash mismatch"
    );

    // Missing file also returns Ok(false)
    let missing = dir.path().join("does_not_exist.bin");
    let result_missing = verify_sha1(&missing, wrong_hash).await.unwrap();
    assert!(
        !result_missing,
        "verify_sha1 should return Ok(false) for missing file"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cache_fresh_within_ttl() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    tokio::fs::write(&path, b"{}").await.unwrap();

    // File just written -- should be fresh
    let fresh = cache_is_fresh(&path, MANIFEST_CACHE_TTL).await.unwrap();
    assert!(fresh, "newly created file should be within TTL");

    // Backdate mtime by 2 hours
    let past = filetime::FileTime::from_unix_time(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 7200,
        0,
    );
    filetime::set_file_mtime(&path, past).unwrap();

    let stale = cache_is_fresh(&path, MANIFEST_CACHE_TTL).await.unwrap();
    assert!(!stale, "file backdated 2h should be stale with 1h TTL");

    // Missing file returns false (not an error)
    let missing = dir.path().join("missing.json");
    let missing_result = cache_is_fresh(&missing, Duration::from_secs(3600))
        .await
        .unwrap();
    assert!(
        !missing_result,
        "missing file should return Ok(false) from cache_is_fresh"
    );
}

// ---------------------------------------------------------------------------
// Task 2-03-02 tests added after client implementation
// ---------------------------------------------------------------------------

// These tests are in the same file but reference the client module.
// They will be compiled once src/mojang/client.rs is created.

#[tokio::test(flavor = "multi_thread")]
async fn test_client_builds_with_user_agent() {
    use ichr::mojang::client::{MojangClient, USER_AGENT};
    let client = MojangClient::new().expect("MojangClient::new should not fail");
    assert_eq!(client.user_agent(), USER_AGENT);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_download_verified_skips_when_already_correct() {
    use ichr::mojang::client::MojangClient;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    let data = b"correct data";
    tokio::fs::write(&path, data).await.unwrap();
    let expected_sha1 = sha1_hex_of_bytes(data);

    let client = MojangClient::new().unwrap();
    // URL that will definitely fail if contacted -- we guarantee no network call
    let unreachable_url = "http://127.0.0.1:1/nonexistent";
    let result = client
        .download_verified(unreachable_url, &path, &expected_sha1)
        .await;
    assert!(
        result.is_ok(),
        "download_verified should skip network when file already matches SHA1"
    );
    // File contents unchanged
    let after = tokio::fs::read(&path).await.unwrap();
    assert_eq!(after, data);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_download_verified_retries_once_on_mismatch() {
    use ichr::mojang::client::MojangClient;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("downloaded.bin");

    let correct_data = b"the correct content";
    let wrong_data = b"wrong!";
    let expected_sha1 = sha1_hex_of_bytes(correct_data);

    // Spin up a local server: first request returns wrong bytes, second returns correct bytes
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/");

    let responses: Vec<Vec<u8>> = vec![wrong_data.to_vec(), correct_data.to_vec()];
    tokio::spawn(serve_canned(listener, responses));

    let client = MojangClient::new().unwrap();
    client
        .download_verified(&url, &dest, &expected_sha1)
        .await
        .expect("download_verified should succeed after retry with correct bytes");

    let written = tokio::fs::read(&dest).await.unwrap();
    assert_eq!(
        written, correct_data,
        "file should contain the correct data after retry"
    );
}

/// Minimal hand-rolled HTTP/1.1 server that serves canned responses in sequence.
/// Each connection gets the next response body from `responses`.
async fn serve_canned(listener: tokio::net::TcpListener, mut responses: Vec<Vec<u8>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    while !responses.is_empty() {
        if let Ok((mut sock, _)) = listener.accept().await {
            let body = responses.remove(0);
            // Consume the request (read until we see the end of headers)
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(header.as_bytes()).await;
            let _ = sock.write_all(&body).await;
            let _ = sock.shutdown().await;
        }
    }
}

#[ignore]
#[tokio::test(flavor = "multi_thread")]
async fn test_fetch_manifest_live() {
    use ichr::mojang::client::MojangClient;
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("version_manifest_v2.json");
    let client = MojangClient::new().unwrap();
    let manifest = client
        .fetch_manifest(&cache_path)
        .await
        .expect("live manifest fetch failed");
    assert!(
        !manifest.latest.release.is_empty(),
        "latest.release should not be empty"
    );
    assert!(
        manifest.versions.len() > 100,
        "manifest should contain more than 100 versions, got {}",
        manifest.versions.len()
    );
}
