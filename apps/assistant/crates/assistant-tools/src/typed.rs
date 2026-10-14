//! Built-in typed tools for the voice assistant.
//!
//! Provides safely sandboxed read-only file access, git status queries,
//! system inspection, and coding agent handoffs.

use crate::agent::launch_coding_agent;
use anyhow::{Context, Result};
use assistant_core::config::{AgentCfg, PermissionsCfg};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use tokio::process::Command;

pub async fn execute_typed_tool(
    name: &str,
    args: &Value,
    permissions: &PermissionsCfg,
    agent_cfg: &AgentCfg,
) -> Result<String> {
    match name {
        "files_read" | "files.read" => {
            if permissions.files_read != "allow" {
                anyhow::bail!("files_read permission is not allowed in config");
            }
            let path = args
                .get("path")
                .and_then(|p| p.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required argument: path"))?;
            let start = args
                .get("start_line")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let end = args
                .get("end_line")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let path = path.to_string();
            tokio::task::spawn_blocking(move || read_file_safe(&path, start, end)).await?
        }
        "git_status" | "git.status" => {
            let dir = args.get("directory").and_then(|d| d.as_str());
            git_status(dir).await
        }
        "system_status" | "system.status" => system_status().await,
        "agent_handoff" | "agent.handoff" => {
            if !agent_cfg.enabled {
                anyhow::bail!("agent handoff is disabled in config");
            }
            let prompt = args.get("prompt").and_then(|p| p.as_str()).unwrap_or("");
            let agent = args
                .get("agent")
                .and_then(|a| a.as_str())
                .unwrap_or(&agent_cfg.default_agent);
            launch_coding_agent(agent, prompt, &agent_cfg.terminal).await
        }
        _ => anyhow::bail!("unknown typed tool: {name}"),
    }
}

pub fn read_file_safe(
    path_str: &str,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> Result<String> {
    let path = home_path(path_str)?;
    if !path.is_file() {
        anyhow::bail!("file not found: {}", path.display());
    }
    let file = File::open(&path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = BufReader::new(file.take(64 * 1024));

    let start = start_line.unwrap_or(1).max(1);
    let end = end_line.unwrap_or(start.saturating_add(199)).max(start);
    let max_lines = 500;
    let actual_end = end.min(start.saturating_add(max_lines - 1));

    let mut out = String::new();
    let mut count = 0;
    for (idx, line_res) in reader.lines().enumerate() {
        let line_num = idx + 1;
        if line_num >= start && line_num <= actual_end {
            let line = line_res?;
            out.push_str(&format!("{line_num}: {line}\n"));
            count += 1;
        }
        if line_num > actual_end {
            break;
        }
    }

    if count == 0 {
        Ok(format!(
            "(file {} has no lines in range {start}-{actual_end})",
            path.display()
        ))
    } else {
        Ok(out)
    }
}

pub async fn git_status(dir_opt: Option<&str>) -> Result<String> {
    if dir_opt.is_none() {
        home_path(".")?;
    }
    let mut cmd = Command::new("git");
    cmd.args([
        "-c",
        "core.fsmonitor=false",
        "status",
        "--short",
        "--branch",
    ]);
    if let Some(dir) = dir_opt {
        cmd.current_dir(home_path(dir)?);
    }
    cmd.kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(15), cmd.output()).await??;
    if !out.status.success() {
        anyhow::bail!(
            "git status failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let res = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if res.is_empty() {
        Ok("Working tree clean.".into())
    } else {
        Ok(res)
    }
}

pub async fn system_status() -> Result<String> {
    let mut details = Vec::new();

    // Check battery if available
    if let Ok(cap) = std::fs::read_to_string("/sys/class/power_supply/BAT0/capacity") {
        let status = std::fs::read_to_string("/sys/class/power_supply/BAT0/status")
            .unwrap_or_else(|_| "Unknown".into());
        details.push(format!("Battery: {}% ({})", cap.trim(), status.trim()));
    }

    // Check load avg
    if let Ok(load) = std::fs::read_to_string("/proc/loadavg") {
        let parts: Vec<&str> = load.split_whitespace().take(3).collect();
        details.push(format!("Load: {}", parts.join(", ")));
    }

    // Check meminfo
    if let Ok(mem) = std::fs::read_to_string("/proc/meminfo") {
        let mut total = 0u64;
        let mut avail = 0u64;
        for line in mem.lines() {
            if line.starts_with("MemTotal:") {
                total = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            } else if line.starts_with("MemAvailable:") {
                avail = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
        }
        if total > 0 {
            let used_gib = (total.saturating_sub(avail)) as f64 / 1024.0 / 1024.0;
            let total_gib = total as f64 / 1024.0 / 1024.0;
            details.push(format!("Memory: {:.1} / {:.1} GiB", used_gib, total_gib));
        }
    }

    if details.is_empty() {
        Ok("System status unavailable.".into())
    } else {
        Ok(details.join(" | "))
    }
}

/// A readable path inside HOME, outside secret folders (see `crate::fs::guard`). Relative
/// paths are relative to HOME.
pub fn home_path(input: &str) -> Result<std::path::PathBuf> {
    crate::fs::guard(&crate::fs::home()?, input, crate::fs::Access::Read)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_file_safe() {
        // The checkout is normally inside HOME; the guard refuses anything outside it.
        let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
        match read_file_safe(manifest, Some(1), Some(5)) {
            Ok(res) => assert!(res.contains("1:")),
            Err(e) => assert!(e.to_string().contains("outside the home directory"), "{e}"),
        }
    }
}
