//! Slug generation for instance directory names.
//!
//! Policy (locked 02-RESEARCH.md §"Slug Generation"):
//!   - lowercase
//!   - whitespace → '-'
//!   - strip all chars that aren't [a-z0-9-]
//!   - collapse runs of '-' to single '-'
//!   - trim leading/trailing '-'
//!   - truncate to 40 chars
//!   - empty result → "instance"
//!   - collision → append '-N' where N starts at 2

use std::path::Path;

/// Maximum slug length (excluding any `-N` suffix appended by `unique_slug`).
pub const MAX_SLUG_LEN: usize = 40;

/// Convert a display name into a filesystem-safe slug.
///
/// Security: strips ALL non-`[a-z0-9-]` characters before path construction,
/// preventing path injection (T-2-04-01).
pub fn slugify(display_name: &str) -> String {
    let lower = display_name.to_lowercase();
    // Replace any whitespace with '-'.
    let whitespace_replaced: String = lower
        .chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .collect();
    // Keep only [a-z0-9-]; drop all else (strips unicode, punctuation, etc.).
    let stripped: String = whitespace_replaced
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    // Collapse multiple '-' into one.
    let mut collapsed = String::with_capacity(stripped.len());
    let mut prev_dash = false;
    for c in stripped.chars() {
        if c == '-' {
            if !prev_dash {
                collapsed.push('-');
            }
            prev_dash = true;
        } else {
            collapsed.push(c);
            prev_dash = false;
        }
    }
    // Trim leading/trailing '-'.
    let trimmed = collapsed.trim_matches('-').to_string();
    // Truncate at char boundary; all chars are ASCII at this point so byte == char.
    let truncated = if trimmed.len() > MAX_SLUG_LEN {
        trimmed[..MAX_SLUG_LEN].to_string()
    } else {
        trimmed
    };
    if truncated.is_empty() {
        "instance".to_string()
    } else {
        truncated
    }
}

/// Find a non-colliding slug given a base.
///
/// If `{instances_dir}/{base}` does not exist, returns `base`. Otherwise
/// returns `base-2`, `base-3`, ... until a non-existing directory is found.
/// Safety valve at N=9999 (T-2-04-04).
pub async fn unique_slug(base: &str, instances_dir: &Path) -> String {
    if !tokio::fs::try_exists(instances_dir.join(base))
        .await
        .unwrap_or(false)
    {
        return base.to_string();
    }
    let mut n: u32 = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !tokio::fs::try_exists(instances_dir.join(&candidate))
            .await
            .unwrap_or(false)
        {
            return candidate;
        }
        n += 1;
        if n > 9999 {
            // Safety valve: something is very wrong if we get here.
            return format!("{base}-{n}");
        }
    }
}
