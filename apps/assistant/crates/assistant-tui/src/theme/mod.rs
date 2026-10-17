mod omarchy;

use ratatui::style::{Color, Style};

pub use omarchy::{find as find_omarchy, load_from as omarchy_load, spawn_watcher};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dark,
    Light,
}

/// Semantic colors shared by every screen.
#[derive(Clone, Debug)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub bg: Color,
    pub bg_deep: Color,
    pub bg_raised: Color,
    pub border: Color,
    pub selection: Color,
    pub selection_fg: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub muted: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub red: Color,
    pub yellow: Color,
    pub green: Color,
    pub cyan: Color,
    pub blue: Color,
    pub magenta: Color,
}

impl Theme {
    pub fn base(&self) -> Style {
        Style::default().bg(self.bg).fg(self.fg)
    }

    pub fn status(&self) -> Style {
        if self.kind == Kind::Light && !matches!(self.fg, Color::Reset) {
            Style::default().bg(self.fg).fg(self.bg)
        } else {
            Style::default().bg(self.bg_deep).fg(self.fg)
        }
    }

    pub fn accent_style(&self) -> Style {
        Style::default().bg(self.accent).fg(self.on_accent)
    }

    pub fn selected(&self) -> Style {
        Style::default().bg(self.selection).fg(self.selection_fg)
    }
}

#[derive(Clone, Copy)]
pub struct Choice {
    pub group: &'static str,
    pub id: &'static str,
    pub label: &'static str,
}

/// Built-in catalog. Omarchy is resolved from disk, not from this list's colors.
pub static CHOICES: &[Choice] = &[
    Choice {
        id: "aurora",
        label: "Aurora",
        group: "Built-in",
    },
    Choice {
        group: "System",
        id: "omarchy",
        label: "Follow Omarchy",
    },
    Choice {
        group: "System",
        id: "terminal",
        label: "Terminal",
    },
    Choice {
        group: "Dark",
        id: "catppuccin-mocha",
        label: "Catppuccin Mocha",
    },
    Choice {
        group: "Dark",
        id: "catppuccin-macchiato",
        label: "Catppuccin Macchiato",
    },
    Choice {
        group: "Dark",
        id: "catppuccin-frappe",
        label: "Catppuccin Frappé",
    },
    Choice {
        group: "Dark",
        id: "tokyo-night",
        label: "Tokyo Night",
    },
    Choice {
        group: "Dark",
        id: "tokyo-night-storm",
        label: "Tokyo Night Storm",
    },
    Choice {
        group: "Dark",
        id: "dracula",
        label: "Dracula",
    },
    Choice {
        group: "Dark",
        id: "gruvbox-dark",
        label: "Gruvbox Dark",
    },
    Choice {
        group: "Dark",
        id: "nord",
        label: "Nord",
    },
    Choice {
        group: "Dark",
        id: "one-dark-pro",
        label: "One Dark Pro",
    },
    Choice {
        group: "Dark",
        id: "solarized-dark",
        label: "Solarized Dark",
    },
    Choice {
        group: "Dark",
        id: "rose-pine",
        label: "Rosé Pine",
    },
    Choice {
        group: "Dark",
        id: "rose-pine-moon",
        label: "Rosé Pine Moon",
    },
    Choice {
        group: "Dark",
        id: "kanagawa",
        label: "Kanagawa",
    },
    Choice {
        group: "Dark",
        id: "everforest-dark",
        label: "Everforest Dark",
    },
    Choice {
        group: "Dark",
        id: "monokai-pro",
        label: "Monokai Pro",
    },
    Choice {
        group: "Light",
        id: "catppuccin-latte",
        label: "Catppuccin Latte",
    },
    Choice {
        group: "Light",
        id: "tokyo-night-day",
        label: "Tokyo Night Day",
    },
    Choice {
        group: "Light",
        id: "gruvbox-light",
        label: "Gruvbox Light",
    },
    Choice {
        group: "Light",
        id: "one-light",
        label: "One Light",
    },
    Choice {
        group: "Light",
        id: "solarized-light",
        label: "Solarized Light",
    },
    Choice {
        group: "Light",
        id: "rose-pine-dawn",
        label: "Rosé Pine Dawn",
    },
];

pub fn known(id: &str) -> bool {
    CHOICES.iter().any(|c| c.id == id)
}

