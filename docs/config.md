# Configuration

ichr looks for `config.toml` in the platform's user-config directory:

| Platform | Path |
|----------|------|
| Linux    | `~/.config/ichr/config.toml` |
| macOS    | `~/Library/Application Support/ichr/config.toml` |
| Windows  | `%APPDATA%\ichr\config.toml` |

The file is **optional**. If it's absent or empty, every setting uses
its built-in default. ichr never writes this file -- create it
yourself when you want to override something. Restart the launcher
after editing; there is no live reload.

Parse errors fall back to defaults plus a `tracing::warn!` line in
`ichr.log`. Unknown top-level fields, unknown color names, and
unknown keybind action names are *errors*, not silent ignores -- so
typos surface fast.

## Color palette

```toml
[colors]
# Primary accent: focused-input borders, active filter chips, the
# "running" instance badge. Default: yellow.
accent      = "yellow"
# Subdued tone for placeholders and inactive borders. Default: darkgray.
dim         = "darkgray"
# Errors. Default: red.
error       = "red"
# Successful actions, "installed" markers. Default: green.
success     = "green"
# Loading messages, hints. Default: cyan.
info        = "cyan"
# Default text color. Default: gray.
text        = "gray"
# Selected-row background highlight in lists. Default: blue.
selected_bg = "blue"
```

Color values may be either:

- A 16-color ANSI name (case-insensitive match against the lowercase
  list): `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`,
  `gray`, `darkgray`, `lightred`, `lightgreen`, `lightyellow`,
  `lightblue`, `lightmagenta`, `lightcyan`, `white`, `reset`.
- A 6-digit hex literal: `"#FFAA00"`.

Unknown names (e.g. `"yelow"`, `"orange"`) are rejected at load time.

Migration is incremental: not every styled element reads from this
palette yet. The most-visible ones (search-bar borders + filter chips
in the mod / resource pack / shader pack browsers) already do; deeper
chrome (modal headings, list-row colors) still uses the historical
hardcoded values. Each release moves more under the palette.

## Keybinds

```toml
[keybinds]
quit                       = "q"
open_create_instance       = "c"
open_modpack_import        = "i"
launch_instance            = "Enter"
open_loader_picker         = "L"
open_mod_browser           = "M"
open_installed_mods        = "m"
open_pack_resource_browser = "R"
open_pack_shader_browser   = "S"
open_accounts_list         = "A"
open_java_picker           = "J"
open_cf_browser            = "F"
browser_begin_search       = "/"
browser_toggle_mc_filter   = "v"
browser_toggle_loader_filter = "l"
```

Wire format:

- A bare letter, e.g. `"q"` or `"L"`. **Uppercase letter implies
  Shift** (so `"L"` is what currently opens the loader picker;
  `"l"` is the lowercase MC-filter toggle in browsers).
- Modifier prefixes joined with `+`: `"Ctrl+L"`, `"Ctrl+Shift+s"`,
  `"Alt+Tab"`. Modifier names are case-insensitive.
- Named keys: `Enter`, `Esc`, `Tab`, `BackTab`, `Backspace`, `Up`,
  `Down`, `Left`, `Right`, `Home`, `End`, `PageUp`, `PageDown`,
  `Delete`, `Insert`, `Space`, `F1` -- `F12`.

Slots not present in your `[keybinds]` table keep their default.
Unknown action names produce an error -- the file is rejected and
ichr falls back to the full default set with a warning logged. The
matcher is case-sensitive on letter codes (so `"l"` and `"L"` are
distinct slots) but case-insensitive when a `Ctrl` modifier is set
(some terminals report Ctrl-letter as either upper or lower case).

Migration is incremental for keybinds too: the list above covers
the global + instance-list + browser-shared keys. Per-view modal
keys (uninstall confirm, version picker, etc.) still use hardcoded
bindings.

## Example: dark theme + vim-leaning binds

```toml
[colors]
accent      = "lightcyan"
dim         = "#444444"
error       = "lightred"
success     = "lightgreen"
info        = "lightblue"
text        = "white"
selected_bg = "#003366"

[keybinds]
quit            = "Q"
launch_instance = "Enter"
# Open the mod browser with a leader-style chord -- Ctrl+M instead of M.
open_mod_browser = "Ctrl+M"
```
