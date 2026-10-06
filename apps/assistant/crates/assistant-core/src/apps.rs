use crate::paths;
use serde::Deserialize;
use std::fs;
use std::path::Path;

const SHIPPED: &str = include_str!("../../../config/apps.toml");

#[derive(Debug, Clone, Deserialize)]
pub struct AppsFile {
    #[serde(default)]
    pub app: Vec<AppLaunch>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppLaunch {
    pub phrases: Vec<String>,
    #[serde(default = "default_reply")]
    pub reply: String,
    #[serde(default)]
    pub omarchy: Vec<String>,
    #[serde(default)]
    pub exec: Vec<String>,
}

fn default_reply() -> String {
    "Opening.".into()
}

impl AppLaunch {
    pub fn args(&self) -> Vec<String> {
        if !self.exec.is_empty() {
            let mut v = vec!["exec".into()];
            v.extend(self.exec.iter().cloned());
            v
        } else {
            self.omarchy.clone()
        }
    }
}

impl AppsFile {
    pub fn shipped() -> Self {
        toml::from_str(SHIPPED).expect("shipped apps.toml")
    }

    pub fn load() -> crate::Result<Self> {
        Self::load_from(&paths::config_dir().join("apps.toml"))
    }

    pub fn load_from(path: &Path) -> crate::Result<Self> {
        if path.exists() {
            let text = fs::read_to_string(path)?;
            Ok(toml::from_str(&text)?)
        } else {
            Ok(Self::shipped())
        }
    }

    pub fn vocab(&self) -> Vec<String> {
        let mut words = Vec::new();
        for app in &self.app {
            for phrase in &app.phrases {
                for w in phrase.split_whitespace() {
                    if w.len() >= 4 && !words.iter().any(|x| x == w) {
                        words.push(w.to_string());
                    }
                }
            }
        }
        words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_apps_parse() {
        let apps = AppsFile::shipped();
        assert!(apps.app.iter().any(|a| a.phrases.iter().any(|p| p == "open terminal")));
        assert!(apps.app.iter().any(|a| a.omarchy == ["launch", "nautilus"]));
    }
}
