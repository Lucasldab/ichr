//! Named color palette. Slots are a *closed* enumeration -- adding a new
//! one is a code change. The rendering layer reads `Color::from(&spec)`
//! to convert a configured `ColorSpec` into the `ratatui::style::Color`
//! it actually paints with.
//!
//! Slot defaults match the historical hardcoded values; introducing this
//! module without migrating any render fn is a no-op.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// All configurable color slots. Each slot maps to one or more
/// rendering decisions across the TUI. Keep this list short -- a flood
/// of fine-grained slots makes config files unreadable; users prefer a
/// handful of semantic roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Palette {
    /// Primary accent: focused-input borders, active filter chips,
    /// the "running" badge, etc. Default: Yellow.
    pub accent: ColorSpec,
    /// Subdued / placeholder text and inactive borders. Default: DarkGray.
    pub dim: ColorSpec,
    /// Errors, fatal status. Default: Red.
    pub error: ColorSpec,
    /// Successful actions, "installed" markers. Default: Green.
    pub success: ColorSpec,
    /// Secondary informational tone (loading messages, hints).
    /// Default: Cyan.
    pub info: ColorSpec,
    /// Default text. Default: Gray.
    pub text: ColorSpec,
    /// Selected-row background highlight in lists. Default: Blue.
    pub selected_bg: ColorSpec,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            accent: ColorSpec::Named(NamedColor::Yellow),
            dim: ColorSpec::Named(NamedColor::DarkGray),
            error: ColorSpec::Named(NamedColor::Red),
            success: ColorSpec::Named(NamedColor::Green),
            info: ColorSpec::Named(NamedColor::Cyan),
            text: ColorSpec::Named(NamedColor::Gray),
            selected_bg: ColorSpec::Named(NamedColor::Blue),
        }
    }
}

/// A color value as expressed in TOML: either a named ANSI palette
/// entry (`"red"`, `"yellow"`) or a hex literal (`"#FFAA00"`). Stored
/// here in a struct-like enum so `serde` can deserialize either form
/// from a single TOML string via the custom (de)serializer below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpec {
    Named(NamedColor),
    Rgb(u8, u8, u8),
}

impl ColorSpec {
    /// Convert to the ratatui `Color` we actually paint with.
    pub fn to_color(self) -> Color {
        match self {
            ColorSpec::Named(n) => n.to_color(),
            ColorSpec::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }
    }
}

impl From<ColorSpec> for Color {
    fn from(c: ColorSpec) -> Color {
        c.to_color()
    }
}

impl From<&ColorSpec> for Color {
    fn from(c: &ColorSpec) -> Color {
        c.to_color()
    }
}

/// 16-color ANSI palette. We keep the named set explicit so typos in
/// config (e.g. `"yelow"`) are caught at deserialize time rather than
/// silently mapping to a fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NamedColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
    Reset,
}

impl NamedColor {
    pub fn to_color(self) -> Color {
        match self {
            NamedColor::Black => Color::Black,
            NamedColor::Red => Color::Red,
            NamedColor::Green => Color::Green,
            NamedColor::Yellow => Color::Yellow,
            NamedColor::Blue => Color::Blue,
            NamedColor::Magenta => Color::Magenta,
            NamedColor::Cyan => Color::Cyan,
            NamedColor::Gray => Color::Gray,
            NamedColor::DarkGray => Color::DarkGray,
            NamedColor::LightRed => Color::LightRed,
            NamedColor::LightGreen => Color::LightGreen,
            NamedColor::LightYellow => Color::LightYellow,
            NamedColor::LightBlue => Color::LightBlue,
            NamedColor::LightMagenta => Color::LightMagenta,
            NamedColor::LightCyan => Color::LightCyan,
            NamedColor::White => Color::White,
            NamedColor::Reset => Color::Reset,
        }
    }
}

// ── Serde glue: parse `"yellow"` or `"#FFAA00"` from a single string ──

