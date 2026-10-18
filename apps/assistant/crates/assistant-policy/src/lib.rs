//! Permission decisions for assistant tools.
//!
//! The decision point (`decide`) turns a tool name and the user's rules into a `Verdict`.
//! Only a verdict can mint a `Grant`, and the registry's enforcement point dispatches only
//! with a grant bound to the exact tool and arguments. The audit journal (`store`) records
//! decisions, approvals and outcomes with sensitive values redacted.

pub mod capability;
pub mod grant;
pub mod store;
pub mod summary;
pub mod task;

pub use capability::Capability;
pub use grant::{digest, Grant, GrantKind};
use serde::{Deserialize, Serialize};
pub use task::{resource_of, Budget, Task, TaskState, Verification};

/// Ordered from least to most restrictive, so `max` picks the stricter one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    /// Reserved for sandboxed execution (bubblewrap). Until that exists it asks.
    Sandbox,
    Ask,
    Deny,
}

impl Decision {
    /// Config spelling: `allow`, `confirm` (or `ask`), `deny`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "allow" => Some(Self::Allow),
            "confirm" | "ask" => Some(Self::Ask),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    /// The spelling the IPC, TUI and HUD already use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Sandbox | Self::Ask => "confirm",
            Self::Deny => "deny",
        }
    }

    pub fn needs_approval(self) -> bool {
        matches!(self, Self::Sandbox | Self::Ask)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutonomyMode {
    /// Permitted actions run without asking; only rules marked confirm ask.
    #[default]
    Autonomous,
    /// Reads and low-risk desktop changes run; other changes ask.
    Assisted,
    /// Anything that changes state asks.
    Manual,
}

/// The user's capability rules (`[permissions]` in config.toml), already parsed.
#[derive(Debug, Clone, Copy)]
pub struct Rules {
    pub launch_app: Decision,
    pub volume: Decision,
    pub files_read: Decision,
    pub files_write: Decision,
    pub shutdown: Decision,
    /// `admin.*`: only ask or deny; an allow still asks.
    pub admin: Decision,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            launch_app: Decision::Allow,
            volume: Decision::Allow,
            files_read: Decision::Allow,
            files_write: Decision::Ask,
            shutdown: Decision::Ask,
            admin: Decision::Ask,
        }
    }
}

/// What the host knows about the tool when it asks for a decision.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// The tool's plugin is installed, enabled and healthy.
    pub available: bool,
    /// The tool's plugin is first-party or explicitly trusted.
    pub trusted: bool,
    /// A per-tool rule (`[mcp.permissions]`), if the user set one.
    pub tool_rule: Option<Decision>,
    pub rules: &'a Rules,
    pub mode: AutonomyMode,
    /// The call's resource is one the user pre-approved in config (a service unit listed in
    /// `[system] user_units`). Lets an asking capability run without asking; a deny still wins.
    pub resource_approved: bool,
    /// The call writes somewhere that runs as the user later, outside any sandbox (shell
    /// startup files, `~/.local/bin`, autostart, Hyprland config, git hooks). It always asks,
    /// by key press, one call at a time.
    pub runs_code: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    tool: String,
    capability: Capability,
    decision: Decision,
    rule: &'static str,
    /// Approved by key press only, and never for the rest of a task.
    key_press: bool,
    /// Asked about on its own, never covered by a task-wide approval.
    single: bool,
}

