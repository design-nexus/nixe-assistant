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
    /// Administrative actions run as root by the polkit helper: packages and system
    /// services. Always asks, needs a key press each time, and polkit asks for a password.
    Privileged,
    /// Reading and operating app windows (`ui.*`, `screen.*`, `input.*`). Asks with a key
    /// press; one approval may cover the rest of a task.
    ComputerUse,
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
    &["routines.", "projects.", "calendar.", "file_search.", "media.", "power.", "history.", "notes.", "reminders.", "notepad.", "write.", "music.", "contacts.", "archive.", "chat.", "fonts.", "photos.", "glimpse.",
      "video.", "pdf.", "settings.", "store.", "tasks.", "torrents."];

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
    "find", "show", "query", "scroll", "active", "devices", "installed", "albums", "pages", "top", "tabs",
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
            "services.system_status" | "packages.search" | "packages.info" => return Self::Inspect,
            "services.control" => return Self::Service,
            "network.wifi" => return Self::Network,
            "app.launch" | "apps.launch" | "agent.handoff" => return Self::AppLaunch,
            "system.shutdown" => return Self::Shutdown,
            "capture.screen" | "capture.clipboard" => return Self::Capture,
            "history.undo" => return Self::PluginData,
            // Nixe apps worked directly: reading a file in, typing into a document, saving.
            "notepad.open" | "write.open" | "archive.list" | "archive.open" | "photos.open" | "pdf.open" => return Self::FileRead,
            "notepad.insert_text" | "write.insert_text" | "contacts.add" | "contacts.update" | "chat.send" => return Self::PluginEdit,
            "notepad.save" | "write.save" | "write.export_pdf" | "archive.extract" | "archive.create" => return Self::FileWrite,
            // Reach the network with what they're given.
            "fonts.install" | "torrents.add" => return Self::External,
            _ => {}
        }
        if tool.starts_with("admin.") {
            Self::Privileged
        } else if ["ui.", "screen.", "input."].iter().any(|p| tool.starts_with(p)) {
            Self::ComputerUse
        } else if tool.ends_with(".undo") || PLUGIN_EDITS.contains(&tool) {
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
            Self::Privileged => Some(rules.admin),
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
            | Self::Privileged
            | Self::ComputerUse
            | Self::Network => Decision::Ask,
            // Visible-browser form actions are external tools until they are typed.
            Self::Web if !READ_BROWSER.contains(&tool) => Decision::Ask,
            _ => Decision::Allow,
        }
    }

    /// The least a rule can make this capability: generated commands and admin actions
    /// always ask.
    pub(crate) fn floor(self) -> Option<Decision> {
        match self {
            Self::TerminalGenerated | Self::Privileged | Self::ComputerUse => Some(Decision::Ask),
            Self::Disabled => Some(Decision::Deny),
            _ => None,
        }
    }

    /// Whether a spoken "approve" counts. A voice nearby, a video or a web page read aloud
    /// can say it, so generated commands, shutdown, captures and admin actions need a key
    /// press (HUD, TUI or `nixe-assistant approve`). A spoken "deny" always counts.
    pub fn voice_approvable(self) -> bool {
        !matches!(self, Self::TerminalGenerated | Self::Shutdown | Self::Capture | Self::Privileged | Self::ComputerUse)
    }

    /// Whether one approval may cover the rest of a task. The same tools that need a key
    /// press each time are approved one call at a time.
    pub fn task_approvable(self) -> bool {
        (self.voice_approvable() || self == Self::ComputerUse) && self != Self::Disabled
    }

    pub(crate) fn low_risk(self) -> bool {
        matches!(self, Self::Desktop | Self::Audio | Self::AppLaunch)
    }

    /// The call's result carries content someone else wrote (pages, documents, notes, command
    /// output, other apps' windows), which may hold instructions.
    pub fn brings_in_untrusted(tool: &str) -> bool {
        let c = Self::of(tool);
        match c {
            Self::Web | Self::External | Self::FileRead => true,
            Self::PluginData | Self::ComputerUse | Self::Terminal => c.passive(tool) || COMPUTER_READS.contains(&tool),
            _ => false,
        }
    }

    /// The call's result carries the user's own data: files, notes, captures, what's on screen,
    /// command output.
    pub fn reads_private(tool: &str) -> bool {
        let c = Self::of(tool);
        match c {
            Self::FileRead | Self::Capture => true,
            Self::PluginData | Self::ComputerUse | Self::Terminal => c.passive(tool) || COMPUTER_READS.contains(&tool),
            _ => false,
        }
    }

    /// The call sends something it was given to another machine: a URL, a search query, a
    /// form field, or anything an external tool does with its arguments.
    pub fn sends_out(tool: &str, args: &serde_json::Value) -> bool {
        let opens_url = tool == "browser.tabs" && args["url"].as_str().is_some_and(|u| !u.trim().is_empty());
        opens_url
            || matches!(Self::of(tool), Self::External)
            || matches!(
                tool,
                "browser.navigate"
                    | "browser.search"
                    | "browser.open_source"
                    | "browser.fill"
                    | "browser.click"
                    | "web.search"
                    | "web.read"
            )
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

/// Computer-use tools that only read a window.
const COMPUTER_READS: &[&str] = &["ui.tree", "ui.find", "screen.marks", "screen.look"];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nixe_app_tools() {
        assert_eq!(Capability::of("notepad.read"), Capability::PluginData);
        assert_eq!(Capability::of("write.new_document"), Capability::PluginData);
        assert_eq!(Capability::of("notepad.open"), Capability::FileRead);
        assert_eq!(Capability::of("write.insert_text"), Capability::PluginEdit);
        assert_eq!(Capability::of("notepad.save"), Capability::FileWrite);
        assert_eq!(Capability::of("write.export_pdf"), Capability::FileWrite);
        assert_eq!(Capability::FileWrite.default_decision("write.save"), Decision::Ask);
        assert_eq!(Capability::PluginData.default_decision("notepad.tabs"), Decision::Allow);
        assert_eq!(Capability::of("contacts.add"), Capability::PluginEdit);
        assert_eq!(Capability::of("music.play"), Capability::PluginData);
        assert!(Capability::reads_private("contacts.find"));
        assert_eq!(Capability::of("archive.extract"), Capability::FileWrite);
        assert_eq!(Capability::of("chat.send"), Capability::PluginEdit);
        assert_eq!(Capability::of("torrents.add"), Capability::External);
        assert_eq!(Capability::of("tasks.top"), Capability::PluginData);
        assert!(Capability::reads_private("pdf.read") && Capability::reads_private("photos.search"));
    }
}
