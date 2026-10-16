//! The TUI's own settings, apart from the assistant's `config.toml` (which the daemon
//! validates): `~/.config/nixe-assistant/tui.toml`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TuiConfig {
    /// `aurora`, `omarchy`, `terminal` or a built-in palette.
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_theme() -> String {
    "aurora".into()
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self { theme: default_theme() }
    }
}

pub fn path() -> PathBuf {
    assistant_core::paths::config_dir().join("tui.toml")
}

impl TuiConfig {
    /// The saved choice; Aurora when there is none or it names a theme that no longer exists.
    pub fn load(path: &Path) -> Self {
        let mut cfg: Self = std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
        if !crate::theme::known(&cfg.theme) {
            cfg.theme = default_theme();
        }
        cfg
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        assistant_core::config::atomic_write(path, toml::to_string(self)?.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_an_explicit_choice_and_defaults_to_aurora() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tui.toml");
        assert_eq!(TuiConfig::load(&path).theme, "aurora");
        TuiConfig { theme: "omarchy".into() }.save(&path).unwrap();
        assert_eq!(TuiConfig::load(&path).theme, "omarchy");
        std::fs::write(&path, "theme = \"gone\"").unwrap();
        assert_eq!(TuiConfig::load(&path).theme, "aurora");
    }
}
