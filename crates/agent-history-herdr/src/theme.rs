//! Resolves the theme configured for Herdr into the browser palette.
//!
//! Herdr does not publish its palette to the panes it hosts: it styles its
//! own chrome and leaves panes on the terminal emulator's colors. To match
//! Herdr, this module reads Herdr's `config.toml` and looks the theme name up
//! in copies of Herdr's built-in palettes.
//!
//! Those copies can drift from Herdr. They are taken from Herdr v0.7.5
//! `src/app/state.rs` (`Palette::catppuccin` … `Palette::vesper` and
//! `Palette::from_name`), which are unchanged since v0.7.1. To limit the
//! damage when Herdr changes:
//! - `[theme.custom]` and `[ui] accent` are read live, never copied;
//! - a theme name not in the copied table falls back to the terminal palette
//!   rather than to a guess;
//! - `terminal` maps to the browser's own ANSI palette, not a copy.
//!
//! Nothing here fails: a missing, unreadable or malformed config, or an
//! unparseable color, degrades to the terminal palette or the base token.
use agent_history_tui::{Color, Palette};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Where Herdr reads its config, following Herdr's own precedence:
/// `HERDR_CONFIG_PATH`, then `$XDG_CONFIG_HOME/herdr`, then `~/.config/herdr`.
pub fn config_path(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(path) = env("HERDR_CONFIG_PATH").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    if let Some(dir) = env("XDG_CONFIG_HOME").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(dir).join("herdr/config.toml"));
    }
    env("HOME")
        .filter(|p| !p.is_empty())
        .map(|home| PathBuf::from(home).join(".config/herdr/config.toml"))
}

/// The palette for the Herdr config in the process environment.
pub fn palette_from_env() -> Palette {
    config_path(|name| std::env::var_os(name))
        .map_or_else(Palette::terminal, |path| palette_from_file(&path))
}

pub fn palette_from_file(path: &Path) -> Palette {
    std::fs::read_to_string(path).map_or_else(|_| Palette::terminal(), |text| resolve(&text))
}

/// Resolves config text the way Herdr resolves its effective theme.
pub fn resolve(config: &str) -> Palette {
    let Ok(config) = config.parse::<toml::Table>() else {
        return Palette::terminal();
    };
    let section = |name: &str| config.get(name).and_then(toml::Value::as_table);
    let theme = section("theme");
    let string = |key: &str| theme.and_then(|t| t.get(key)).and_then(toml::Value::as_str);
    let name = string("name").unwrap_or("catppuccin");
    let auto_switch = theme
        .and_then(|t| t.get("auto_switch"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    // Herdr picks the light or dark name from the host terminal's reported
    // background and assumes dark when it cannot tell. This process does not
    // query the terminal, so it takes Herdr's dark default.
    let effective = if auto_switch {
        string("dark_name")
            .map(str::to_string)
            .unwrap_or_else(|| dark_sibling(name))
    } else {
        name.to_string()
    };
    let Some(mut palette) = builtin(&effective) else {
        return Palette::terminal();
    };
    let custom = theme
        .and_then(|t| t.get("custom"))
        .and_then(toml::Value::as_table);
    if let Some(custom) = custom {
        for (token, value) in custom {
            if let (Some(slot), Some(color)) = (
                token_mut(&mut palette, token),
                value.as_str().and_then(parse_color),
            ) {
                *slot = color;
            }
        }
    }
    let custom_accent = custom.is_some_and(|c| c.contains_key("accent"));
    let legacy_accent = section("ui")
        .and_then(|ui| ui.get("accent"))
        .and_then(toml::Value::as_str)
        .filter(|accent| !accent.eq_ignore_ascii_case("cyan"));
    if let (false, Some(color)) = (custom_accent, legacy_accent.and_then(parse_color)) {
        palette.accent = color;
    }
    palette
}

fn token_mut<'a>(palette: &'a mut Palette, token: &str) -> Option<&'a mut Color> {
    Some(match token {
        "accent" => &mut palette.accent,
        "panel_bg" => &mut palette.panel_bg,
        "surface0" => &mut palette.surface0,
        "surface1" => &mut palette.surface1,
        "surface_dim" => &mut palette.surface_dim,
        "overlay0" => &mut palette.overlay0,
        "overlay1" => &mut palette.overlay1,
        "text" => &mut palette.text,
        "subtext0" => &mut palette.subtext0,
        "mauve" => &mut palette.mauve,
        "green" => &mut palette.green,
        "yellow" => &mut palette.yellow,
        "red" => &mut palette.red,
        "blue" => &mut palette.blue,
        "teal" => &mut palette.teal,
        "peach" => &mut palette.peach,
        _ => return None,
    })
}

