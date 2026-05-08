//! Windows @argfile writer — pure file-write used by the launch service on
//! Windows to bypass `CreateProcess` command-line length limits (MAX_PATH).
//!
//! See `.planning/research/PITFALLS.md` Pitfall 14 and
//! `.planning/phases/03-launcher-process-and-offline-launch/03-RESEARCH.md`
//! §"Pattern 5: Windows @argfile" for format rules.
//!
//! Format (Java 9+ @argfile spec):
//! - One argument per line.
//! - If an argument contains a space or a double-quote, wrap it in double-quotes
//!   and escape any internal double-quotes as `\"`.
//! - Backslash characters inside double-quoted paths do NOT need further
//!   escaping (Java's @argfile parser handles Windows paths natively).
//! - No shell interpretation; the file is parsed by the JVM launcher, not a shell.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::persistence::paths::AppPaths;

/// Canonical path for the per-instance @argfile.
///
/// Placed inside `instance_dir(slug)` so concurrent launches of different
/// instances never stomp each other. The file is overwritten on each launch.
pub fn argfile_path(paths: &AppPaths, slug: &str) -> PathBuf {
    paths.instance_dir(slug).join(".mineltui-argfile.txt")
}

/// Write `args` to `path` in Java @argfile format.
///
/// - One argument per line.
/// - Arguments containing a space or `"` are wrapped in double-quotes with
///   internal `"` escaped as `\"`.
/// - Parent directories are created if absent.
pub fn write_argfile(args: &[String], path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut content = String::new();
    for arg in args {
        if arg.contains(' ') || arg.contains('"') {
            let escaped = arg.replace('"', "\\\"");
            let _ = writeln!(content, "\"{escaped}\"");
        } else {
            let _ = writeln!(content, "{arg}");
        }
    }
    std::fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_argfile_plain_arg() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("args.txt");
        write_argfile(&["-cp".to_string()], &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "-cp\n");
    }

    #[test]
    fn test_argfile_quotes_path_with_spaces() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("args.txt");
        write_argfile(&["/home/my user/x.jar".to_string()], &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "\"/home/my user/x.jar\"\n");
    }

    #[test]
    fn test_argfile_escapes_internal_quote() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("args.txt");
        write_argfile(&["a\"b".to_string()], &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "\"a\\\"b\"\n");
    }

    #[test]
    fn test_argfile_creates_parent_dir() {
        let dir = TempDir::new().unwrap();
        let nested = dir
            .path()
            .join("nonexistent")
            .join("subdir")
            .join("args.txt");
        write_argfile(&["--foo".to_string()], &nested).unwrap();
        assert!(nested.exists(), "argfile must be created at nested path");
        assert!(
            nested.parent().unwrap().exists(),
            "parent dirs must be created"
        );
    }

    #[test]
    fn test_argfile_multiple_args() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("args.txt");
        let args = vec![
            "-Xmx2G".to_string(),
            "-cp".to_string(),
            "/path/to/file.jar".to_string(),
        ];
        write_argfile(&args, &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "-Xmx2G\n-cp\n/path/to/file.jar\n");
    }

    #[test]
    fn test_argfile_empty_args() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("args.txt");
        write_argfile(&[], &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "");
    }
}
