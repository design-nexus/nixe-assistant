//! Dynamic theme matching Omarchy desktop themes or falling back cleanly.

use notify::Watcher;
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dark,
    Light,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Theme {
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

    pub fn card(&self) -> Style {
        Style::default().bg(self.bg_raised).fg(self.fg)
    }

    pub fn selected(&self) -> Style {
        Style::default()
            .bg(self.selection)
            .fg(self.selection_fg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn accent_text(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn muted_text(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn focus_border(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn success(&self) -> Style {
        Style::default().fg(self.green).add_modifier(Modifier::BOLD)
    }

    pub fn warning(&self) -> Style {
        Style::default()
            .fg(self.yellow)
            .add_modifier(Modifier::BOLD)
    }

    pub fn error(&self) -> Style {
        Style::default().fg(self.red).add_modifier(Modifier::BOLD)
    }

    pub fn tab_active(&self) -> Style {
        Style::default()
            .bg(self.accent)
            .fg(self.on_accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_inactive(&self) -> Style {
        Style::default().bg(self.bg_deep).fg(self.muted)
    }
}

pub fn default_dark() -> Theme {
    Theme {
        name: "Default Dark".into(),
        kind: Kind::Dark,
        bg: hex("1e1e2e"),
        bg_deep: hex("181825"),
        bg_raised: hex("313244"),
        border: hex("45475a"),
        selection: hex("585b70"),
        selection_fg: hex("cdd6f4"),
        fg: hex("cdd6f4"),
        fg_dim: hex("a6adc8"),
        muted: hex("6c7086"),
        accent: hex("89b4fa"),
        on_accent: hex("11111b"),
        red: hex("f38ba8"),
        yellow: hex("f9e2af"),
        green: hex("a6e3a1"),
        cyan: hex("89dceb"),
        blue: hex("89b4fa"),
        magenta: hex("cba6f7"),
    }
}

pub fn find_omarchy() -> Option<(String, Theme, PathBuf)> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/state/omarchy/current"));
        dirs.push(home.join(".config/omarchy/current"));
    }
    for dir in dirs {
        if let Some((name, theme)) = load_from(&dir) {
            return Some((name, theme, dir));
        }
    }
    None
}

pub fn load_from(dir: &Path) -> Option<(String, Theme)> {
    let colors_path = dir.join("theme").join("colors.toml");
    let text = std::fs::read_to_string(colors_path).ok()?;
    let name = std::fs::read_to_string(dir.join("theme.name"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let name = if name.is_empty() {
        "omarchy".into()
    } else {
        name
    };
    let theme = theme_from_str(&text, &name).ok()?;
    Some((name, theme))
}

#[derive(Debug, Deserialize)]
struct RawColors {
    mode: Option<String>,
    accent: Option<String>,
    selection: Option<String>,
    background: Option<String>,
    foreground: Option<String>,
    dark_background: Option<String>,
    darker_background: Option<String>,
    lighter_background: Option<String>,
    light_foreground: Option<String>,
    dark_foreground: Option<String>,
    bright_foreground: Option<String>,
    muted: Option<String>,
    red: Option<String>,
    bright_red: Option<String>,
    green: Option<String>,
    bright_green: Option<String>,
    yellow: Option<String>,
    bright_yellow: Option<String>,
    blue: Option<String>,
    bright_blue: Option<String>,
    magenta: Option<String>,
    bright_magenta: Option<String>,
    cyan: Option<String>,
    bright_cyan: Option<String>,
}

pub fn theme_from_str(text: &str, name: &str) -> Result<Theme, String> {
    let raw: RawColors = toml::from_str(text).map_err(|e| e.to_string())?;
    let background = parse_color(&raw.background).ok_or("missing background")?;
    let foreground = parse_color(&raw.foreground).ok_or("missing foreground")?;
    let deep = first_color(&[
        &raw.dark_background,
        &raw.darker_background,
        &raw.background,
    ])
    .unwrap_or(background);
    let raised =
        first_color(&[&raw.lighter_background, &raw.selection, &raw.background]).unwrap_or(deep);
    let border = first_color(&[&raw.muted, &raw.selection, &raw.dark_foreground]).unwrap_or(raised);
    let selection =
        first_color(&[&raw.selection, &raw.lighter_background, &raw.muted]).unwrap_or(raised);
    let dim = first_color(&[
        &raw.light_foreground,
        &raw.foreground,
        &raw.bright_foreground,
    ])
    .unwrap_or(foreground);
    let muted =
        first_color(&[&raw.dark_foreground, &raw.muted, &raw.light_foreground]).unwrap_or(dim);
    let accent = first_color(&[&raw.accent, &raw.blue, &raw.cyan]).unwrap_or(hex("89b4fa"));
    let red = first_color(&[&raw.red, &raw.bright_red]).unwrap_or(hex("f38ba8"));
    let yellow = first_color(&[&raw.yellow, &raw.bright_yellow]).unwrap_or(hex("f9e2af"));
    let green = first_color(&[&raw.green, &raw.bright_green]).unwrap_or(hex("a6e3a1"));
    let cyan = first_color(&[&raw.cyan, &raw.bright_cyan]).unwrap_or(hex("89dceb"));
    let blue = first_color(&[&raw.blue, &raw.bright_blue]).unwrap_or(hex("89b4fa"));
    let magenta = first_color(&[&raw.magenta, &raw.bright_magenta]).unwrap_or(hex("cba6f7"));

    let kind = if raw.mode.as_deref() == Some("light") {
        Kind::Light
    } else {
        Kind::Dark
    };
    let on_accent = if kind == Kind::Light {
        hex("ffffff")
    } else {
        hex("11111b")
    };
    let selection_fg = foreground;

    Ok(Theme {
        name: format!("Omarchy · {name}"),
        kind,
        bg: background,
        bg_deep: deep,
        bg_raised: raised,
        border,
        selection,
        selection_fg,
        fg: foreground,
        fg_dim: dim,
        muted,
        accent,
        on_accent,
        red,
        yellow,
        green,
        cyan,
        blue,
        magenta,
    })
}

fn first_color(opts: &[&Option<String>]) -> Option<Color> {
    for c in opts.iter().copied().flatten() {
        if let Some(parsed) = parse_hex(c) {
            return Some(parsed);
        }
    }
    None
}

fn parse_color(opt: &Option<String>) -> Option<Color> {
    opt.as_ref().and_then(|s| parse_hex(s))
}

pub fn hex(s: &str) -> Color {
    parse_hex(s).unwrap_or(Color::Reset)
}

pub fn parse_hex(raw: &str) -> Option<Color> {
    let clean = raw.trim().trim_start_matches('#');
    if clean.len() != 6 || !clean.is_ascii() {
        return None;
    }
    let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
    let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
    let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

pub fn spawn_watcher<F>(dir: PathBuf, mut on_change: F) -> Option<notify::RecommendedWatcher>
where
    F: FnMut() + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx).ok()?;
    let watch_path = dir.join("theme");
    let target = if watch_path.is_dir() { watch_path } else { dir };
    watcher
        .watch(&target, notify::RecursiveMode::Recursive)
        .ok()?;
    std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            if let Ok(event) = event {
                // Reading theme files emits Access events; reloading those loops forever.
                if matches!(
                    event.kind,
                    notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_)
                ) {
                    on_change();
                }
            }
        }
    });
    Some(watcher)
}
