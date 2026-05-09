//! Configurable keybinds. Closed enumeration of `ActionKey` slots, each
//! mapped to a `KeySpec` (key code + modifier bitmask). Defaults match
//! the historical hardcoded keys in `src/tui/run.rs` + per-view
//! `map_*_event` functions.
//!
//! Wire-format example:
//!
//! ```toml
//! [keybinds]
//! quit                  = "q"
//! launch_instance       = "Enter"
//! open_loader_picker    = "L"
//! open_mod_browser      = "M"
//! open_pack_resource    = "R"
//! open_pack_shader      = "S"
//! browser_begin_search  = "/"
//! ```
//!
//! Modifiers chain with `+`: `"Ctrl+L"`, `"Shift+Tab"`, `"Ctrl+Alt+s"`.
//! The matcher is case-insensitive on modifier names but preserves
//! letter-key case (so `"L"` matches Shift+L while `"l"` matches plain
//! l) -- this matches how crossterm reports the events.
//!
//! Slots not present in the user's TOML keep their default. Unknown
//! slots in the TOML are a parse error (handled at `Config::load`
//! by falling back to defaults; see `config::Config`).

use std::collections::HashMap;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

/// Every action that can be rebound. Adding a new action here is a
/// code change. Names are stable wire identifiers -- renaming one is a
/// breaking change to existing user configs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKey {
    // ── Global ──
    Quit,
    // ── Instance list ──
    OpenCreateInstance,
    OpenModpackImport,
    LaunchInstance,
    OpenLoaderPicker,
    OpenModBrowser,
    OpenInstalledMods,
    OpenPackResourceBrowser,
    OpenPackShaderBrowser,
    OpenAccountsList,
    OpenJavaPicker,
    OpenCfBrowser,
    // ── Browser shared ──
    BrowserBeginSearch,
    BrowserToggleMcFilter,
    BrowserToggleLoaderFilter,
}

/// A configurable key combination: code + modifier bitmask. Stored in
/// the wire format as a single string ("Ctrl+L", "Enter", etc.) and
/// parsed via the `Serialize`/`Deserialize` impls below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySpec {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeySpec {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// User-facing label (matches the wire format) for embedding in
    /// hint strings: `"L"`, `"Ctrl+L"`, `"Enter"`, etc. Used by
    /// renderers so on-screen prompts track user overrides instead of
    /// quoting hardcoded defaults.
    pub fn display(&self) -> String {
        keyspec_to_wire(self)
    }

    /// True iff the incoming `KeyEvent` matches this binding. Modifier
    /// match is exact (`Ctrl+L` does NOT match plain `L`); code match
    /// is exact for `Char(_)` (case-sensitive, deliberately).
    pub fn matches(&self, ev: &KeyEvent) -> bool {
        // crossterm reports Ctrl-modifiers with the shifted form
        // sometimes -- normalize by ignoring case mismatches when
        // modifiers carry CONTROL (so `"Ctrl+l"` and `"Ctrl+L"` both
        // work). For non-CONTROL bindings we keep case-sensitivity so
        // `"L"` (Shift+L) and `"l"` are distinct slots.
        if self.modifiers != ev.modifiers {
            return false;
        }
        match (self.code, ev.code) {
            (KeyCode::Char(a), KeyCode::Char(b)) => {
                if self.modifiers.contains(KeyModifiers::CONTROL) {
                    a.eq_ignore_ascii_case(&b)
                } else {
                    a == b
                }
            }
            (a, b) => a == b,
        }
    }
}

/// Map of `ActionKey` -> `KeySpec`. `Default` populates every slot
/// with the historical hardcoded binding. The wire format is a
/// flat TOML table keyed by snake_case action name.
#[derive(Debug, Clone)]
pub struct Keybinds {
    bindings: HashMap<ActionKey, KeySpec>,
}

impl Keybinds {
    /// True iff `ev` matches the binding currently configured for
    /// `action`. Returns `false` if the slot is somehow missing
    /// (shouldn't happen because `Default` populates everything; the
    /// guard is here to keep call sites total).
    pub fn matches(&self, action: ActionKey, ev: &KeyEvent) -> bool {
        self.bindings
            .get(&action)
            .map(|k| k.matches(ev))
            .unwrap_or(false)
    }