impl Serialize for ColorSpec {
    fn serialize<S>(&self, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            ColorSpec::Named(n) => {
                // Map back through serde's lowercase rename so round-trip
                // produces the same string the user originally wrote.
                let s = match n {
                    NamedColor::Black => "black",
                    NamedColor::Red => "red",
                    NamedColor::Green => "green",
                    NamedColor::Yellow => "yellow",
                    NamedColor::Blue => "blue",
                    NamedColor::Magenta => "magenta",
                    NamedColor::Cyan => "cyan",
                    NamedColor::Gray => "gray",
                    NamedColor::DarkGray => "darkgray",
                    NamedColor::LightRed => "lightred",
                    NamedColor::LightGreen => "lightgreen",
                    NamedColor::LightYellow => "lightyellow",
                    NamedColor::LightBlue => "lightblue",
                    NamedColor::LightMagenta => "lightmagenta",
                    NamedColor::LightCyan => "lightcyan",
                    NamedColor::White => "white",
                    NamedColor::Reset => "reset",
                };
                ser.serialize_str(s)
            }
            ColorSpec::Rgb(r, g, b) => ser.serialize_str(&format!("#{r:02X}{g:02X}{b:02X}")),
        }
    }
}

impl<'de> Deserialize<'de> for ColorSpec {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(de)?;
        if let Some(hex) = s.strip_prefix('#') {
            if hex.len() != 6 {
                return Err(serde::de::Error::custom(format!(
                    "invalid hex color {s:?}: expected `#RRGGBB`"
                )));
            }
            let r = u8::from_str_radix(&hex[0..2], 16)
                .map_err(|e| serde::de::Error::custom(format!("invalid red byte: {e}")))?;
            let g = u8::from_str_radix(&hex[2..4], 16)
                .map_err(|e| serde::de::Error::custom(format!("invalid green byte: {e}")))?;
            let b = u8::from_str_radix(&hex[4..6], 16)
                .map_err(|e| serde::de::Error::custom(format!("invalid blue byte: {e}")))?;
            return Ok(ColorSpec::Rgb(r, g, b));
        }
        // Re-deserialize as a `NamedColor` via its serde rename.
        let named: NamedColor =
            serde::Deserialize::deserialize(serde::de::value::StrDeserializer::<
                serde::de::value::Error,
            >::new(&s))
            .map_err(|e| {
                serde::de::Error::custom(format!(
                    "unknown color {s:?}: must be one of black, red, green, yellow, \
                 blue, magenta, cyan, gray, darkgray, lightred, lightgreen, \
                 lightyellow, lightblue, lightmagenta, lightcyan, white, reset, \
                 or a #RRGGBB hex literal ({e})"
                ))
            })?;
        Ok(ColorSpec::Named(named))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(spec: ColorSpec) -> ColorSpec {
        let s = toml::to_string(&Wrapper { c: spec }).unwrap();
        let w: Wrapper = toml::from_str(&s).unwrap();
        w.c
    }

    #[derive(Serialize, Deserialize)]
    struct Wrapper {
        c: ColorSpec,
    }

    #[test]
    fn named_roundtrips() {
        assert_eq!(
            roundtrip(ColorSpec::Named(NamedColor::Yellow)),
            ColorSpec::Named(NamedColor::Yellow)
        );
        assert_eq!(
            roundtrip(ColorSpec::Named(NamedColor::DarkGray)),
            ColorSpec::Named(NamedColor::DarkGray)
        );
    }

    #[test]
    fn rgb_roundtrips() {
        assert_eq!(
            roundtrip(ColorSpec::Rgb(0xFF, 0xAA, 0)),
            ColorSpec::Rgb(0xFF, 0xAA, 0)
        );
    }

    #[test]
    fn parse_named_lowercase() {
        let w: Wrapper = toml::from_str("c = \"yellow\"").unwrap();
        assert_eq!(w.c, ColorSpec::Named(NamedColor::Yellow));
    }

    #[test]
    fn parse_hex() {
        let w: Wrapper = toml::from_str("c = \"#FFAA00\"").unwrap();
        assert_eq!(w.c, ColorSpec::Rgb(0xFF, 0xAA, 0));
    }

    #[test]
    fn parse_unknown_named_errors() {
        // `"yelow"` is a typo -- must be rejected, not silently mapped.
        assert!(toml::from_str::<Wrapper>("c = \"yelow\"").is_err());
    }

    #[test]
    fn parse_short_hex_errors() {
        assert!(toml::from_str::<Wrapper>("c = \"#FFF\"").is_err());
    }

    #[test]
    fn default_palette_matches_historical_colors() {
        let p = Palette::default();
        assert_eq!(Color::from(p.accent), Color::Yellow);
        assert_eq!(Color::from(p.dim), Color::DarkGray);
        assert_eq!(Color::from(p.error), Color::Red);
        assert_eq!(Color::from(p.success), Color::Green);
        assert_eq!(Color::from(p.info), Color::Cyan);
        assert_eq!(Color::from(p.text), Color::Gray);
        assert_eq!(Color::from(p.selected_bg), Color::Blue);
    }
}
