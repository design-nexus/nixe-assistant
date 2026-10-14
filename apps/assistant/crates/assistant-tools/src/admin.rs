//! Administrative tools: packages and system services, run as root by the polkit helper
//! (`assistant-privileged`). The looking tools (`packages.*`, `services.system_status`)
//! run as the user.
//!
//! `preview` is the unprivileged dry run the host shows with the approval, so the person
//! sees the packages (dependencies included) or the unit before approving. The call itself
//! runs `pkexec nixe-assistant-helper VERB ARGS…`; polkit then asks for the password, and
//! the helper checks every argument again.

use anyhow::{bail, ensure, Context, Result};
use assistant_privileged::{protected_package, Action, HELPER, HELPER_VERSION, POLICY, POLICY_XML};
use serde_json::{json, Value};
use std::time::Duration;

pub const TOOLS: &[(&str, &str, &str)] = &[
    (
        "packages.search",
        "Search the configured package repositories by name or description.",
        r#"{"type":"object","properties":{"query":{"type":"string","minLength":1,"maxLength":100}},"required":["query"],"additionalProperties":false}"#,
    ),
    (
        "packages.info",
        "Details of a repository package and whether it is installed.",
        r#"{"type":"object","properties":{"package":{"type":"string","maxLength":200}},"required":["package"],"additionalProperties":false}"#,
    ),
    (
        "services.system_status",
        "Status of one systemd system unit.",
        r#"{"type":"object","properties":{"unit":{"type":"string","maxLength":128}},"required":["unit"],"additionalProperties":false}"#,
    ),
    (
        "admin.install",
        "Install packages from the configured repositories (name or repo/name). Asks for approval and the password.",
        r#"{"type":"object","properties":{"packages":{"type":"array","items":{"type":"string","maxLength":200},"minItems":1,"maxItems":10}},"required":["packages"],"additionalProperties":false}"#,
    ),
    (
        "admin.remove",
        "Remove installed packages and dependencies nothing else needs. Asks for approval and the password.",
        r#"{"type":"object","properties":{"packages":{"type":"array","items":{"type":"string","maxLength":200},"minItems":1,"maxItems":10}},"required":["packages"],"additionalProperties":false}"#,
    ),
    (
        "admin.update",
        "Update all system packages. Asks for approval and the password.",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    ),
    (
        "admin.service",
        "Start, stop or restart a systemd system service. Asks for approval and the password.",
        r#"{"type":"object","properties":{"unit":{"type":"string","maxLength":128},"action":{"enum":["start","stop","restart"]}},"required":["unit","action"],"additionalProperties":false}"#,
    ),
];

/// How long a call may take. Package work waits on downloads, so it gets far longer than
/// the usual 15 s; it includes the time spent at the password prompt.
pub fn deadline(tool: &str) -> Duration {
    Duration::from_secs(match tool {
        "admin.update" => 45 * 60,
        "admin.install" | "admin.remove" => 15 * 60,
        "admin.service" => 3 * 60,
        _ => 15,
    })
}

fn action(name: &str, args: &Value) -> Result<Action> {
    let list = || -> Vec<&str> { args["packages"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default() };
    let mut argv: Vec<&str> = match name {
        "admin.install" => vec!["install"],
        "admin.remove" => vec!["remove"],
        "admin.update" => vec!["update"],
        "admin.service" => vec!["service", args["action"].as_str().unwrap_or(""), args["unit"].as_str().unwrap_or("")],
        _ => bail!("unknown admin tool"),
    };
    if matches!(name, "admin.install" | "admin.remove") {
        argv.extend(list());
    }
    Action::parse(&argv).map_err(anyhow::Error::msg)
}

async fn run(program: &str, args: &[&str], secs: u64) -> Result<std::process::Output> {
    tokio::time::timeout(
        Duration::from_secs(secs),
        tokio::process::Command::new(program).args(args).env("LC_ALL", "C").kill_on_drop(true).output(),
    )
    .await
    .with_context(|| format!("{program} timed out"))?
    .with_context(|| format!("{program} isn't available"))
}

async fn stdout(program: &str, args: &[&str]) -> Result<String> {
    let out = run(program, args, 60).await?;
    ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("error: "));
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn bare(package: &str) -> &str {
    package.rsplit('/').next().unwrap_or(package)
}

async fn installed(package: &str) -> bool {
    run("pacman", &["-Q", "--", bare(package)], 30).await.is_ok_and(|o| o.status.success())
}

async fn unit_state(unit: &str) -> Result<Value> {
    let text = stdout("systemctl", &["show", "-p", "LoadState,ActiveState,SubState,UnitFileState,Description", "--", unit]).await?;
    let mut status = serde_json::Map::new();
    status.insert("unit".into(), json!(unit));
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            status.insert(k.to_ascii_lowercase(), json!(v));
        }
    }
    Ok(Value::Object(status))
}