    /// Look up the spec for a slot. Used by help/hint text rendering
    /// so the displayed shortcut tracks the user's overrides.
    pub fn get(&self, action: ActionKey) -> Option<&KeySpec> {
        self.bindings.get(&action)
    }

    /// Convenience: resolve a slot to its user-facing label, or fall
    /// back to `"?"` if somehow unbound. `Default` populates every
    /// slot, so this should not happen at runtime; the fallback keeps
    /// hint text rendering total against malformed configs.
    pub fn label(&self, action: ActionKey) -> String {
        self.bindings
            .get(&action)
            .map(|k| k.display())
            .unwrap_or_else(|| "?".to_string())
    }
}

impl Default for Keybinds {
    fn default() -> Self {
        use ActionKey::*;
        use KeyCode::{Char, Enter};

        let none = KeyModifiers::NONE;
        let shift = KeyModifiers::SHIFT;

        let pairs: &[(ActionKey, KeySpec)] = &[
            (Quit, KeySpec::new(Char('q'), none)),
            (OpenCreateInstance, KeySpec::new(Char('c'), none)),
            (OpenModpackImport, KeySpec::new(Char('i'), none)),
            (LaunchInstance, KeySpec::new(Enter, none)),
            // Uppercase letters arrive as Char('X') + SHIFT modifier
            // from crossterm; encode them that way so `"L"` in TOML
            // round-trips cleanly.
            (OpenLoaderPicker, KeySpec::new(Char('L'), shift)),
            (OpenModBrowser, KeySpec::new(Char('M'), shift)),
            (OpenInstalledMods, KeySpec::new(Char('m'), none)),
            (OpenPackResourceBrowser, KeySpec::new(Char('R'), shift)),
            (OpenPackShaderBrowser, KeySpec::new(Char('S'), shift)),
            (OpenAccountsList, KeySpec::new(Char('A'), shift)),
            (OpenJavaPicker, KeySpec::new(Char('J'), shift)),
            (OpenCfBrowser, KeySpec::new(Char('F'), shift)),
            (BrowserBeginSearch, KeySpec::new(Char('/'), none)),
            (BrowserToggleMcFilter, KeySpec::new(Char('v'), none)),
            (BrowserToggleLoaderFilter, KeySpec::new(Char('l'), none)),
        ];
        Self {
            bindings: pairs.iter().copied().collect(),
        }
    }
}

// ── Serde glue for KeySpec: parse `"Ctrl+L"` etc. from a TOML string ──

impl Serialize for KeySpec {
    fn serialize<S>(&self, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ser.serialize_str(&keyspec_to_wire(self))
    }
}

impl<'de> Deserialize<'de> for KeySpec {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(de)?;
        keyspec_from_wire(&s).map_err(serde::de::Error::custom)
    }
}

// ── Serde glue for Keybinds: TOML table -> partial overrides on top of Default ──

impl Serialize for Keybinds {
    fn serialize<S>(&self, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Stable serialization order (sorted by snake_case action name)
        // makes generated config files diff-friendly.
        let mut entries: Vec<(String, KeySpec)> = self
            .bindings
            .iter()
            .map(|(k, v)| (action_to_snake(*k).to_string(), *v))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let map: std::collections::BTreeMap<_, _> = entries.into_iter().collect();
        map.serialize(ser)
    }
}

impl<'de> Deserialize<'de> for Keybinds {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Parse as a free-form map<String, KeySpec>, then merge over the
        // default bindings so a partial `[keybinds]` table only
        // overrides the slots the user explicitly listed.
        let raw: HashMap<String, KeySpec> = HashMap::deserialize(de)?;
        let mut binds = Keybinds::default();
        for (name, spec) in raw {
            let action = snake_to_action(&name).ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "unknown keybind action {name:?} -- must be one of: \
                     quit, open_create_instance, open_modpack_import, \
                     launch_instance, open_loader_picker, open_mod_browser, \
                     open_installed_mods, open_pack_resource_browser, \
                     open_pack_shader_browser, open_accounts_list, \
                     open_java_picker, open_cf_browser, browser_begin_search, \
                     browser_toggle_mc_filter, browser_toggle_loader_filter"
                ))
            })?;
            binds.bindings.insert(action, spec);
        }
        Ok(binds)
    }
}

