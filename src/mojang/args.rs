//! Argument resolution for VERS-04.
//!
//! Collapses both Mojang JSON argument formats into a flat `Vec<String>`
//! of ordered tokens. Token substitution (`${auth_player_name}` etc.) is
//! Phase 3; this function ONLY handles:
//!   - legacy `minecraftArguments` (pre-1.13): split on ASCII whitespace
//!   - modern `arguments.game` / `arguments.jvm` (1.13+): iterate each
//!     `ArgumentEntry`, emit plain strings, evaluate `Conditional` rules
//!     against `RuleContext`, emit the value (single or multi) when allowed
//!
//! Precedence: if `arguments` is Some, it wins over `minecraft_arguments`.
//! If both are None, returns an empty vec.

use super::rules::{evaluate_rules, RuleContext};
use super::types::{ArgValue, ArgumentEntry, ResolvedVersion};

/// Resolve the ordered list of game-arg tokens for a given rule context.
pub fn resolve_game_args(version: &ResolvedVersion, ctx: &RuleContext) -> Vec<String> {
    if let Some(args) = version.arguments.as_ref() {
        return resolve_arg_entries(&args.game, ctx);
    }
    if let Some(legacy) = version.minecraft_arguments.as_ref() {
        return split_legacy_template(legacy);
    }
    Vec::new()
}

/// Resolve the ordered list of JVM-arg tokens for a given rule context.
///
/// Pre-1.13 versions have NO `arguments.jvm` block in their JSON. The
/// launcher (Phase 3) supplies the canonical legacy set (-cp / -Djava.library.path
/// / etc.) at launch time. For Phase 2, when the version provides no JVM
/// args, return an empty Vec — Phase 3 will fall back to its hard-coded
/// legacy baseline if the returned vec is empty.
pub fn resolve_jvm_args(version: &ResolvedVersion, ctx: &RuleContext) -> Vec<String> {
    if let Some(args) = version.arguments.as_ref() {
        return resolve_arg_entries(&args.jvm, ctx);
    }
    Vec::new()
}

/// Resolve both game and JVM args in a single sweep. Returns `(game, jvm)`.
pub fn resolve_arguments(version: &ResolvedVersion, ctx: &RuleContext) -> (Vec<String>, Vec<String>) {
    (resolve_game_args(version, ctx), resolve_jvm_args(version, ctx))
}

fn resolve_arg_entries(entries: &[ArgumentEntry], ctx: &RuleContext) -> Vec<String> {
    let mut out = Vec::new();
    for entry in entries {
        match entry {
            ArgumentEntry::Plain(s) => out.push(s.clone()),
            ArgumentEntry::Conditional(cond) => {
                if evaluate_rules(&cond.rules, ctx) {
                    match &cond.value {
                        ArgValue::Single(s) => out.push(s.clone()),
                        ArgValue::Multiple(vs) => out.extend(vs.iter().cloned()),
                    }
                }
            }
        }
    }
    out
}

/// Split a legacy `minecraftArguments` template string on ASCII whitespace,
/// preserving `${token}` placeholders intact. Empty tokens are dropped.
fn split_legacy_template(s: &str) -> Vec<String> {
    s.split_ascii_whitespace().map(|t| t.to_string()).collect()
}
