//! Library rule evaluation.
//!
//! Spec: empty rules array -> include. Non-empty: start `included = false`,
//! iterate top-down; last matching rule's `action` wins. See
//! 02-RESEARCH.md §6 and PITFALLS.md Pitfall 3.
//!
//! Feature flags (Phase 3): RuleContext will grow a `features: HashSet<String>`
//! field; for now an empty default suffices and any feature-gated rule fails.

use std::collections::HashSet;

use super::types::Rule;
use crate::domain::platform::{Arch, OsName};

/// Evaluation context for rules. Captures OS, arch, and feature flags.
/// Phase 2 seeds features as an empty set; Phase 3 populates `is_demo_user`
/// and `has_custom_resolution` from the launch request.
#[derive(Debug, Clone)]
pub struct RuleContext {
    pub os: OsName,
    pub arch: Arch,
    pub features: HashSet<String>,
}

impl RuleContext {
    pub fn for_os_arch(os: OsName, arch: Arch) -> Self {
        Self {
            os,
            arch,
            features: HashSet::new(),
        }
    }

    pub fn current() -> Self {
        Self::for_os_arch(OsName::current(), Arch::current())
    }

    pub fn os_str(&self) -> &str {
        self.os.mojang_str()
    }

    pub fn arch_str(&self) -> &str {
        self.arch.mojang_str()
    }
}

/// Evaluate a library's rules against `ctx`.
///
/// `rules` evaluated top-down. Empty rules => `true` (include).
/// Rules that reference features not in `ctx.features` match falsely.
pub fn evaluate_rules(rules: &[Rule], ctx: &RuleContext) -> bool {
    if rules.is_empty() {
        return true;
    }
    let os_str = ctx.os_str();
    let arch_str = ctx.arch_str();
    let mut included = false;
    for rule in rules {
        let os_matches = rule.os.as_ref().is_none_or(|cond| {
            cond.name.as_deref().is_none_or(|n| n == os_str)
                && cond.arch.as_deref().is_none_or(|a| a == arch_str)
            // `version` regex: skip evaluation in v1 (treat as match-if-absent)
        });
        // Features: if the rule specifies a features object, every key-value
        // pair whose value is `true` must be present in ctx.features. Phase 2
        // uses an empty feature set, so any feature-gated rule fails to match.
        let features_match = match rule.features.as_ref() {
            None => true,
            Some(v) => features_all_satisfied(v, &ctx.features),
        };
        if os_matches && features_match {
            included = rule.action == "allow";
        }
    }
    included
}

/// True iff every entry in `features_obj` whose value is `true` is present
/// in `enabled`. Unknown keys treated conservatively — if the value is
/// `true` but the key is not in `enabled`, return false.
fn features_all_satisfied(features_obj: &serde_json::Value, enabled: &HashSet<String>) -> bool {
    let Some(obj) = features_obj.as_object() else {
        return false;
    };
    for (key, val) in obj {
        let required = matches!(val, serde_json::Value::Bool(true));
        if required && !enabled.contains(key) {
            return false;
        }
    }
    true
}