fn action_to_snake(a: ActionKey) -> &'static str {
    match a {
        ActionKey::Quit => "quit",
        ActionKey::OpenCreateInstance => "open_create_instance",
        ActionKey::OpenModpackImport => "open_modpack_import",
        ActionKey::LaunchInstance => "launch_instance",
        ActionKey::OpenLoaderPicker => "open_loader_picker",
        ActionKey::OpenModBrowser => "open_mod_browser",
        ActionKey::OpenInstalledMods => "open_installed_mods",
        ActionKey::OpenPackResourceBrowser => "open_pack_resource_browser",
        ActionKey::OpenPackShaderBrowser => "open_pack_shader_browser",
        ActionKey::OpenAccountsList => "open_accounts_list",
        ActionKey::OpenJavaPicker => "open_java_picker",
        ActionKey::OpenCfBrowser => "open_cf_browser",
        ActionKey::BrowserBeginSearch => "browser_begin_search",
        ActionKey::BrowserToggleMcFilter => "browser_toggle_mc_filter",
        ActionKey::BrowserToggleLoaderFilter => "browser_toggle_loader_filter",
    }
}

fn snake_to_action(s: &str) -> Option<ActionKey> {
    Some(match s {
        "quit" => ActionKey::Quit,
        "open_create_instance" => ActionKey::OpenCreateInstance,
        "open_modpack_import" => ActionKey::OpenModpackImport,
        "launch_instance" => ActionKey::LaunchInstance,
        "open_loader_picker" => ActionKey::OpenLoaderPicker,
        "open_mod_browser" => ActionKey::OpenModBrowser,
        "open_installed_mods" => ActionKey::OpenInstalledMods,
        "open_pack_resource_browser" => ActionKey::OpenPackResourceBrowser,
        "open_pack_shader_browser" => ActionKey::OpenPackShaderBrowser,
        "open_accounts_list" => ActionKey::OpenAccountsList,
        "open_java_picker" => ActionKey::OpenJavaPicker,
        "open_cf_browser" => ActionKey::OpenCfBrowser,
        "browser_begin_search" => ActionKey::BrowserBeginSearch,
        "browser_toggle_mc_filter" => ActionKey::BrowserToggleMcFilter,
        "browser_toggle_loader_filter" => ActionKey::BrowserToggleLoaderFilter,
        _ => return None,
    })
}

// ── Wire-format string parser/serializer for KeySpec ─────────────────────

fn keyspec_to_wire(k: &KeySpec) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if k.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt");
    }
    if k.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("Shift");
    }
    let code_str = match k.code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        other => format!("{other:?}"),
    };
    let parts_str = parts.join("+");
    if parts_str.is_empty() {
        code_str
    } else {
        format!("{parts_str}+{code_str}")
    }
}

fn keyspec_from_wire(s: &str) -> Result<KeySpec, String> {
    let mut modifiers = KeyModifiers::NONE;
    let raw_parts: Vec<&str> = s.split('+').collect();
    let (mod_parts, code_part) = raw_parts
        .split_last()
        .map(|(last, rest)| (rest, *last))
        .ok_or_else(|| "empty keybind".to_string())?;
    for p in mod_parts {
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" | "meta" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            "" => return Err(format!("invalid keybind {s:?}: empty modifier segment")),
            other => return Err(format!("unknown modifier {other:?} in keybind {s:?}")),
        }
    }
    let code = parse_keycode(code_part).ok_or_else(|| format!("unknown key {code_part:?}"))?;
    // If the code is a single uppercase letter, encode SHIFT implicitly
    // so users can write `"L"` instead of `"Shift+L"`. Lowercase letters
    // stay unmodified.
    if let KeyCode::Char(c) = code {
        if c.is_ascii_uppercase() {
            modifiers |= KeyModifiers::SHIFT;
        }
    }
    Ok(KeySpec { code, modifiers })
}