/// The dry run shown with the approval. Fails, before anyone is asked, when the action
/// can't work: a package that doesn't exist, a protected package, a critical unit.
pub async fn preview(name: &str, args: &Value) -> Result<Value> {
    match action(name, args)? {
        Action::Install { packages } => {
            let mut argv = vec!["-Sp", "--needed", "--print-format", "%r/%n %v", "--"];
            argv.extend(packages.iter().map(String::as_str));
            let list: Vec<String> = stdout("pacman", &argv).await?.lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
            Ok(json!({"will_install": list, "note": if list.is_empty() { "already installed" } else { "" }}))
        }
        Action::Remove { packages } => {
            let mut argv = vec!["-Rsp", "--print-format", "%n %v", "--"];
            argv.extend(packages.iter().map(String::as_str));
            let list: Vec<String> = stdout("pacman", &argv).await?.lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
            if let Some(p) = list.iter().find(|l| protected_package(l.split(' ').next().unwrap_or(""))) {
                bail!("removing that would also remove {p}, which the assistant never removes");
            }
            Ok(json!({"will_remove": list}))
        }
        Action::Update => {
            // checkupdates syncs a private copy of the databases; exit 2 means nothing to do.
            let out = run("checkupdates", &[], 120).await;
            Ok(match out {
                Ok(o) if o.status.code() == Some(2) => json!({"will_update": [], "note": "everything is up to date"}),
                Ok(o) if o.status.success() => {
                    let list: Vec<String> = String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect();
                    json!({"will_update": list})
                }
                _ => json!({"will_update": "all packages (checkupdates couldn't list them)"}),
            })
        }
        Action::Service { action, unit } => Ok(json!({"action": action.as_str(), "status": unit_state(&unit).await?})),
    }
}

/// Whether the installed helper and policy are the ones this build carries.
fn helper_ready() -> Result<()> {
    let hint = "run: sudo nixe-assistant install-helper";
    ensure!(std::path::Path::new(HELPER).is_file(), "the admin helper isn't installed; {hint}");
    ensure!(std::fs::read_to_string(POLICY).ok().as_deref() == Some(POLICY_XML), "the admin helper's polkit policy is missing or outdated; {hint}");
    let version = std::process::Command::new(HELPER)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok());
    ensure!(version.is_some_and(|v| v >= HELPER_VERSION), "the admin helper is outdated; {hint}");
    Ok(())
}

