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
