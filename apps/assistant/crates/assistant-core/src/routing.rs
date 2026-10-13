//! Validate phrase tables before saving or reloading. No command is dispatched here.
use crate::{AppsFile, CommandsFile};
use std::collections::HashMap;
pub fn phrase_key(phrase: &str) -> String {
    phrase
        .chars()
        .filter(|c| *c != '\'' && *c != '’')
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn validate(apps: &AppsFile, commands: &CommandsFile) -> crate::Result<()> {
    let mut seen = HashMap::new();
    let mut add = |phrases: &[String], owner: String| -> crate::Result<()> {
        if phrases.is_empty() {
            return Err(crate::Error::msg(format!("{owner}: no phrases")));
        }
        for phrase in phrases {
            let key = phrase_key(phrase);
            if key.is_empty() {
                return Err(crate::Error::msg("empty phrase"));
            }
            if let Some(previous) = seen.insert(key.clone(), owner.clone()) {
                return Err(crate::Error::msg(format!(
                    "phrase collision '{key}': {previous} / {owner}"
                )));
            }
            if key.starts_with("use tools to ") {
                return Err(crate::Error::msg("phrase reserved for tool mode"));
            }
        }
        Ok(())
    };
    add(&commands.forget, "forget".into())?;
    for (i, say) in commands.say.iter().enumerate() {
        add(&say.phrases, format!("say {i}"))?;
    }
    for (i, app) in apps.app.iter().enumerate() {
        add(&app.phrases, format!("app {i}"))?;
        if app.exec.is_empty() == app.omarchy.is_empty() {
            return Err(crate::Error::msg(
                "app must have exactly one nonempty argument array: exec or omarchy",
            ));
        }
        if app.args().iter().any(|arg| arg.contains('\0')) {
            return Err(crate::Error::msg("NUL in argument"));
        }
    }
    for (i, command) in commands.command.iter().enumerate() {
        add(&command.phrases, format!("command {i}"))?;
        if command.tool.is_empty()
            || command.omarchy.is_empty()
            || command.omarchy.iter().any(|arg| arg.contains('\0'))
        {
            return Err(crate::Error::msg(
                "command requires tool and nonempty argument array",
            ));
        }
    }
    Ok(())
}
