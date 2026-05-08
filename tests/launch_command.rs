//! Pure end-to-end snapshot of launcher::command::compose for the two
//! fixture versions already maintained in Phase 2. Catches regressions
//! in the composition layer without requiring Java or network.
//!
//! Runs in the default `cargo test` suite. NOT #[ignore]d.

use std::path::{Path, PathBuf};

use std::collections::HashMap;

use ichr::domain::platform::{Arch, OsName};
use ichr::launcher::command::compose;
use ichr::launcher::offline::{offline_auth, offline_uuid};
use ichr::mojang::inherits::resolve_inherits;
use ichr::mojang::rules::RuleContext;
use ichr::mojang::types::{ResolvedVersion, VersionJson};
use ichr::persistence::paths::AppPaths;

fn fixture_paths() -> AppPaths {
    AppPaths::with_roots(
        PathBuf::from("/data"),
        PathBuf::from("/config"),
        PathBuf::from("/cache"),
    )
}

/// Load a vanilla fixture and resolve to ResolvedVersion (compose takes
/// `&ResolvedVersion` after Phase 8.3). Vanilla fixtures declare
/// asset_index/assets/downloads inline, so resolve_inherits with an empty
/// parents map produces a clean ResolvedVersion (root_id == id).
fn load(path: &str) -> ResolvedVersion {
    let raw = std::fs::read_to_string(path).expect("fixture present");
    let v: VersionJson = serde_json::from_str(&raw).expect("fixture parses");
    resolve_inherits(&v, &HashMap::new())
        .expect("vanilla fixture resolves cleanly into ResolvedVersion")
}

fn assert_arg_pair(args: &[String], flag: &str, value: &str) {
    let found = args.windows(2).any(|w| w[0] == flag && w[1] == value);
    assert!(
        found,
        "expected `{flag} {value}` consecutively; got {args:?}"
    );
}

fn assert_no_placeholders(args: &[String]) {
    for a in args {
        assert!(
            !a.contains("${"),
            "unsubstituted ${{var}} token in argv: {a}"
        );
    }
}

#[test]
fn snapshot_compose_1_21_4() {
    let v = load("tests/fixtures/mojang/version_1_21_4.json");
    let auth = offline_auth("TestUser");
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    let cmd = compose(
        &v,
        &auth,
        &fixture_paths(),
        "myslug",
        &ctx,
        Path::new("java"),
    )
    .unwrap();

    assert_eq!(
        cmd.main_class, v.main_class,
        "main_class mismatch for 1.21.4"
    );
    assert!(
        cmd.jvm_args.iter().any(|a| a == "-cp"),
        "jvm_args must contain -cp; got {:?}",
        cmd.jvm_args
    );
    assert_arg_pair(&cmd.game_args, "--username", "TestUser");
    assert_arg_pair(&cmd.game_args, "--accessToken", "0");
    assert_arg_pair(&cmd.game_args, "--uuid", &offline_uuid("TestUser"));

    let all: Vec<&String> = cmd.jvm_args.iter().chain(cmd.game_args.iter()).collect();
    for a in &all {
        assert!(!a.contains("${"), "unsubstituted ${{var}} in argv: {a}");
    }
}

#[test]
fn snapshot_compose_1_12_2_legacy_args() {
    let v = load("tests/fixtures/mojang/version_1_12_2.json");
    let auth = offline_auth("TestUser");
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    let cmd = compose(
        &v,
        &auth,
        &fixture_paths(),
        "myslug",
        &ctx,
        Path::new("java"),
    )
    .unwrap();

    assert_eq!(
        cmd.main_class, v.main_class,
        "main_class mismatch for 1.12.2"
    );
    // Pre-1.13: arguments.jvm is absent -> LEGACY_JVM_ARGS baseline applies.
    assert!(
        cmd.jvm_args
            .iter()
            .any(|a| a.starts_with("-Djava.library.path=")),
        "legacy versions need -Djava.library.path; got {:?}",
        cmd.jvm_args
    );
    assert!(
        cmd.jvm_args.iter().any(|a| a == "-cp"),
        "jvm_args must contain -cp for legacy; got {:?}",
        cmd.jvm_args
    );
    // minecraftArguments includes ${auth_player_name}; must be substituted.
    assert_arg_pair(&cmd.game_args, "--username", "TestUser");
    assert_no_placeholders(&cmd.jvm_args);
    assert_no_placeholders(&cmd.game_args);
}

#[test]
fn snapshot_access_token_is_offline_placeholder_zero() {
    // AUTH-05: --accessToken "0" is the literal string used for offline mode.
    let v = load("tests/fixtures/mojang/version_1_21_4.json");
    let auth = offline_auth("OfflineDude");
    let ctx = RuleContext::for_os_arch(OsName::Linux, Arch::X86_64);
    let cmd = compose(&v, &auth, &fixture_paths(), "slug", &ctx, Path::new("java")).unwrap();
    assert_arg_pair(&cmd.game_args, "--accessToken", "0");
}

#[test]
fn snapshot_offline_uuid_is_version_3() {
    let uuid = offline_uuid("AnyName");
    // position 14 (0-indexed) is the version nibble; must be '3'
    assert_eq!(
        uuid.chars().nth(14),
        Some('3'),
        "offline UUID must be version 3; got {uuid}"
    );
    // position 19 must be one of RFC 4122 variants
    assert!(
        matches!(uuid.chars().nth(19), Some('8' | '9' | 'a' | 'b')),
        "offline UUID variant must be RFC 4122; got {uuid}"
    );
}
