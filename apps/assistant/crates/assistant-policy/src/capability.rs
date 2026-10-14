//! What a tool can do, from its name. One table in place of the old chain of matches.

use crate::{Decision, Rules};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Weather, location, system and job status.
    Inspect,
    /// Background search, page reading and the visible browser.
    Web,
    /// Hyprland, Omarchy toggles, brightness, wallpaper, screenshots, lock.
    Desktop,
    /// Volume, mute and output devices (`permissions.volume`).
    Audio,
    /// Configured apps and coding-agent handoff (`permissions.launch_app`).
    AppLaunch,
    /// `permissions.shutdown`.
    Shutdown,
    /// Reading, listing and searching files and git status inside HOME (`permissions.files_read`).
    FileRead,
    /// Writing, moving, trashing and restoring files, with backups (`permissions.files_write`).
    FileWrite,
    /// Starting, stopping and restarting systemd user units. Asks unless the unit is listed
    /// in `[system] user_units`.
    Service,
    /// Changing network state (Wi-Fi on/off).
    Network,
    /// Screen and clipboard captures handed to a plugin.
    Capture,
    /// Configured terminal commands and job control.
    Terminal,
    /// Generated argument arrays and shell scripts. Always asks.
    TerminalGenerated,
    /// Abilities' own data: notes, reminders, calendar, routines, projects, media, power.
    PluginData,
    /// Destructive edits, undo, routine runs and resource changes in abilities.
    PluginEdit,
    /// Never runs.
    Disabled,
    /// External MCP and third-party tools nothing else describes.
    External,
}

const PLUGIN_EDITS: &[&str] = &[
    "notes.edit",
    "notes.delete",
    "routines.save",
    "routines.delete",
    "routines.run",
    "projects.save",
    "projects.delete",
    "projects.action",
    "calendar.create",
    "calendar.edit",
    "calendar.delete",
    "file_search.index",
    "file_search.clear",
    "power.battery",
    "power.normal",
    "power.configure",
    "power.unload",
];

const PLUGIN_PREFIXES: &[&str] =
    &["routines.", "projects.", "calendar.", "file_search.", "media.", "power.", "history.", "notes.", "reminders."];

const DESKTOP_PREFIXES: &[&str] = &["hypr.", "toggle.", "display.", "theme.", "windows.", "workspaces.", "monitors."];

const DESKTOP: &[&str] = &[
    "capture.screenshot",
    "capture.screenshot_region",
    "capture.screenshot_window",
    "capture.screenshot_fullscreen",
    "capture.screenrecord",
    "capture.text",
    "system.lock",
    "system.power_profile",
    "bluetooth.toggle",
];

/// Verbs that only look, or stop something, for the Assisted and Manual modes.
/// Stopping is never held for approval.
const PASSIVE_VERBS: &[&str] = &[
    "cancel", "stop", "list", "get", "read", "search", "status", "current", "preview", "snapshot", "tabs", "now", "lookup",
    "find", "show", "query", "scroll", "active", "devices",
];

impl Capability {
    pub fn of(tool: &str) -> Self {
        match tool {
            "shell" => return Self::Disabled,
            "terminal.run" | "terminal.shell" => return Self::TerminalGenerated,
            "terminal.configured" | "terminal.cancel" | "terminal.open" => return Self::Terminal,
            "weather.lookup" | "location.current" | "system.status" | "terminal.status" | "clock.now" => return Self::Inspect,
            "files.read" | "files.list" | "files.search" | "git.status" => return Self::FileRead,
            "files.write" | "files.move" | "files.trash" | "files.restore" => return Self::FileWrite,
            "services.list" | "services.status" | "network.status" => return Self::Inspect,
            "services.control" => return Self::Service,
            "network.wifi" => return Self::Network,
            "app.launch" | "apps.launch" | "agent.handoff" => return Self::AppLaunch,
            "system.shutdown" => return Self::Shutdown,
            "capture.screen" | "capture.clipboard" => return Self::Capture,
            "history.undo" => return Self::PluginData,
            _ => {}
        }
        if tool.ends_with(".undo") || PLUGIN_EDITS.contains(&tool) {
            Self::PluginEdit
        } else if PLUGIN_PREFIXES.iter().any(|p| tool.starts_with(p)) {
            Self::PluginData
        } else if tool.starts_with("audio.") {
            Self::Audio
        } else if tool.starts_with("browser.") || tool.starts_with("web.") {
            Self::Web
        } else if DESKTOP.contains(&tool) || DESKTOP_PREFIXES.iter().any(|p| tool.starts_with(p)) {
            Self::Desktop
        } else {
            Self::External
        }
    }

    /// The user's capability rule, for capabilities that have one.
    pub(crate) fn rule(self, rules: &Rules) -> Option<Decision> {
        match self {
            Self::Audio => Some(rules.volume),
            Self::AppLaunch => Some(rules.launch_app),
            Self::FileRead => Some(rules.files_read),
            Self::FileWrite => Some(rules.files_write),
            Self::Shutdown => Some(rules.shutdown),
            _ => None,
        }
    }

    pub(crate) fn default_decision(self, tool: &str) -> Decision {
        match self {
            Self::Disabled => Decision::Deny,
            Self::Capture
            | Self::PluginEdit
            | Self::TerminalGenerated
            | Self::External
            | Self::FileWrite
            | Self::Service
            | Self::Network => Decision::Ask,
            // Visible-browser form actions are external tools until they are typed.
            Self::Web if !READ_BROWSER.contains(&tool) => Decision::Ask,
            _ => Decision::Allow,
        }
    }

    /// The least a rule can make this capability: generated commands always ask.
    pub(crate) fn floor(self) -> Option<Decision> {
        match self {
            Self::TerminalGenerated => Some(Decision::Ask),
            Self::Disabled => Some(Decision::Deny),
            _ => None,
        }
    }

    /// Whether a spoken "approve" counts. A voice nearby, a video or a web page read aloud
    /// can say it, so generated commands, shutdown and captures need a key press (HUD, TUI
    /// or `nixe-assistant approve`). A spoken "deny" always counts.
    pub fn voice_approvable(self) -> bool {
        !matches!(self, Self::TerminalGenerated | Self::Shutdown | Self::Capture)
    }

    /// Whether one approval may cover the rest of a task. The same tools that need a key
    /// press each time are approved one call at a time.
    pub fn task_approvable(self) -> bool {
        self.voice_approvable() && self != Self::Disabled
    }

    pub(crate) fn low_risk(self) -> bool {
        matches!(self, Self::Desktop | Self::Audio | Self::AppLaunch)
    }

    pub fn passive(self, tool: &str) -> bool {
        match self {
            Self::Inspect | Self::FileRead => true,
            Self::Web => READ_BROWSER.contains(&tool),
            Self::PluginData | Self::Terminal | Self::Desktop | Self::Audio => {
                tool.rsplit('.').next().is_some_and(|verb| PASSIVE_VERBS.iter().any(|r| verb.starts_with(r)))
            }
            _ => false,
        }
    }
}

/// Browser and web tools that only read or move around; the rest ask by default.
const READ_BROWSER: &[&str] = &[
    "browser.search",
    "browser.navigate",
    "browser.remember_sources",
    "browser.open_source",
    "browser.snapshot",
    "browser.tabs",
    "browser.scroll",
    "browser.takeover",
    "browser.resume",
    "web.search",
    "web.read",
];