/// Herdr's color syntax: `#rrggbb`, `#rgb`, `rgb(r, g, b)`, reset aliases and
/// ANSI names. Unlike Herdr, an unknown value is rejected instead of
/// becoming cyan, so the base theme's token is kept.
pub fn parse_color(value: &str) -> Option<Color> {
    let s = value.trim().to_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        let digits: Option<Vec<u8>> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect();
        return match digits?.as_slice() {
            [r1, r2, g1, g2, b1, b2] => Some(Color::Rgb(r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2)),
            [r, g, b] => Some(Color::Rgb(r * 17, g * 17, b * 17)),
            _ => None,
        };
    }
    if let Some(inner) = s.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<u8> = inner
            .split(',')
            .map(|p| p.trim().parse::<u8>().ok())
            .collect::<Option<_>>()?;
        return match parts.as_slice() {
            [r, g, b] => Some(Color::Rgb(*r, *g, *b)),
            _ => None,
        };
    }
    Some(match s.as_str() {
        "reset" | "default" | "none" | "transparent" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        _ => return None,
    })
}

fn normalize(name: &str) -> String {
    name.to_lowercase().replace([' ', '_'], "-")
}

/// Herdr's default dark theme for a configured name (`sibling_theme_names`).
fn dark_sibling(name: &str) -> String {
    let name = normalize(name);
    let dark = match name.as_str() {
        "catppuccin-latte" | "latte" | "light" => "catppuccin",
        "tokyo-night-day" | "tokyo-day" | "tokyonight-day" => "tokyo-night",
        "gruvbox-light" => "gruvbox",
        "one-light" | "onelight" => "one-dark",
        "solarized-light" => "solarized",
        "kanagawa-lotus" | "lotus" => "kanagawa",
        "rose-pine-dawn" | "rosepine-dawn" | "dawn" => "rose-pine",
        _ => return name,
    };
    dark.to_string()
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// One of Herdr's built-in themes, by any name Herdr accepts for it.
pub fn builtin(name: &str) -> Option<Palette> {
    let name = normalize(name);
    THEMES
        .iter()
        .find(|(names, _)| names.contains(&name.as_str()))
        .map(|(_, palette)| *palette)
}

/// Built-in theme names Herdr lists, in Herdr's order.
pub const THEME_NAMES: &[&str] = &[
    "catppuccin",
    "catppuccin-latte",
    "terminal",
    "tokyo-night",
    "tokyo-night-day",
    "dracula",
    "nord",
    "gruvbox",
    "gruvbox-light",
    "one-dark",
    "one-light",
    "solarized",
    "solarized-light",
    "kanagawa",
    "kanagawa-lotus",
    "rose-pine",
    "rose-pine-dawn",
    "vesper",
];

/// Copied from Herdr v0.7.5 `src/app/state.rs`; see the module docs.
/// Token order: accent, panel_bg, surface0, surface1, surface_dim, overlay0,
/// overlay1, text, subtext0, mauve, green, yellow, red, blue, teal, peach.
#[rustfmt::skip]
const THEMES: &[(&[&str], Palette)] = &[
    (&["catppuccin", "catppuccin-mocha"], p([rgb(137,180,250), rgb(24,24,37), rgb(49,50,68), rgb(69,71,90), rgb(30,30,46), rgb(108,112,134), rgb(127,132,156), rgb(205,214,244), rgb(166,173,200), rgb(203,166,247), rgb(166,227,161), rgb(249,226,175), rgb(243,139,168), rgb(137,180,250), rgb(148,226,213), rgb(250,179,135)])),
    (&["catppuccin-latte", "latte", "light"], p([rgb(30,102,245), rgb(239,241,245), rgb(204,208,218), rgb(188,192,204), rgb(230,233,239), rgb(156,160,176), rgb(140,143,161), rgb(76,79,105), rgb(108,111,133), rgb(136,57,239), rgb(64,160,43), rgb(223,142,29), rgb(210,15,57), rgb(30,102,245), rgb(23,146,153), rgb(254,100,11)])),
    (&["terminal"], Palette::terminal()),
    (&["tokyo-night", "tokyonight"], p([rgb(122,162,247), rgb(26,27,38), rgb(36,40,59), rgb(65,72,104), rgb(26,27,38), rgb(86,95,137), rgb(105,113,150), rgb(192,202,245), rgb(169,177,214), rgb(187,154,247), rgb(158,206,106), rgb(224,175,104), rgb(247,118,142), rgb(122,162,247), rgb(125,207,255), rgb(255,158,100)])),
    (&["tokyo-night-day", "tokyo-day", "tokyonight-day"], p([rgb(46,125,233), rgb(225,226,231), rgb(196,200,218), rgb(168,174,203), rgb(210,211,218), rgb(137,144,179), rgb(104,112,154), rgb(55,96,191), rgb(97,114,176), rgb(120,71,189), rgb(88,117,57), rgb(140,108,62), rgb(245,42,101), rgb(46,125,233), rgb(17,140,116), rgb(177,92,0)])),
    (&["dracula"], p([rgb(189,147,249), rgb(40,42,54), rgb(68,71,90), rgb(98,114,164), rgb(40,42,54), rgb(98,114,164), rgb(130,140,180), rgb(248,248,242), rgb(210,210,220), rgb(255,121,198), rgb(80,250,123), rgb(241,250,140), rgb(255,85,85), rgb(139,233,253), rgb(139,233,253), rgb(255,184,108)])),
    (&["nord"], p([rgb(136,192,208), rgb(46,52,64), rgb(59,66,82), rgb(67,76,94), rgb(46,52,64), rgb(76,86,106), rgb(100,110,130), rgb(236,239,244), rgb(216,222,233), rgb(180,142,173), rgb(163,190,140), rgb(235,203,139), rgb(191,97,106), rgb(129,161,193), rgb(143,188,187), rgb(208,135,112)])),
    (&["gruvbox", "gruvbox-dark"], p([rgb(215,153,33), rgb(40,40,40), rgb(60,56,54), rgb(80,73,69), rgb(40,40,40), rgb(146,131,116), rgb(168,153,132), rgb(235,219,178), rgb(213,196,161), rgb(211,134,155), rgb(184,187,38), rgb(250,189,47), rgb(251,73,52), rgb(131,165,152), rgb(142,192,124), rgb(254,128,25)])),
    (&["gruvbox-light"], p([rgb(7,102,120), rgb(251,241,199), rgb(235,219,178), rgb(213,196,161), rgb(242,229,188), rgb(146,131,116), rgb(124,111,100), rgb(60,56,54), rgb(80,73,69), rgb(143,63,113), rgb(121,116,14), rgb(181,118,20), rgb(157,0,6), rgb(7,102,120), rgb(66,123,88), rgb(175,58,3)])),
    (&["one-dark", "onedark"], p([rgb(97,175,239), rgb(40,44,52), rgb(44,49,58), rgb(62,68,81), rgb(40,44,52), rgb(92,99,112), rgb(115,122,135), rgb(171,178,191), rgb(150,156,168), rgb(198,120,221), rgb(152,195,121), rgb(229,192,123), rgb(224,108,117), rgb(97,175,239), rgb(86,182,194), rgb(209,154,102)])),
    (&["one-light", "onelight"], p([rgb(64,120,242), rgb(250,250,250), rgb(240,240,241), rgb(229,229,230), rgb(245,245,246), rgb(160,161,167), rgb(104,107,119), rgb(56,58,66), rgb(104,107,119), rgb(166,38,164), rgb(80,161,79), rgb(193,132,1), rgb(228,86,73), rgb(64,120,242), rgb(1,132,188), rgb(152,104,1)])),
    (&["solarized", "solarized-dark"], p([rgb(38,139,210), rgb(0,43,54), rgb(7,54,66), rgb(88,110,117), rgb(0,43,54), rgb(88,110,117), rgb(101,123,131), rgb(147,161,161), rgb(131,148,150), rgb(211,54,130), rgb(133,153,0), rgb(181,137,0), rgb(220,50,47), rgb(38,139,210), rgb(42,161,152), rgb(203,75,22)])),
    (&["solarized-light"], p([rgb(38,139,210), rgb(253,246,227), rgb(238,232,213), rgb(147,161,161), rgb(238,232,213), rgb(147,161,161), rgb(88,110,117), rgb(101,123,131), rgb(131,148,150), rgb(211,54,130), rgb(133,153,0), rgb(181,137,0), rgb(220,50,47), rgb(38,139,210), rgb(42,161,152), rgb(203,75,22)])),
    (&["kanagawa"], p([rgb(126,156,216), rgb(31,31,40), rgb(42,42,55), rgb(54,54,70), rgb(31,31,40), rgb(114,113,105), rgb(135,134,125), rgb(220,215,186), rgb(200,195,170), rgb(149,127,184), rgb(118,148,106), rgb(192,163,110), rgb(195,64,67), rgb(126,156,216), rgb(127,180,202), rgb(255,160,102)])),
    (&["kanagawa-lotus", "lotus"], p([rgb(77,105,155), rgb(242,236,188), rgb(220,213,172), rgb(201,203,209), rgb(213,206,163), rgb(160,156,172), rgb(138,137,128), rgb(84,84,100), rgb(67,67,108), rgb(98,76,131), rgb(111,137,78), rgb(119,113,63), rgb(200,64,83), rgb(77,105,155), rgb(78,140,162), rgb(204,109,0)])),
    (&["rose-pine", "rosepine"], p([rgb(196,167,231), rgb(25,23,36), rgb(31,29,46), rgb(38,35,58), rgb(25,23,36), rgb(110,106,134), rgb(144,140,170), rgb(224,222,244), rgb(200,197,220), rgb(196,167,231), rgb(49,116,143), rgb(246,193,119), rgb(235,111,146), rgb(49,116,143), rgb(156,207,216), rgb(234,154,151)])),
    (&["rose-pine-dawn", "rosepine-dawn", "dawn"], p([rgb(144,122,169), rgb(250,244,237), rgb(242,233,225), rgb(255,250,243), rgb(242,233,225), rgb(152,147,165), rgb(121,117,147), rgb(70,66,97), rgb(121,117,147), rgb(144,122,169), rgb(40,105,131), rgb(234,157,52), rgb(180,99,122), rgb(40,105,131), rgb(86,148,159), rgb(215,130,126)])),
    (&["vesper"], p([rgb(255,199,153), rgb(26,26,26), rgb(35,35,35), rgb(40,40,40), rgb(16,16,16), rgb(92,92,92), rgb(126,126,126), rgb(255,255,255), rgb(160,160,160), rgb(255,209,168), rgb(153,255,228), rgb(255,199,153), rgb(255,128,128), rgb(176,176,176), rgb(102,221,204), rgb(255,199,153)])),
];

const fn p(c: [Color; 16]) -> Palette {
    Palette {
        accent: c[0],
        panel_bg: c[1],
        surface0: c[2],
        surface1: c[3],
        surface_dim: c[4],
        overlay0: c[5],
        overlay1: c[6],
        text: c[7],
        subtext0: c[8],
        mauve: c[9],
        green: c[10],
        yellow: c[11],
        red: c[12],
        blue: c[13],
        teal: c[14],
        peach: c[15],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const MOCHA_ACCENT: Color = Color::Rgb(137, 180, 250);

    fn temp_config(name: &str, body: &str) -> (PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("agent-history-theme-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, body).unwrap();
        (dir, path)
    }

    #[test]
    fn valid_config_selects_the_named_builtin() {
        let (dir, path) = temp_config(
            "valid",
            "[theme]\nauto_switch = false\nname = \"Tokyo Night\"\n[ui]\naccent = \"cyan\"\n",
        );
        let palette = palette_from_file(&path);
        assert_eq!(palette, builtin("tokyo-night").unwrap());
        assert_eq!(palette.accent, Color::Rgb(122, 162, 247));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_file_falls_back_to_the_terminal() {
        let path = std::env::temp_dir().join("agent-history-theme-absent/config.toml");
        assert_eq!(palette_from_file(&path), Palette::terminal());
    }

    #[test]
    fn config_without_a_theme_uses_herdrs_default() {
        assert_eq!(resolve(""), builtin("catppuccin").unwrap());
        assert_eq!(resolve("[ui]\nsound = true\n").accent, MOCHA_ACCENT);
    }

    #[test]
    fn unknown_theme_name_falls_back_to_the_terminal() {
        assert_eq!(
            resolve("[theme]\nname = \"neon-2099\"\n"),
            Palette::terminal()
        );
        assert_eq!(
            resolve("[theme]\nname = \"terminal\"\n"),
            Palette::terminal()
        );
    }

    #[test]
    fn malformed_toml_or_types_never_fail() {
        assert_eq!(resolve("[theme\nname ="), Palette::terminal());
        // A wrongly typed name is ignored like an absent one.
        assert_eq!(resolve("[theme]\nname = 7\n").accent, MOCHA_ACCENT);
    }

    #[test]
    fn malformed_custom_color_keeps_the_base_token() {
        let palette = resolve(
            "[theme]\nname = \"nord\"\n[theme.custom]\naccent = \"#12345z\"\nred = \"#abcd\"\ngreen = 5\nblue = \"rgb(1, 2, 300)\"\n",
        );
        assert_eq!(palette, builtin("nord").unwrap());
    }

    #[test]
    fn custom_tokens_override_the_base_theme() {
        let palette = resolve(
            "[theme]\nname = \"nord\"\n[theme.custom]\naccent = \"#F5C2E7\"\nred = \"rgb(255, 85, 85)\"\npanel_bg = \"reset\"\nteal = \"#0f0\"\nunknown_token = \"#000000\"\n",
        );
        let nord = builtin("nord").unwrap();
        assert_eq!(palette.accent, Color::Rgb(0xf5, 0xc2, 0xe7));
        assert_eq!(palette.red, Color::Rgb(255, 85, 85));
        assert_eq!(palette.panel_bg, Color::Reset);
        assert_eq!(palette.teal, Color::Rgb(0, 255, 0));
        assert_eq!(palette.text, nord.text);
    }

    #[test]
    fn legacy_ui_accent_applies_unless_a_custom_accent_is_set() {
        assert_eq!(
            resolve("[ui]\naccent = \"magenta\"\n").accent,
            Color::Magenta
        );
        assert_eq!(
            resolve("[ui]\naccent = \"magenta\"\n[theme.custom]\naccent = \"#000000\"\n").accent,
            Color::Rgb(0, 0, 0)
        );
    }

    #[test]
    fn auto_switch_uses_the_dark_theme() {
        let palette = resolve("[theme]\nname = \"gruvbox-light\"\nauto_switch = true\n");
        assert_eq!(palette, builtin("gruvbox").unwrap());
        let palette =
            resolve("[theme]\nname = \"nord\"\nauto_switch = true\ndark_name = \"dracula\"\n");
        assert_eq!(palette, builtin("dracula").unwrap());
    }

    #[test]
    fn every_listed_theme_resolves() {
        for name in THEME_NAMES {
            assert!(builtin(name).is_some(), "{name}");
        }
        assert_eq!(builtin("Catppuccin_Mocha"), builtin("catppuccin"));
    }

    #[test]
    fn config_path_follows_herdr_precedence() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                vars.iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        assert_eq!(
            config_path(env(&[("HERDR_CONFIG_PATH", "/x/c.toml"), ("HOME", "/h")])),
            Some(PathBuf::from("/x/c.toml"))
        );
        assert_eq!(
            config_path(env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/h")])),
            Some(PathBuf::from("/xdg/herdr/config.toml"))
        );
        assert_eq!(
            config_path(env(&[("HOME", "/h")])),
            Some(PathBuf::from("/h/.config/herdr/config.toml"))
        );
        assert_eq!(config_path(env(&[])), None);
    }
}
