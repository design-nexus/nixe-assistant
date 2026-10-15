//! Windows computer use never touches: password managers, terminals (a typed line runs as a
//! command), authentication prompts, lock screens and the assistant's own windows.

use anyhow::{bail, Result};
use assistant_omarchy::hypr;

pub const DEFAULT_DENY: &[&str] = &[
    "1password", "keepassxc", "bitwarden", "org.keepassxc", "polkit", "hyprpolkitagent", "gcr-prompter", "pinentry",
    "foot", "kitty", "alacritty", "ghostty", "wezterm", "org.wezfurlong.wezterm", "xterm", "konsole", "terminal",
    "hyprlock", "swaylock", "nixe-assistant", "org.omarchy.agent", "quickshell",
];

/// Refuse a window whose class (or class prefix, case-insensitive) is denied.
pub fn check(window: &hypr::Client, extra: &[String]) -> Result<()> {
    let class = window.class.to_lowercase();
    let denied = DEFAULT_DENY.iter().copied().chain(extra.iter().map(String::as_str)).any(|d| {
        let d = d.to_lowercase();
        !d.is_empty() && (class == d || class.starts_with(&format!("{d}.")) || class.ends_with(&format!(".{d}")) || class.contains(&d) && d.len() >= 6)
    });
    if denied {
        bail!("computer use doesn't touch “{}” ({}) windows", window.title, window.class);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(class: &str) -> hypr::Client {
        serde_json::from_value(serde_json::json!({"address":"0x1","class":class,"title":"t","workspace":{"id":1,"name":"1"}})).unwrap()
    }

    #[test]
    fn denies_sensitive_windows() {
        for c in ["foot", "kitty", "com.mitchellh.ghostty", "1Password", "org.keepassxc.KeePassXC", "Bitwarden", "nixe-assistant-tui", "hyprpolkitagent"] {
            assert!(check(&win(c), &[]).is_err(), "{c}");
        }
        for c in ["firefox", "io.github.design_nexus.nixe.Settings", "obsidian", "Spotify", "code"] {
            assert!(check(&win(c), &[]).is_ok(), "{c}");
        }
        assert!(check(&win("obsidian"), &["obsidian".into()]).is_err());
    }
}
