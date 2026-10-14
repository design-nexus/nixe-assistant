//! Permission decisions for assistant tools.
//!
//! The decision point (`decide`) turns a tool name and the user's rules into a `Verdict`.
//! Only a verdict can mint a `Grant`, and the registry's enforcement point dispatches only
//! with a grant bound to the exact tool and arguments. The audit journal (`store`) records
//! decisions, approvals and outcomes with sensitive values redacted.

pub mod capability;
pub mod grant;
pub mod store;

pub use capability::Capability;
pub use grant::{digest, Grant, GrantKind};
use serde::{Deserialize, Serialize};

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
    pub shutdown: Decision,
}

impl Default for Rules {
    fn default() -> Self {
        Self { launch_app: Decision::Allow, volume: Decision::Allow, files_read: Decision::Allow, shutdown: Decision::Ask }
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    tool: String,
    capability: Capability,
    decision: Decision,
    rule: &'static str,
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
}

/// The decision point. Order: unavailable or disabled tools are denied; an explicit deny
/// (per tool or per capability) wins; then the per-tool rule, untrusted plugins asking,
/// the capability rule, the capability default; then the autonomy mode may only tighten,
/// and mandatory floors cap what any rule can allow.
pub fn decide(tool: &str, cx: &Context) -> Verdict {
    let capability = Capability::of(tool);
    let verdict = |decision, rule| Verdict { tool: tool.into(), capability, decision, rule };
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
    verdict(decision, rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(rules: &Rules) -> Context<'_> {
        Context { available: true, trusted: true, tool_rule: None, rules, mode: AutonomyMode::Autonomous }
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
}
