use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const SHIPPED: &str = include_str!("../../../config/commands.toml");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsFile {
    #[serde(default = "default_can_do")]
    pub can_do: String,
    #[serde(default)]
    pub forget: Vec<String>,
    #[serde(default)]
    pub starts: Vec<String>,
    #[serde(default)]
    pub vocab: Vec<String>,
    #[serde(default)]
    pub workspace: WorkspaceCommands,
    #[serde(default)]
    pub say: Vec<SayPhrase>,
    #[serde(default)]
    pub command: Vec<CommandPhrase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SayPhrase {
    pub phrases: Vec<String>,
    pub reply: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandPhrase {
    pub phrases: Vec<String>,
    pub tool: String,
    #[serde(default = "default_reply")]
    pub reply: String,
    #[serde(default)]
    pub omarchy: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceCommands {
    pub max: u8,
    pub tool: String,
    pub move_tool: String,
    pub spoken: String,
    pub move_spoken: String,
    pub focus: String,
    pub r#move: String,
}

impl Default for WorkspaceCommands {
    fn default() -> Self {
        Self {
            max: 5,
            tool: "hypr.workspace".into(),
            move_tool: "hypr.workspace_move".into(),
            spoken: "Workspace {n}.".into(),
            move_spoken: "Moving to workspace {n}.".into(),
            focus: r#"hl.dispatch(hl.dsp.focus({ workspace = "{n}" }))"#.into(),
            r#move: r#"hl.dispatch(hl.dsp.window.move({ workspace = "{n}" }))"#.into(),
        }
    }
}

fn default_can_do() -> String {
    "I didn't catch a command I can run. I can lock, close a window, change volume or brightness, switch workspaces 1 to 5, cycle wallpaper, toggle the bar, gaps, or night light, or take a screenshot".into()
}

fn default_reply() -> String {
    "Okay.".into()
}

impl CommandsFile {
    pub fn shipped() -> Self {
        toml::from_str(SHIPPED).expect("shipped commands.toml")
    }

    pub fn load() -> crate::Result<Self> {
        Self::load_from(&paths::commands_path())
    }

    pub fn load_from(path: &Path) -> crate::Result<Self> {
        if path.exists() {
            let text = fs::read_to_string(path)?;
            Ok(toml::from_str(&text)?)
        } else {
            Ok(Self::shipped())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_commands_parse() {
        let cmds = CommandsFile::shipped();
        assert!(cmds.command.iter().any(|c| c.tool == "system.lock"));
        assert!(cmds.forget.iter().any(|p| p == "forget everything"));
        assert_eq!(cmds.workspace.max, 5);
        assert!(cmds.say.len() >= 2);
    }
}
