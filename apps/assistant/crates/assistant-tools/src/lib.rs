pub mod agent;
pub mod jobs;
pub mod mcp;
pub mod typed;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    Safe,
    Confirm,
    Dangerous,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub permission: Permission,
    pub description: String,
}

pub fn builtin_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "agent.handoff".into(),
            permission: Permission::Safe,
            description: "Hand off a task to an interactive terminal coding agent".into(),
        },
        ToolSpec {
            name: "files.read".into(),
            permission: Permission::Safe,
            description: "Read the contents of a local file safely".into(),
        },
        ToolSpec {
            name: "git.status".into(),
            permission: Permission::Safe,
            description: "Check git repository status".into(),
        },
        ToolSpec {
            name: "system.status".into(),
            permission: Permission::Safe,
            description: "Inspect system hardware and resource status".into(),
        },
        ToolSpec {
            name: "audio.volume_down".into(),
            permission: Permission::Safe,
            description: "Lower output volume".into(),
        },
        ToolSpec {
            name: "audio.volume_up".into(),
            permission: Permission::Safe,
            description: "Raise output volume".into(),
        },
        ToolSpec {
            name: "audio.mute".into(),
            permission: Permission::Safe,
            description: "Toggle output mute".into(),
        },
        ToolSpec {
            name: "app.launch".into(),
            permission: Permission::Safe,
            description: "Launch an application".into(),
        },
        ToolSpec {
            name: "capture.screenshot".into(),
            permission: Permission::Safe,
            description: "Take a screenshot".into(),
        },
        ToolSpec {
            name: "toggle.nightlight".into(),
            permission: Permission::Safe,
            description: "Toggle night light".into(),
        },
        ToolSpec {
            name: "system.lock".into(),
            permission: Permission::Safe,
            description: "Lock the session".into(),
        },
        ToolSpec {
            name: "display.brightness_up".into(),
            permission: Permission::Safe,
            description: "Raise display brightness".into(),
        },
        ToolSpec {
            name: "display.brightness_down".into(),
            permission: Permission::Safe,
            description: "Lower display brightness".into(),
        },
        ToolSpec {
            name: "theme.bg_next".into(),
            permission: Permission::Safe,
            description: "Cycle wallpaper".into(),
        },
        ToolSpec {
            name: "hypr.workspace".into(),
            permission: Permission::Safe,
            description: "Switch Hyprland workspace".into(),
        },
        ToolSpec {
            name: "hypr.workspace_move".into(),
            permission: Permission::Safe,
            description: "Move the focused window to a workspace".into(),
        },
        ToolSpec {
            name: "audio.mic_mute".into(),
            permission: Permission::Safe,
            description: "Toggle microphone mute".into(),
        },
        ToolSpec {
            name: "bluetooth.toggle".into(),
            permission: Permission::Safe,
            description: "Toggle Bluetooth power".into(),
        },
        ToolSpec {
            name: "toggle.dnd".into(),
            permission: Permission::Safe,
            description: "Toggle do-not-disturb".into(),
        },
        ToolSpec {
            name: "toggle.idle".into(),
            permission: Permission::Safe,
            description: "Stay awake / block idle".into(),
        },
        ToolSpec {
            name: "toggle.bar".into(),
            permission: Permission::Safe,
            description: "Toggle the Omarchy bar".into(),
        },
        ToolSpec {
            name: "hypr.gaps".into(),
            permission: Permission::Safe,
            description: "Toggle window gaps".into(),
        },
        ToolSpec {
            name: "toggle.animations".into(),
            permission: Permission::Safe,
            description: "Toggle animations and effects".into(),
        },
        ToolSpec {
            name: "hypr.transparency".into(),
            permission: Permission::Safe,
            description: "Toggle window transparency".into(),
        },
        ToolSpec {
            name: "hypr.fullscreen".into(),
            permission: Permission::Safe,
            description: "Toggle tiled fullscreen".into(),
        },
        ToolSpec {
            name: "hypr.layout".into(),
            permission: Permission::Safe,
            description: "Toggle dwindle/master layout".into(),
        },
        ToolSpec {
            name: "capture.screenshot_region".into(),
            permission: Permission::Safe,
            description: "Screenshot a selected region".into(),
        },
        ToolSpec {
            name: "capture.screenshot_window".into(),
            permission: Permission::Safe,
            description: "Screenshot a window".into(),
        },
        ToolSpec {
            name: "capture.screenshot_fullscreen".into(),
            permission: Permission::Safe,
            description: "Screenshot the whole screen".into(),
        },
        ToolSpec {
            name: "capture.text".into(),
            permission: Permission::Safe,
            description: "OCR text from a screen region".into(),
        },
        ToolSpec {
            name: "hypr.window_close".into(),
            permission: Permission::Safe,
            description: "Close the focused window".into(),
        },
        ToolSpec {
            name: "hypr.scratchpad".into(),
            permission: Permission::Safe,
            description: "Toggle the scratchpad".into(),
        },
        ToolSpec {
            name: "hypr.scratchpad_move".into(),
            permission: Permission::Safe,
            description: "Move the focused window to the scratchpad".into(),
        },
        ToolSpec {
            name: "hypr.float".into(),
            permission: Permission::Safe,
            description: "Toggle window floating".into(),
        },
        ToolSpec {
            name: "hypr.split".into(),
            permission: Permission::Safe,
            description: "Toggle the window split".into(),
        },
        ToolSpec {
            name: "hypr.focus".into(),
            permission: Permission::Safe,
            description: "Focus a window in a direction".into(),
        },
        ToolSpec {
            name: "hypr.window_cycle".into(),
            permission: Permission::Safe,
            description: "Cycle focused window".into(),
        },
        ToolSpec {
            name: "hypr.window_pop".into(),
            permission: Permission::Safe,
            description: "Pop the window out (float and pin)".into(),
        },
        ToolSpec {
            name: "capture.screenrecord".into(),
            permission: Permission::Safe,
            description: "Start or stop screen recording".into(),
        },
        ToolSpec {
            name: "toggle.touchpad".into(),
            permission: Permission::Safe,
            description: "Toggle the touchpad".into(),
        },
        ToolSpec {
            name: "toggle.screensaver".into(),
            permission: Permission::Safe,
            description: "Toggle screensaver availability".into(),
        },
        ToolSpec {
            name: "toggle.fullscreen_desktop".into(),
            permission: Permission::Safe,
            description: "Toggle fullscreen desktop (bar + gaps)".into(),
        },
        ToolSpec {
            name: "audio.output_switch".into(),
            permission: Permission::Safe,
            description: "Switch audio output device".into(),
        },
        ToolSpec {
            name: "display.keyboard_up".into(),
            permission: Permission::Safe,
            description: "Raise keyboard backlight".into(),
        },
        ToolSpec {
            name: "display.keyboard_down".into(),
            permission: Permission::Safe,
            description: "Lower keyboard backlight".into(),
        },
        ToolSpec {
            name: "theme.switcher".into(),
            permission: Permission::Safe,
            description: "Open the theme switcher".into(),
        },
        ToolSpec {
            name: "system.shutdown".into(),
            permission: Permission::Confirm,
            description: "Shut down the machine".into(),
        },
        ToolSpec {
            name: "shell".into(),
            permission: Permission::Disabled,
            description: "Arbitrary shell (disabled)".into(),
        },
    ]
}

