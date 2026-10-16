//! Coding agent handoff runner for Omarchy.
//!
//! Spawns an interactive coding agent in a dedicated terminal window on the current Hyprland
//! workspace. The agents are the ones Omarchy knows (`omarchy-default-agent`), each started
//! with its own way of taking a first prompt. Unlike Omarchy's launcher, none is started with
//! its "skip permission prompts" flag: a spoken request shouldn't remove the agent's own checks.

use anyhow::Result;

pub struct Agent {
    /// The config value and Omarchy's name for it.
    pub id: &'static str,
    pub name: &'static str,
    /// The program that must be on PATH.
    pub program: &'static str,
    /// How it's said: "ask grok to …", "open muse".
    pub words: &'static [&'static str],
}

pub const AGENTS: &[Agent] = &[
    Agent { id: "claude", name: "Claude Code", program: "claude", words: &["claude", "claude code"] },
    Agent { id: "codex", name: "Codex", program: "codex", words: &["codex"] },
    Agent { id: "opencode", name: "OpenCode", program: "opencode", words: &["opencode", "open code"] },
    Agent { id: "pi", name: "Pi", program: "pi", words: &["pi"] },
    Agent { id: "omp", name: "Oh My Pi", program: "omp", words: &["oh my pi", "omp"] },
    Agent { id: "ori", name: "Ori", program: "ori", words: &["ori"] },
    Agent { id: "grok", name: "Grok", program: "grok", words: &["grok"] },
    Agent { id: "agy", name: "Antigravity", program: "agy", words: &["antigravity", "agy"] },
    Agent { id: "copilot", name: "GitHub Copilot", program: "copilot", words: &["github copilot", "copilot"] },
    Agent { id: "crush", name: "Crush", program: "crush", words: &["crush"] },
    Agent { id: "cursor-agent", name: "Cursor CLI", program: "cursor-agent", words: &["cursor agent", "cursor cli"] },
    Agent { id: "cursor", name: "Cursor", program: "cursor", words: &["cursor"] },
    Agent { id: "muse", name: "Muse Code", program: "muse", words: &["muse code", "muse"] },
    Agent { id: "hermes", name: "Hermes", program: "hermes", words: &["hermes"] },
    Agent { id: "openclaw", name: "OpenClaw", program: "openclaw", words: &["openclaw", "open claw"] },
];

pub fn agent(id: &str) -> Option<&'static Agent> {
    let id = id.trim().to_lowercase();
    AGENTS.iter().find(|a| a.id == id || a.words.contains(&id.as_str()))
}

/// The config id for a name or id; unknown names fall back to Antigravity, as before.
pub fn normalize_agent_name(name: &str) -> &'static str {
    agent(name).map_or("agy", |a| a.id)
}

pub fn spoken_agent_name(name: &str) -> &'static str {
    agent(normalize_agent_name(name)).map_or("Antigravity", |a| a.name)
}

/// Whether `program` is an executable on PATH.
pub fn on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            std::fs::metadata(dir.join(program)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

/// The agents installed here, in the table's order.
pub fn installed_agents() -> Vec<&'static Agent> {
    AGENTS.iter().filter(|a| on_path(a.program)).collect()
}

/// The agent's command line, with the prompt as its first message when there is one.
pub fn agent_args(id: &str, prompt: &str) -> Vec<String> {
    let p = prompt.trim();
    let has = !p.is_empty();
    let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut args = match normalize_agent_name(id) {
        // `-- prompt` keeps a prompt that starts with a dash from being read as an option.
        a @ ("claude" | "codex" | "grok" | "muse" | "omp") => {
            let mut x = v(&[a]);
            if has {
                x.extend(v(&["--", p]));
            }
            x
        }
        "pi" => v(&["pi"]),
        "opencode" => v(&if has { vec!["opencode", "--prompt"] } else { vec!["opencode"] }),
        "agy" => v(&if has { vec!["agy", "--prompt-interactive"] } else { vec!["agy"] }),
        "copilot" => v(&if has { vec!["copilot", "--interactive"] } else { vec!["copilot"] }),
        "crush" => v(&if has { vec!["crush", "run"] } else { vec!["crush"] }),
        // Named outright so a one-word prompt like "update" isn't taken as a subcommand.
        "cursor-agent" => v(&if has { vec!["cursor-agent", "agent", "--"] } else { vec!["cursor-agent"] }),
        "hermes" => {
            return if has {
                v(&["env", "-u", "HERMES_SESSION_SOURCE", "hermes", "chat", "--tui", &format!("--query={p}")])
            } else {
                v(&["hermes"])
            }
        }
        "ori" => v(&if has { vec!["ori", "code", "--interactive", "--prompt"] } else { vec!["ori", "code"] }),
        "openclaw" => v(&if has { vec!["omarchy-launch-openclaw", "--tui", "--message"] } else { vec!["omarchy-launch-openclaw", "--tui"] }),
        _ => v(&["agy"]),
    };
    if has && !matches!(normalize_agent_name(id), "claude" | "codex" | "grok" | "muse" | "omp") {
        args.push(p.to_string());
    }
    args
}