pub async fn call(name: &str, args: &Value) -> Option<Result<Value>> {
    let s = |k: &str| args[k].as_str().unwrap_or("").to_string();
    Some(match name {
        "packages.search" => async {
            let q = s("query");
            ensure!(!q.starts_with('-'), "invalid query");
            let out = run("pacman", &["-Ss", "--", &q], 60).await?;
            // Exit 1 with no output is "no matches".
            let text = String::from_utf8_lossy(&out.stdout);
            let mut results = Vec::new();
            let mut lines = text.lines().peekable();
            while let Some(head) = lines.next() {
                let mut parts = head.split_whitespace();
                let (Some(id), Some(version)) = (parts.next(), parts.next()) else { continue };
                let description = lines.next_if(|l| l.starts_with(' ')).unwrap_or("").trim();
                results.push(json!({"package": id, "version": version, "installed": head.contains("[installed"), "description": description}));
                if results.len() == 30 {
                    break;
                }
            }
            Ok(json!({"results": results}))
        }
        .await,
        "packages.info" => async {
            let p = s("package");
            ensure!(assistant_privileged::valid_package(bare(&p)) && !p.starts_with('-'), "invalid package name");
            let text = stdout("pacman", &["-Si", "--", &p]).await?;
            let mut info = serde_json::Map::new();
            // The first block: a package in two repositories is listed twice.
            for line in text.lines().take_while(|l| !l.is_empty()) {
                if let Some((k, v)) = line.split_once(" : ") {
                    let key = k.trim();
                    if matches!(key, "Repository" | "Name" | "Version" | "Description" | "URL" | "Depends On" | "Download Size" | "Installed Size") {
                        info.insert(key.to_ascii_lowercase().replace(' ', "_"), json!(v.trim()));
                    }
                }
            }
            let local = run("pacman", &["-Q", "--", bare(&p)], 30).await?;
            let installed = local.status.success().then(|| String::from_utf8_lossy(&local.stdout).split_whitespace().nth(1).unwrap_or("").to_string());
            info.insert("installed_version".into(), json!(installed));
            Ok(Value::Object(info))
        }
        .await,
        "services.system_status" => async {
            let unit = s("unit");
            ensure!(crate::system::valid_unit(&unit), "invalid unit name");
            unit_state(&unit).await
        }
        .await,
        "admin.install" | "admin.remove" | "admin.update" | "admin.service" => async {
            let action = action(name, args)?;
            helper_ready()?;
            let mut argv = vec![HELPER.to_string()];
            argv.extend(action.to_args());
            let out = tokio::time::timeout(deadline(name), async {
                tokio::process::Command::new("pkexec").args(&argv).stdin(std::process::Stdio::null()).kill_on_drop(true).output().await
            })
            .await
            .context("the admin action timed out; it may still be running (see nixe-assistant admin-log)")?
            .context("pkexec isn't available")?;
            match out.status.code() {
                Some(0) => {}
                // pkexec: 126 when the prompt was dismissed, 127 when not authorized.
                Some(126 | 127) if out.stdout.is_empty() => bail!("authentication was cancelled or refused; nothing was changed"),
                _ => bail!("{}", String::from_utf8_lossy(&out.stderr).trim()),
            }
            let mut result: Value = serde_json::from_slice(&out.stdout).context("unexpected reply from the admin helper")?;
            let verified = match &action {
                Action::Install { packages } => {
                    let mut all = true;
                    for p in packages {
                        all &= installed(p).await;
                    }
                    all
                }
                Action::Remove { packages } => {
                    let mut none = true;
                    for p in packages {
                        none &= !installed(p).await;
                    }
                    none
                }
                Action::Update => true,
                Action::Service { action, unit } => {
                    let state = unit_state(unit).await.ok();
                    let active = state.as_ref().and_then(|s| s["activestate"].as_str()).unwrap_or("").to_string();
                    result["status"] = state.unwrap_or(Value::Null);
                    match action {
                        assistant_privileged::ServiceAction::Stop => matches!(active.as_str(), "inactive" | "failed"),
                        _ => active == "active",
                    }
                }
            };
            result["verified"] = json!(verified);
            Ok(result)
        }
        .await,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_parse_and_actions() {
        for (name, _, schema) in TOOLS {
            serde_json::from_str::<Value>(schema).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        assert_eq!(action("admin.install", &json!({"packages":["extra/sl"]})).unwrap().to_args(), ["install", "extra/sl"]);
        assert_eq!(action("admin.service", &json!({"unit":"cups.service","action":"restart"})).unwrap().to_args(), ["service", "restart", "cups.service"]);
        assert!(action("admin.install", &json!({"packages":["-Syu"]})).is_err());
        assert!(action("admin.service", &json!({"unit":"dbus.service","action":"stop"})).is_err());
        assert!(action("admin.update", &json!({})).is_ok());
        assert!(deadline("admin.update") > deadline("admin.service") && deadline("files.write") == Duration::from_secs(15));
    }
}