/// Gate a routed tool against config permission strings.
/// Confirm tools have no UI yet, so only `system.shutdown` with `allow` passes.
pub fn tool_allowed(tool: &str, volume: &str, launch_app: &str, shutdown: &str) -> bool {
    let Some(spec) = builtin_tools().into_iter().find(|t| t.name == tool) else {
        return false;
    };
    match spec.permission {
        Permission::Disabled | Permission::Dangerous => false,
        Permission::Confirm => tool == "system.shutdown" && shutdown == "allow",
        Permission::Safe => match tool {
            "audio.volume_down" | "audio.volume_up" | "audio.mute" | "audio.output_switch" => {
                volume == "allow"
            }
            "app.launch" => launch_app == "allow",
            "audio.mic_mute" => false,
            _ => true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_respects_config() {
        assert!(tool_allowed("audio.mute", "allow", "allow", "confirm"));
        assert!(!tool_allowed("audio.mute", "deny", "allow", "confirm"));
    }

    #[test]
    fn shell_never() {
        assert!(!tool_allowed("shell", "allow", "allow", "allow"));
    }

    #[test]
    fn shutdown_needs_allow() {
        assert!(!tool_allowed(
            "system.shutdown",
            "allow",
            "allow",
            "confirm"
        ));
        assert!(tool_allowed("system.shutdown", "allow", "allow", "allow"));
    }

    #[test]
    fn unknown_denied() {
        assert!(!tool_allowed("nope.tool", "allow", "allow", "allow"));
    }

    #[test]
    fn lock_is_safe() {
        assert!(tool_allowed("system.lock", "allow", "allow", "confirm"));
    }
}

pub mod registry;

pub mod plugins;
