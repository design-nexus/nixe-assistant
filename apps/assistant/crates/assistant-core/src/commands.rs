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
    /// No longer used: workspace phrases run the typed `focus_workspace` and
    /// `move_to_workspace` actions. Kept so older files still load.
    #[serde(default, skip_serializing)]
    pub focus: String,
    #[serde(default, skip_serializing, rename = "move")]
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
            focus: String::new(),
            r#move: String::new(),
        }
    }
}

fn default_can_do() -> String {
    "I didn't catch a command I can run. I can lock, close a window, change volume or brightness, switch workspaces 1 to 5, cycle wallpaper, toggle the bar, gaps, or night light, or take a screenshot".into()
}

fn default_reply() -> String {
    "Okay.".into()
}

/// The typed action for a Hyprland Lua line the shipped file used to carry, so files
/// written before typed actions keep working. Anything else stays as written, and the
/// worker refuses to run `hyprctl eval`.
pub fn typed_action(lua: &str) -> Option<Vec<String>> {
    let action: &[&str] = match lua.trim() {
        "hl.dispatch(hl.dsp.window.close())" => &["close_window"],
        r#"hl.dispatch(hl.dsp.focus({ workspace = "e+1" }))"# => &["focus_workspace", "e+1"],
        r#"hl.dispatch(hl.dsp.focus({ workspace = "e-1" }))"# => &["focus_workspace", "e-1"],
        r#"hl.dispatch(hl.dsp.focus({ workspace = "previous" }))"# => &["focus_workspace", "previous"],
        r#"hl.dispatch(hl.dsp.workspace.toggle_special("scratchpad"))"# => &["toggle_special", "scratchpad"],
        r#"hl.dispatch(hl.dsp.window.move({ workspace = "special:scratchpad", follow = false }))"# => &["move_to_workspace", "special:scratchpad", "stay"],
        r#"hl.dispatch(hl.dsp.window.float({ action = "toggle" }))"# => &["float_toggle"],
        r#"hl.dispatch(hl.dsp.layout("togglesplit"))"# => &["toggle_split"],
        r#"hl.dispatch(hl.dsp.focus({ direction = "l" }))"# => &["focus_direction", "l"],
        r#"hl.dispatch(hl.dsp.focus({ direction = "r" }))"# => &["focus_direction", "r"],
        r#"hl.dispatch(hl.dsp.focus({ direction = "u" }))"# => &["focus_direction", "u"],
        r#"hl.dispatch(hl.dsp.focus({ direction = "d" }))"# => &["focus_direction", "d"],
        "hl.dispatch(hl.dsp.window.cycle_next())" => &["cycle_next"],
        "hl.dispatch(hl.dsp.window.cycle_next({ next = false }))" => &["cycle_previous"],
        _ => return None,
    };
    Some(std::iter::once("hypr").chain(action.iter().copied()).map(String::from).collect())
}

impl CommandsFile {
    pub fn shipped() -> Self {
        toml::from_str::<Self>(SHIPPED).expect("shipped commands.toml").typed()
    }

    /// Known `hyprctl eval` lines become typed actions.
    fn typed(mut self) -> Self {
        for c in &mut self.command {
            if let [hyprctl, eval, lua] = c.omarchy.as_slice() {
                if hyprctl == "hyprctl" && eval == "eval" {
                    if let Some(typed) = typed_action(lua) {
                        c.omarchy = typed;
                    }
                }
            }
        }
        self
    }

    pub fn load() -> crate::Result<Self> {
        Self::load_from(&paths::commands_path())
    }

    pub fn load_from(path: &Path) -> crate::Result<Self> {
        if path.exists() {
            let text = fs::read_to_string(path)?;
            Ok(toml::from_str::<Self>(&text)?.typed())
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

    #[test]
    fn nothing_shipped_runs_hyprctl_eval() {
        for c in CommandsFile::shipped().command {
            assert!(!(c.omarchy.first().is_some_and(|p| p == "hyprctl") && c.omarchy.get(1).is_some_and(|a| a == "eval")), "{}: {:?}", c.tool, c.omarchy);
        }
    }

    #[test]
    fn older_files_get_typed_actions() {
        let old = r#"
[[command]]
phrases = ["close window"]
tool = "hypr.window_close"
omarchy = ["hyprctl", "eval", "hl.dispatch(hl.dsp.window.close())"]

[[command]]
phrases = ["mine"]
tool = "hypr.mine"
omarchy = ["hyprctl", "eval", "hl.dispatch(hl.dsp.exec_cmd('foot'))"]

[workspace]
focus = 'hl.dispatch(hl.dsp.focus({ workspace = "{n}" }))'
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.toml");
        std::fs::write(&path, old).unwrap();
        let cmds = CommandsFile::load_from(&path).unwrap();
        assert_eq!(cmds.command[0].omarchy, ["hypr", "close_window"]);
        assert_eq!(cmds.command[1].omarchy[1], "eval", "unknown Lua is left as written (and refused when run)");
    }
}