pub fn by_id(id: &str) -> Option<Theme> {
    Some(match id {
        "aurora" => aurora(),
        "terminal" => terminal(),
        "catppuccin-mocha" => mocha(),
        "catppuccin-macchiato" => macchiato(),
        "catppuccin-frappe" => frappe(),
        "catppuccin-latte" => latte(),
        "tokyo-night" => tokyo_night(),
        "tokyo-night-storm" => tokyo_storm(),
        "tokyo-night-day" => tokyo_day(),
        "dracula" => dracula(),
        "gruvbox-dark" => gruvbox_dark(),
        "gruvbox-light" => gruvbox_light(),
        "nord" => nord(),
        "one-dark-pro" => one_dark(),
        "one-light" => one_light(),
        "solarized-dark" => solarized_dark(),
        "solarized-light" => solarized_light(),
        "rose-pine" => rose_pine(),
        "rose-pine-moon" => rose_moon(),
        "rose-pine-dawn" => rose_dawn(),
        "kanagawa" => kanagawa(),
        "everforest-dark" => everforest(),
        "monokai-pro" => monokai(),
        _ => return None,
    })
}

pub fn resolve(id: &str, omarchy: Option<&Theme>) -> Theme {
    if id == "omarchy" {
        if let Some(theme) = omarchy {
            return theme.clone();
        }
    }
    by_id(id).unwrap_or_else(mocha)
}

pub fn fallback_id(_omarchy_installed: bool) -> &'static str {
    "aurora"
}

fn terminal() -> Theme {
    Theme {
        id: "terminal".into(),
        name: "Terminal".into(),
        kind: Kind::Dark,
        bg: Color::Reset,
        bg_deep: Color::Reset,
        bg_raised: Color::Reset,
        border: Color::DarkGray,
        selection: Color::DarkGray,
        selection_fg: Color::White,
        fg: Color::Reset,
        fg_dim: Color::Gray,
        muted: Color::DarkGray,
        accent: Color::Cyan,
        on_accent: Color::Black,
        red: Color::Red,
        yellow: Color::Yellow,
        green: Color::Green,
        cyan: Color::Cyan,
        blue: Color::Blue,
        magenta: Color::Magenta,
    }
}

/// bg, deep, raised, border, selection, fg, dim, muted, accent, red, yellow, green, cyan, blue, magenta
fn pal(id: &'static str, name: &'static str, kind: Kind, c: [&str; 15]) -> Theme {
    let accent = hex(c[8]);
    let fg = hex(c[5]);
    Theme {
        id: id.into(),
        name: name.into(),
        kind,
        bg: hex(c[0]),
        bg_deep: hex(c[1]),
        bg_raised: hex(c[2]),
        border: hex(c[3]),
        selection: hex(c[4]),
        selection_fg: fg,
        fg,
        fg_dim: hex(c[6]),
        muted: hex(c[7]),
        accent,
        on_accent: on_color(accent),
        red: hex(c[9]),
        yellow: hex(c[10]),
        green: hex(c[11]),
        cyan: hex(c[12]),
        blue: hex(c[13]),
        magenta: hex(c[14]),
    }
}

pub fn hex(s: &str) -> Color {
    parse_hex(s).unwrap_or(Color::White)
}

pub fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

pub fn luminance(color: Color) -> f32 {
    match color {
        Color::Rgb(r, g, b) => 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32,
        Color::Black => 0.0,
        Color::White => 255.0,
        Color::Yellow | Color::Cyan | Color::Green | Color::Gray => 170.0,
        _ => 50.0,
    }
}

pub fn on_color(bg: Color) -> Color {
    if luminance(bg) > 150.0 {
        Color::Black
    } else {
        Color::Rgb(255, 255, 255)
    }
}

pub fn readable(bg: Color, prefer: Color, alt: Color) -> Color {
    let prefer_gap = (luminance(bg) - luminance(prefer)).abs();
    let alt_gap = (luminance(bg) - luminance(alt)).abs();
    if prefer_gap >= 90.0 || prefer_gap >= alt_gap {
        prefer
    } else {
        alt
    }
}

