//! Native library classification: legacy classifier-style (< 1.19) vs
//! embedded-natives (1.19+, LWJGL 3.3.1+).
//!
//! Detection rule: a library needs extraction IFF it has BOTH a `natives`
//! top-level key AND a `downloads.classifiers` map. Either alone = not legacy.
//! See 02-RESEARCH.md §5 and PITFALLS.md Pitfall 4.

use super::types::{Library, LibraryArtifact};
use crate::domain::platform::OsName;

/// Returns `true` if this library uses the legacy classifier-style natives
/// (pre-1.19 / LWJGL 2.x or 3.2.x). Such libraries require the classifier
/// JAR to be downloaded and extracted into the per-instance natives dir.
pub fn needs_native_extraction(lib: &Library) -> bool {
    lib.natives.is_some() && lib.downloads.classifiers.is_some()
}

/// Pick the correct classifier JAR for the current OS from a legacy
/// classifier-style library. Returns `None` if the library is embedded-native
/// (1.19+) or if no classifier exists for this OS.
pub fn native_classifier_artifact(lib: &Library, os: OsName) -> Option<&LibraryArtifact> {
    let os_key = os.mojang_str();
    let classifier_key = lib.natives.as_ref()?.get(os_key)?;
    lib.downloads
        .classifiers
        .as_ref()?
        .get(classifier_key.as_str())
}

/// Return the Maven coordinate `group:artifact` prefix from a library's
/// `name` field. Used for inheritsFrom deduplication.
/// Input: `"group:artifact:version"` or `"group:artifact:version:classifier"`.
/// Output: `"group:artifact"`.
pub fn maven_group_artifact(name: &str) -> &str {
    let mut count = 0;
    for (i, c) in name.char_indices() {
        if c == ':' {
            count += 1;
            if count == 2 {
                return &name[..i];
            }
        }
    }
    name // fallback: malformed name — return as-is
}