impl Verdict {
    pub fn tool(&self) -> &str {
        &self.tool
    }
    pub fn capability(&self) -> Capability {
        self.capability
    }
    pub fn decision(&self) -> Decision {
        self.decision
    }
    /// Which step decided, for Diagnostics and the audit journal.
    pub fn rule(&self) -> &'static str {
        self.rule
    }
    /// Whether only a key press approves it (HUD, Settings, TUI or `nixe-assistant approve`):
    /// a spoken "approve" could come from a video, a page read aloud or someone nearby.
    pub fn key_press(&self) -> bool {
        self.key_press || !self.capability.voice_approvable()
    }
    /// Whether one approval may cover the rest of the task.
    pub fn task_approvable(&self) -> bool {
        !self.single && !self.key_press && self.capability.task_approvable()
    }
    /// For a call that could carry private data out of the machine after the task read
    /// untrusted content: an allowed call asks, by itself, and only a key press approves it
    /// (the untrusted content could be read aloud saying "approve"). A deny stays a deny.
    pub fn ask_before_sending(mut self) -> Self {
        if self.decision < Decision::Ask {
            self.decision = Decision::Ask;
            self.rule = "may send private data";
        }
        self.single = true;
        self.key_press = true;
        self
    }
}

/// The decision point. Order: unavailable or disabled tools are denied; an explicit deny
/// (per tool or per capability) wins; then the per-tool rule, untrusted plugins asking,
/// the capability rule, the capability default; then the autonomy mode may only tighten,
/// and mandatory floors cap what any rule can allow.
pub fn decide(tool: &str, cx: &Context) -> Verdict {
    let capability = Capability::of(tool);
    let verdict =
        |decision, rule| Verdict { tool: tool.into(), capability, decision, rule, key_press: cx.runs_code, single: cx.runs_code };
    if !cx.available {
        return verdict(Decision::Deny, "unavailable");
    }
    if capability == Capability::Disabled {
        return verdict(Decision::Deny, "disabled");
    }
    let capability_rule = capability.rule(cx.rules);
    if cx.tool_rule == Some(Decision::Deny) {
        return verdict(Decision::Deny, "tool rule");
    }
    if capability_rule == Some(Decision::Deny) {
        return verdict(Decision::Deny, "capability rule");
    }
    let (mut decision, mut rule) = if let Some(d) = cx.tool_rule {
        (d, "tool rule")
    } else if !cx.trusted {
        (Decision::Ask, "untrusted plugin")
    } else if let Some(d) = capability_rule {
        (d, "capability rule")
    } else if cx.resource_approved {
        (Decision::Allow, "approved resource")
    } else {
        (capability.default_decision(tool), "default")
    };
    let mode_floor = match cx.mode {
        AutonomyMode::Autonomous => Decision::Allow,
        AutonomyMode::Assisted if capability.passive(tool) || capability.low_risk() => Decision::Allow,
        AutonomyMode::Manual if capability.passive(tool) => Decision::Allow,
        _ => Decision::Ask,
    };
    if mode_floor > decision {
        (decision, rule) = (mode_floor, "autonomy mode");
    }
    if let Some(floor) = capability.floor() {
        if floor > decision {
            (decision, rule) = (floor, "mandatory");
        }
    }
    if cx.runs_code && decision < Decision::Ask {
        (decision, rule) = (Decision::Ask, "runs code");
    }
    verdict(decision, rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(rules: &Rules) -> Context<'_> {
        Context {
            available: true,
            trusted: true,
            tool_rule: None,
            rules,
            mode: AutonomyMode::Autonomous,
            resource_approved: false,
            runs_code: false,
        }
    }

    #[test]
    fn parse_keeps_legacy_spelling() {
        assert_eq!(Decision::parse("confirm"), Some(Decision::Ask));
        assert_eq!(Decision::parse("ask"), Some(Decision::Ask));
        assert_eq!(Decision::parse("alow"), None);
        assert_eq!(Decision::Ask.as_str(), "confirm");
    }

    #[test]
    fn defaults_match_the_previous_table() {
        let rules = Rules::default();
        let d = |t| decide(t, &cx(&rules)).decision();
        for t in [
            "hypr.workspace",
            "notes.add",
            "history.undo",
            "browser.search",
            "files.read",
            "apps.launch",
            "audio.mute",
            "terminal.configured",
        ] {
            assert_eq!(d(t), Decision::Allow, "{t}");
        }
        for t in [
            "notes.undo",
            "capture.screen",
            "routines.run",
            "calendar.create",
            "system.shutdown",
            "terminal.run",
            "terminal.shell",
            "ext.unknown",
        ] {
            assert_eq!(d(t), Decision::Ask, "{t}");
        }
        assert_eq!(d("shell"), Decision::Deny);
    }

    #[test]
    fn deny_overrides_allow() {
        let rules = Rules { volume: Decision::Deny, ..Rules::default() };
        let mut c = cx(&rules);
        c.tool_rule = Some(Decision::Allow);
        let v = decide("audio.mute", &c);
        assert_eq!((v.decision(), v.rule()), (Decision::Deny, "capability rule"));
        let rules = Rules::default();
        let mut c = cx(&rules);
        c.tool_rule = Some(Decision::Deny);
        assert_eq!(decide("hypr.workspace", &c).decision(), Decision::Deny);
    }

    #[test]
    fn capability_rules_reach_file_and_handoff_tools() {
        let rules = Rules { files_read: Decision::Deny, launch_app: Decision::Ask, ..Rules::default() };
        assert_eq!(decide("files.read", &cx(&rules)).decision(), Decision::Deny);
        assert_eq!(decide("git.status", &cx(&rules)).decision(), Decision::Deny);
        assert_eq!(decide("agent.handoff", &cx(&rules)).decision(), Decision::Ask);
    }

    #[test]
    fn floor_holds_against_user_allow() {
        let rules = Rules::default();
        let mut c = cx(&rules);
        c.tool_rule = Some(Decision::Allow);
        let v = decide("terminal.shell", &c);
        assert_eq!((v.decision(), v.rule()), (Decision::Ask, "mandatory"));
        assert_eq!(decide("terminal.run", &c).decision(), Decision::Ask);
        assert_eq!(decide("shell", &c).decision(), Decision::Deny);
        // Shutdown may be allowed explicitly, as before.
        assert_eq!(decide("system.shutdown", &c).decision(), Decision::Allow);
    }

    #[test]
    fn phase3_capabilities() {
        let rules = Rules::default();
        let d = |t| decide(t, &cx(&rules)).decision();
        for t in [
            "windows.list",
            "windows.focus",
            "workspaces.switch",
            "monitors.list",
            "audio.devices",
            "files.list",
            "files.search",
            "services.status",
            "network.status",
            "system.power_profile",
        ] {
            assert_eq!(d(t), Decision::Allow, "{t}");
        }
        for t in ["files.write", "files.move", "files.trash", "files.restore", "services.control", "network.wifi"] {
            assert_eq!(d(t), Decision::Ask, "{t}");
        }
        // files_write governs the write tools.
        let rules = Rules { files_write: Decision::Allow, ..Rules::default() };
        assert_eq!(decide("files.write", &cx(&rules)).decision(), Decision::Allow);
        // Read-only desktop tools stay passive in Manual mode; changes ask.
        let rules = Rules::default();
        let mut c = cx(&rules);
        c.mode = AutonomyMode::Manual;
        assert_eq!(decide("windows.list", &c).decision(), Decision::Allow);
        assert_eq!(decide("windows.active", &c).decision(), Decision::Allow);
        assert_eq!(decide("windows.focus", &c).decision(), Decision::Ask);
    }

    #[test]
    fn approved_resources() {
        let rules = Rules::default();
        let mut c = cx(&rules);
        c.resource_approved = true;
        let v = decide("services.control", &c);
        assert_eq!((v.decision(), v.rule()), (Decision::Allow, "approved resource"));
        c.tool_rule = Some(Decision::Deny);
        assert_eq!(decide("services.control", &c).decision(), Decision::Deny, "deny still wins");
        c.tool_rule = None;
        c.mode = AutonomyMode::Manual;
        assert_eq!(decide("services.control", &c).decision(), Decision::Ask, "modes still tighten");
    }

    #[test]
    fn high_impact_needs_a_key_press() {
        for t in ["terminal.shell", "terminal.run", "system.shutdown", "capture.screen", "capture.clipboard"] {
            assert!(!Capability::of(t).voice_approvable(), "{t}");
        }
        for t in ["notes.delete", "calendar.create", "routines.run", "chat.send"] {
            assert!(Capability::of(t).voice_approvable(), "{t}");
        }
        // External tools send their arguments away: a key press, once per task at most.
        for t in ["ext.unknown", "fonts.install", "torrents.add"] {
            assert!(!Capability::of(t).voice_approvable() && Capability::of(t).task_approvable(), "{t}");
        }
    }

    #[test]
    fn admin_always_asks_by_key_press() {
        let mut rules = Rules { admin: Decision::Allow, ..Rules::default() };
        let mut c = cx(&rules);
        c.resource_approved = true;
        c.tool_rule = Some(Decision::Allow);
        for t in ["admin.install", "admin.remove", "admin.update", "admin.service"] {
            let v = decide(t, &c);
            assert_eq!((v.capability(), v.decision(), v.rule()), (Capability::Privileged, Decision::Ask, "mandatory"), "{t}");
            assert!(!v.capability().voice_approvable() && !v.capability().task_approvable(), "{t}");
        }
        rules.admin = Decision::Deny;
        let c = cx(&rules);
        assert_eq!(decide("admin.install", &c).decision(), Decision::Deny);
        for t in ["packages.search", "packages.info", "services.system_status"] {
            assert_eq!(decide(t, &c).decision(), Decision::Allow, "{t} only looks");
        }
    }

    #[test]
    fn computer_use_asks_once_per_task_by_key_press() {
        let rules = Rules::default();
        let mut c = cx(&rules);
        c.tool_rule = Some(Decision::Allow);
        for t in ["ui.tree", "ui.act", "screen.marks", "screen.look", "input.click", "input.type", "input.key", "input.scroll"] {
            let v = decide(t, &c);
            assert_eq!((v.capability(), v.decision()), (Capability::ComputerUse, Decision::Ask), "{t}");
            assert!(!v.capability().voice_approvable(), "{t}: a spoken approve doesn't count");
            assert!(v.capability().task_approvable(), "{t}: one approval covers the task");
        }
        c.tool_rule = Some(Decision::Deny);
        assert_eq!(decide("input.click", &c).decision(), Decision::Deny);
    }

    #[test]
    fn untrusted_and_unavailable() {
        let rules = Rules::default();
        let mut c = cx(&rules);
        c.trusted = false;
        assert_eq!(decide("notes.add", &c).decision(), Decision::Ask);
        c.tool_rule = Some(Decision::Allow);
        assert_eq!(decide("notes.add", &c).decision(), Decision::Allow);
        c.available = false;
        assert_eq!(decide("notes.add", &c).decision(), Decision::Deny);
    }

    #[test]
    fn modes_only_tighten_state_changes() {
        let rules = Rules::default();
        let mut c = cx(&rules);
        c.mode = AutonomyMode::Assisted;
        let d = |c: &Context, t| decide(t, c).decision();
        assert_eq!(d(&c, "files.read"), Decision::Allow);
        assert_eq!(d(&c, "notes.list"), Decision::Allow);
        assert_eq!(d(&c, "hypr.workspace"), Decision::Allow);
        assert_eq!(d(&c, "notes.add"), Decision::Ask);
        assert_eq!(d(&c, "apps.launch"), Decision::Allow);
        c.mode = AutonomyMode::Manual;
        assert_eq!(d(&c, "files.read"), Decision::Allow);
        assert_eq!(d(&c, "weather.lookup"), Decision::Allow);
        assert_eq!(d(&c, "hypr.workspace"), Decision::Ask);
        assert_eq!(d(&c, "apps.launch"), Decision::Ask);
        let v = decide("notes.add", &c);
        assert_eq!(v.rule(), "autonomy mode");
        // A mode never loosens a deny.
        let rules = Rules { files_read: Decision::Deny, ..Rules::default() };
        let mut c = cx(&rules);
        c.mode = AutonomyMode::Manual;
        assert_eq!(d(&c, "files.read"), Decision::Deny);
    }

    #[test]
    fn writes_that_run_code_ask_by_key_press_each_time() {
        let rules = Rules { files_write: Decision::Allow, ..Rules::default() };
        let mut c = cx(&rules);
        c.tool_rule = Some(Decision::Allow);
        c.runs_code = true;
        let v = decide("files.write", &c);
        assert_eq!((v.decision(), v.rule()), (Decision::Ask, "runs code"));
        assert!(v.key_press() && !v.task_approvable());
        c.tool_rule = Some(Decision::Deny);
        assert_eq!(decide("files.write", &c).decision(), Decision::Deny, "deny still wins");
        c.tool_rule = None;
        c.runs_code = false;
        let v = decide("files.write", &c);
        assert_eq!(v.decision(), Decision::Allow);
        assert!(!v.key_press(), "an ordinary write takes a spoken approve");
    }

    #[test]
    fn sending_after_untrusted_reads_asks_on_its_own() {
        let rules = Rules::default();
        let c = cx(&rules);
        let v = decide("web.search", &c);
        assert_eq!(v.decision(), Decision::Allow);
        let v = v.ask_before_sending();
        assert_eq!((v.decision(), v.rule()), (Decision::Ask, "may send private data"));
        assert!(!v.task_approvable() && v.key_press(), "a key press, each time");
        let mut c = cx(&rules);
        c.tool_rule = Some(Decision::Deny);
        assert_eq!(decide("web.search", &c).ask_before_sending().decision(), Decision::Deny);
    }

    /// What a page with injected instructions could make a planner ask for, under the default
    /// rules in the most permissive mode. Nothing that changes the system, runs code or sends
    /// data away goes through on a spoken "approve", and nothing carries over to a new target.
    #[test]
    fn a_compromised_planner_cant_act_alone() {
        let rules = Rules::default();
        let c = cx(&rules);
        // Generated commands, admin work, shutdown, captures, computer use: key press each time.
        for t in ["terminal.run", "terminal.shell", "admin.install", "admin.service", "system.shutdown", "capture.clipboard", "input.type", "ui.act"] {
            let v = decide(t, &c);
            assert_eq!(v.decision(), Decision::Ask, "{t}");
            assert!(v.key_press(), "{t}");
        }
        assert_eq!(decide("shell", &c).decision(), Decision::Deny);
        // Startup files, ~/.local/bin and friends: a key press, never the whole task.
        let mut rc = cx(&rules);
        rc.runs_code = true;
        let v = decide("files.write", &rc);
        assert!(v.key_press() && !v.task_approvable());
        // A third-party tool's own claim of safety doesn't count.
        let mut ext = cx(&rules);
        ext.trusted = false;
        assert_eq!(decide("notes.add", &ext).decision(), Decision::Ask);
        // Data out after untrusted content: asks, and no task-wide approval covers it.
        let mut t = Task::new("1", "summarize this page", Budget { max_calls: 10, max_active_ms: 60_000 });
        t.move_to(TaskState::Planning).unwrap();
        t.note_read("browser.snapshot");
        t.note_read("files.read");
        assert!(t.may_leak("browser.navigate", &serde_json::json!({"url": "https://x.test/?d=key"})));
        let v = decide("browser.navigate", &c).ask_before_sending();
        assert_eq!(v.decision(), Decision::Ask);
        assert!(v.key_press(), "the page itself could say approve");
        assert!(!t.approve_for_task(&v, None) && !t.approved_for_task(&v, None));
        // "Allow for this task" while typing into one window doesn't cover another.
        let typing = decide("input.type", &c);
        assert!(t.approve_for_task(&typing, Some("write")));
        assert!(!t.approved_for_task(&typing, Some("firefox")));
    }
}