pub fn mocha() -> Theme {
    pal(
        "catppuccin-mocha",
        "Catppuccin Mocha",
        Kind::Dark,
        [
            "1e1e2e", "11111b", "313244", "45475a", "45475a", "cdd6f4", "bac2de", "6c7086",
            "cba6f7", "f38ba8", "f9e2af", "a6e3a1", "94e2d5", "89b4fa", "f5c2e7",
        ],
    )
}
fn macchiato() -> Theme {
    pal(
        "catppuccin-macchiato",
        "Catppuccin Macchiato",
        Kind::Dark,
        [
            "24273a", "181926", "363a4f", "494d64", "494d64", "cad3f5", "b8c0e0", "6e738d",
            "c6a0f6", "ed8796", "eed49f", "a6da95", "8bd5ca", "8aadf4", "f5bde6",
        ],
    )
}
fn frappe() -> Theme {
    pal(
        "catppuccin-frappe",
        "Catppuccin Frappé",
        Kind::Dark,
        [
            "303446", "232634", "414559", "51576d", "51576d", "c6d0f5", "b5bfe2", "737994",
            "ca9ee6", "e78284", "e5c890", "a6d189", "81c8be", "8caaee", "f4b8e4",
        ],
    )
}
fn latte() -> Theme {
    pal(
        "catppuccin-latte",
        "Catppuccin Latte",
        Kind::Light,
        [
            "eff1f5", "e6e9ef", "ccd0da", "acb0be", "bcc0cc", "4c4f69", "5c5f77", "8c8fa1",
            "8839ef", "d20f39", "df8e1d", "40a02b", "179299", "1e66f5", "ea76cb",
        ],
    )
}
fn tokyo_night() -> Theme {
    pal(
        "tokyo-night",
        "Tokyo Night",
        Kind::Dark,
        [
            "1a1b26", "16161e", "292e42", "3b4261", "33467c", "c0caf5", "a9b1d6", "565f89",
            "7aa2f7", "f7768e", "e0af68", "9ece6a", "7dcfff", "7aa2f7", "bb9af7",
        ],
    )
}
fn tokyo_storm() -> Theme {
    pal(
        "tokyo-night-storm",
        "Tokyo Night Storm",
        Kind::Dark,
        [
            "24283b", "1f2335", "292e42", "3b4261", "364a82", "c0caf5", "a9b1d6", "565f89",
            "7aa2f7", "f7768e", "e0af68", "9ece6a", "7dcfff", "7aa2f7", "bb9af7",
        ],
    )
}
fn tokyo_day() -> Theme {
    pal(
        "tokyo-night-day",
        "Tokyo Night Day",
        Kind::Light,
        [
            "e1e2e7", "d5d6db", "c4c8da", "a1a6c5", "c4c8da", "3760bf", "6172b0", "848cb5",
            "2e7de9", "f52a65", "8c6c3e", "587539", "007197", "2e7de9", "9854f1",
        ],
    )
}
fn dracula() -> Theme {
    pal(
        "dracula",
        "Dracula",
        Kind::Dark,
        [
            "282a36", "21222c", "343746", "44475a", "44475a", "f8f8f2", "f8f8f2", "6272a4",
            "bd93f9", "ff5555", "f1fa8c", "50fa7b", "8be9fd", "8be9fd", "ff79c6",
        ],
    )
}
fn gruvbox_dark() -> Theme {
    pal(
        "gruvbox-dark",
        "Gruvbox Dark",
        Kind::Dark,
        [
            "282828", "1d2021", "3c3836", "504945", "504945", "ebdbb2", "d5c4a1", "928374",
            "fe8019", "fb4934", "fabd2f", "b8bb26", "8ec07c", "83a598", "d3869b",
        ],
    )
}
fn gruvbox_light() -> Theme {
    pal(
        "gruvbox-light",
        "Gruvbox Light",
        Kind::Light,
        [
            "fbf1c7", "f2e5bc", "ebdbb2", "d5c4a1", "d5c4a1", "3c3836", "504945", "7c6f64",
            "d65d0e", "cc241d", "d79921", "98971a", "689d6a", "458588", "b16286",
        ],
    )
}
fn nord() -> Theme {
    pal(
        "nord",
        "Nord",
        Kind::Dark,
        [
            "2e3440", "242933", "3b4252", "434c5e", "434c5e", "d8dee9", "e5e9f0", "4c566a",
            "88c0d0", "bf616a", "ebcb8b", "a3be8c", "8fbcbb", "81a1c1", "b48ead",
        ],
    )
}
fn one_dark() -> Theme {
    pal(
        "one-dark-pro",
        "One Dark Pro",
        Kind::Dark,
        [
            "282c34", "21252b", "2c313c", "3e4451", "3e4451", "abb2bf", "b6bdca", "5c6370",
            "61afef", "e06c75", "e5c07b", "98c379", "56b6c2", "61afef", "c678dd",
        ],
    )
}
fn one_light() -> Theme {
    pal(
        "one-light",
        "One Light",
        Kind::Light,
        [
            "fafafa", "f0f0f0", "f0f0f0", "d0d0d0", "e5e5e6", "383a42", "696c77", "a0a1a7",
            "4078f2", "e45649", "c18401", "50a14f", "0184bc", "4078f2", "a626a4",
        ],
    )
}
fn solarized_dark() -> Theme {
    pal(
        "solarized-dark",
        "Solarized Dark",
        Kind::Dark,
        [
            "002b36", "00212b", "073642", "586e75", "073642", "839496", "93a1a1", "586e75",
            "268bd2", "dc322f", "b58900", "859900", "2aa198", "268bd2", "d33682",
        ],
    )
}
fn solarized_light() -> Theme {
    pal(
        "solarized-light",
        "Solarized Light",
        Kind::Light,
        [
            "fdf6e3", "eee8d5", "eee8d5", "93a1a1", "eee8d5", "657b83", "586e75", "93a1a1",
            "268bd2", "dc322f", "b58900", "859900", "2aa198", "268bd2", "d33682",
        ],
    )
}
fn rose_pine() -> Theme {
    pal(
        "rose-pine",
        "Rosé Pine",
        Kind::Dark,
        [
            "191724", "16141f", "1f1d2e", "524f67", "403d52", "e0def4", "908caa", "6e6a86",
            "c4a7e7", "eb6f92", "f6c177", "31748f", "9ccfd8", "9ccfd8", "c4a7e7",
        ],
    )
}
fn rose_moon() -> Theme {
    pal(
        "rose-pine-moon",
        "Rosé Pine Moon",
        Kind::Dark,
        [
            "232136", "1e1c2e", "2a273f", "56526e", "44415a", "e0def4", "908caa", "6e6a86",
            "c4a7e7", "eb6f92", "f6c177", "3e8fb0", "9ccfd8", "9ccfd8", "c4a7e7",
        ],
    )
}
fn rose_dawn() -> Theme {
    pal(
        "rose-pine-dawn",
        "Rosé Pine Dawn",
        Kind::Light,
        [
            "faf4ed", "fffaf3", "f2e9e1", "cecacd", "dfdad9", "575279", "797593", "9893a5",
            "907aa9", "b4637a", "ea9d34", "286983", "56949f", "56949f", "907aa9",
        ],
    )
}
fn kanagawa() -> Theme {
    pal(
        "kanagawa",
        "Kanagawa",
        Kind::Dark,
        [
            "1f1f28", "16161d", "2a2a37", "363646", "2d4f67", "dcd7ba", "c8c093", "727169",
            "7e9cd8", "c34043", "e6c384", "76946a", "7fb4ca", "7e9cd8", "957fb8",
        ],
    )
}
fn everforest() -> Theme {
    pal(
        "everforest-dark",
        "Everforest Dark",
        Kind::Dark,
        [
            "2d353b", "232a2e", "343f44", "475258", "543a48", "d3c6aa", "9da9a0", "859289",
            "a7c080", "e67e80", "dbbc7f", "a7c080", "83c092", "7fbbb3", "d699b6",
        ],
    )
}
fn monokai() -> Theme {
    pal(
        "monokai-pro",
        "Monokai Pro",
        Kind::Dark,
        [
            "2d2a2e", "221f22", "403e41", "5b595c", "403e41", "fcfcfa", "c1c0c0", "727072",
            "ffd866", "ff6188", "ffd866", "a9dc76", "78dce8", "78dce8", "ab9df2",
        ],
    )
}

