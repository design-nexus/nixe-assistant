//! Coding agent handoff runner for Omarchy.
//!
//! Spawns interactive coding agents (agy, claude, codex, cursor) in a dedicated
//! terminal window on the current Hyprland workspace.

use anyhow::Result;

pub fn normalize_agent_name(name: &str) -> &str {
    match name.trim().to_lowercase().as_str() {
        "claude" => "claude",
        "codex" => "codex",
        "cursor" => "cursor",
        "antigravity" | "agy" => "agy",
        _ => "agy",
    }
}

pub fn spoken_agent_name(name: &str) -> &str {
    match normalize_agent_name(name) {
        "claude" => "Claude",
        "codex" => "Codex",
        "cursor" => "Cursor",
        "agy" => "Antigravity",
        _ => "Antigravity",
    }
}

pub fn build_launch_cmd(agent: &str, prompt: &str, terminal: &str) -> (String, String) {
    let normalized = normalize_agent_name(agent);
    let prompt_clean = prompt.trim();
    let term = if terminal.trim().is_empty() {
        "omarchy-launch-terminal"
    } else {
        terminal.trim()
    };

    let mut args = vec![term, "--", normalized];
    if normalized == "cursor" {
        args = vec!["cursor", "."];
    } else if !prompt_clean.is_empty() {
        if normalized == "agy" {
            args.push("-i");
        }
        args.push(prompt);
    }
    let cmd = assistant_omarchy::shell_join(&args);
    let spoken = format!(
        "{} {}.",
        if prompt_clean.is_empty() {
            "Opening"
        } else {
            "Handing off to"
        },
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
    }

    #[test]
    fn test_build_launch_cmd() {
        let (cmd, spoken) = build_launch_cmd("agy", "fix the test", "omarchy-launch-terminal");
        assert_eq!(
            cmd,
            "'omarchy-launch-terminal' '--' 'agy' '-i' 'fix the test'"
        );
        assert_eq!(spoken, "Handing off to Antigravity.");

        let (cmd, spoken) = build_launch_cmd("claude", "refactor auth", "omarchy-launch-terminal");
        assert_eq!(
            cmd,
            "'omarchy-launch-terminal' '--' 'claude' 'refactor auth'"
        );
        assert_eq!(spoken, "Handing off to Claude.");

        let (cmd, spoken) = build_launch_cmd("agy", "", "omarchy-launch-terminal");
        assert_eq!(cmd, "'omarchy-launch-terminal' '--' 'agy'");
        assert_eq!(spoken, "Opening Antigravity.");
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