pub fn build_launch_cmd(agent: &str, prompt: &str, terminal: &str) -> (String, String) {
    let normalized = normalize_agent_name(agent);
    let prompt_clean = prompt.trim();
    let term = if terminal.trim().is_empty() { "omarchy-launch-terminal" } else { terminal.trim() };
    // Cursor is the editor: it opens the folder in its own window.
    let args: Vec<String> = if normalized == "cursor" {
        vec!["cursor".into(), ".".into()]
    } else {
        [term.to_string(), "--".to_string()].into_iter().chain(agent_args(normalized, prompt_clean)).collect()
    };
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let cmd = assistant_omarchy::shell_join(&refs);
    let spoken = format!(
        "{} {}.",
        if prompt_clean.is_empty() || normalized == "cursor" { "Opening" } else { "Handing off to" },
        spoken_agent_name(normalized)
    );
    (cmd, spoken)
}

pub async fn launch_coding_agent(agent: &str, prompt: &str, terminal: &str) -> Result<String> {
    let (cmd, spoken) = build_launch_cmd(agent, prompt, terminal);
    tracing::info!(%cmd, %spoken, "dispatching coding agent handoff");
    assistant_omarchy::hypr_exec(&cmd).await?;
    Ok(spoken)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_agent() {
        assert_eq!(normalize_agent_name("Antigravity"), "agy");
        assert_eq!(normalize_agent_name("agy"), "agy");
        assert_eq!(normalize_agent_name("claude"), "claude");
        assert_eq!(normalize_agent_name("codex"), "codex");
        assert_eq!(normalize_agent_name("cursor"), "cursor");
        assert_eq!(normalize_agent_name("Grok"), "grok");
        assert_eq!(normalize_agent_name("muse code"), "muse");
        assert_eq!(normalize_agent_name("open code"), "opencode");
        assert_eq!(normalize_agent_name("cursor agent"), "cursor-agent");
        assert_eq!(normalize_agent_name("nonsense"), "agy");
        assert_eq!(spoken_agent_name("copilot"), "GitHub Copilot");
    }

    #[test]
    fn test_build_launch_cmd() {
        let (cmd, spoken) = build_launch_cmd("agy", "fix the test", "omarchy-launch-terminal");
        assert_eq!(cmd, "'omarchy-launch-terminal' '--' 'agy' '--prompt-interactive' 'fix the test'");
        assert_eq!(spoken, "Handing off to Antigravity.");

        let (cmd, spoken) = build_launch_cmd("claude", "refactor auth", "omarchy-launch-terminal");
        assert_eq!(cmd, "'omarchy-launch-terminal' '--' 'claude' '--' 'refactor auth'");
        assert_eq!(spoken, "Handing off to Claude Code.");

        let (cmd, spoken) = build_launch_cmd("agy", "", "omarchy-launch-terminal");
        assert_eq!(cmd, "'omarchy-launch-terminal' '--' 'agy'");
        assert_eq!(spoken, "Opening Antigravity.");

        let (cmd, _) = build_launch_cmd("cursor", "anything", "t");
        assert_eq!(cmd, "'cursor' '.'");
    }

    #[test]
    fn every_agent_takes_a_prompt_without_skipping_its_checks() {
        for a in AGENTS {
            let with = agent_args(a.id, "-n fix it");
            let without = agent_args(a.id, "");
            if a.id != "hermes" {
                assert!(with.iter().any(|x| x == "-n fix it"), "{}: {with:?}", a.id);
            }
            assert!(!without.iter().any(|x| x.contains("fix")), "{}", a.id);
            for flag in ["--yolo", "--dangerously-skip-permissions", "--allow-all", "--auto", "--auto-approve", "bypassPermissions"] {
                assert!(!with.iter().chain(&without).any(|x| x == flag), "{}: {flag}", a.id);
            }
        }
        assert_eq!(agent_args("hermes", "hi").last().unwrap(), "--query=hi");
        assert_eq!(agent_args("cursor-agent", "update"), ["cursor-agent", "agent", "--", "update"]);
        assert_eq!(agent_args("crush", "x"), ["crush", "run", "x"]);
    }

    #[test]
    fn installed_means_on_path() {
        assert!(on_path("sh"));
        assert!(!on_path("definitely-not-an-agent-xyz"));
        assert!(installed_agents().iter().all(|a| on_path(a.program)));
    }
}

#[cfg(test)]
mod boundary_tests {
    #[test]
    fn untrusted_shell_arguments_round_trip_without_execution() {
        let values = [
            "",
            "space and 'quotes'",
            "$HOME $(touch /tmp/assistant-injection) `id`",
            "line\nbreak",
            "こんにちは\\world",
        ];
        let args = std::iter::once("printf")
            .chain(std::iter::once("%s\\0"))
            .chain(values)
            .collect::<Vec<_>>();
        let output = std::process::Command::new("sh")
            .args(["-c", &assistant_omarchy::shell_join(&args)])
            .output()
            .unwrap();
        assert!(output.status.success());
        let mut expected = Vec::new();
        for value in values {
            expected.extend(value.as_bytes());
            expected.push(0);
        }
        assert_eq!(output.stdout, expected);
        let (cmd, _) = super::build_launch_cmd("codex", values[2], "my terminal");
        assert!(cmd.starts_with("'my terminal' '--' 'codex' "));
    }
}