pub fn aurora() -> Theme {
    let mut theme = pal(
        "aurora",
        "Aurora",
        Kind::Dark,
        [
            "171820", "14151d", "222432", "3d4054", "222432", "ececf4", "a5a9be", "a5a9be",
            "b5a0ff", "ed8796", "eac38c", "a3d6b0", "83d9dc", "b5a0ff", "b5a0ff",
        ],
    );
    theme.selection_fg = theme.accent;
    theme
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_the_popular_set() {
        let ids: Vec<_> = CHOICES.iter().map(|c| c.id).collect();
        for id in [
            "omarchy",
            "terminal",
            "dracula",
            "catppuccin-mocha",
            "catppuccin-macchiato",
            "catppuccin-frappe",
            "catppuccin-latte",
            "tokyo-night",
            "tokyo-night-storm",
            "tokyo-night-day",
            "one-dark-pro",
            "one-light",
            "gruvbox-dark",
            "gruvbox-light",
            "nord",
            "solarized-dark",
            "solarized-light",
            "rose-pine",
            "rose-pine-moon",
            "rose-pine-dawn",
            "kanagawa",
            "everforest-dark",
            "monokai-pro",
        ] {
            assert!(ids.contains(&id), "missing {id}");
            if id != "omarchy" {
                assert_eq!(by_id(id).unwrap().id, id);
            }
        }
    }

    #[test]
    fn omarchy_wins_when_present() {
        let custom = mocha();
        let mut custom = custom;
        custom.name = "Omarchy · catppuccin".into();
        let theme = resolve("omarchy", Some(&custom));
        assert_eq!(theme.name, "Omarchy · catppuccin");
        let fallback = resolve("omarchy", None);
        assert_eq!(fallback.id, "catppuccin-mocha");
        assert_eq!(fallback_id(false), "aurora");
        assert_eq!(fallback_id(true), "aurora");
    }
}