fn parse_keycode(s: &str) -> Option<KeyCode> {
    if s.len() == 1 {
        return Some(KeyCode::Char(s.chars().next().unwrap()));
    }
    if let Some(rest) = s.strip_prefix('F').or_else(|| s.strip_prefix('f')) {
        if let Ok(n) = rest.parse::<u8>() {
            return Some(KeyCode::F(n));
        }
    }
    Some(match s.to_ascii_lowercase().as_str() {
        "space" => KeyCode::Char(' '),
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" => KeyCode::Backspace,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn defaults_match_historical_keys() {
        let kb = Keybinds::default();
        assert!(kb.matches(ActionKey::Quit, &ev(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(kb.matches(
            ActionKey::OpenLoaderPicker,
            &ev(KeyCode::Char('L'), KeyModifiers::SHIFT)
        ));
        assert!(kb.matches(
            ActionKey::OpenModBrowser,
            &ev(KeyCode::Char('M'), KeyModifiers::SHIFT)
        ));
        assert!(kb.matches(
            ActionKey::BrowserBeginSearch,
            &ev(KeyCode::Char('/'), KeyModifiers::NONE)
        ));
    }

    #[test]
    fn lowercase_does_not_match_uppercase_default() {
        let kb = Keybinds::default();
        assert!(!kb.matches(
            ActionKey::OpenLoaderPicker,
            &ev(KeyCode::Char('l'), KeyModifiers::NONE)
        ));
    }

    #[test]
    fn parse_simple_letter() {
        let k = keyspec_from_wire("q").unwrap();
        assert_eq!(k, KeySpec::new(KeyCode::Char('q'), KeyModifiers::NONE));
    }

    #[test]
    fn parse_uppercase_implies_shift() {
        let k = keyspec_from_wire("L").unwrap();
        assert_eq!(k, KeySpec::new(KeyCode::Char('L'), KeyModifiers::SHIFT));
    }

    #[test]
    fn parse_ctrl_letter() {
        let k = keyspec_from_wire("Ctrl+L").unwrap();
        assert_eq!(
            k,
            KeySpec::new(KeyCode::Char('L'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)
        );
    }

    #[test]
    fn parse_named_keys() {
        assert_eq!(
            keyspec_from_wire("Enter").unwrap(),
            KeySpec::new(KeyCode::Enter, KeyModifiers::NONE)
        );
        assert_eq!(
            keyspec_from_wire("F5").unwrap(),
            KeySpec::new(KeyCode::F(5), KeyModifiers::NONE)
        );
        assert_eq!(
            keyspec_from_wire("Shift+Tab").unwrap(),
            KeySpec::new(KeyCode::Tab, KeyModifiers::SHIFT)
        );
    }

    #[test]
    fn parse_unknown_key_errors() {
        assert!(keyspec_from_wire("Banana").is_err());
        assert!(keyspec_from_wire("Ctrl+").is_err());
        assert!(keyspec_from_wire("Hyper+a").is_err());
    }

    #[test]
    fn partial_toml_overrides_one_slot() {
        let kb: Keybinds = toml::from_str(r#"quit = "x""#).unwrap();
        assert!(kb.matches(ActionKey::Quit, &ev(KeyCode::Char('x'), KeyModifiers::NONE)));
        // Other slots keep their defaults.
        assert!(kb.matches(
            ActionKey::OpenLoaderPicker,
            &ev(KeyCode::Char('L'), KeyModifiers::SHIFT)
        ));
    }

    #[test]
    fn unknown_action_in_toml_errors() {
        let res: Result<Keybinds, _> = toml::from_str(r#"banana = "q""#);
        assert!(res.is_err());
    }

    #[test]
    fn ctrl_letter_is_case_insensitive() {
        // crossterm sometimes reports Ctrl-L as either upper or lower
        // case depending on the terminal; the matcher should accept
        // both for any binding that carries CONTROL.
        let spec = KeySpec::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
        assert!(spec.matches(&ev(KeyCode::Char('l'), KeyModifiers::CONTROL)));
        assert!(spec.matches(&ev(KeyCode::Char('L'), KeyModifiers::CONTROL)));
    }
}
