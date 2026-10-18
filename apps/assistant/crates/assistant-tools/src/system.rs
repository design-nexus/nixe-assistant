//! Typed desktop and system tools: Hyprland windows and workspaces, systemd user units,
//! NetworkManager, PipeWire devices and power profiles.
//!
//! Each change re-reads the state it changed and reports `verified`, so a task knows
//! whether the window moved or the unit started instead of assuming it.

use anyhow::{bail, ensure, Context, Result};
use assistant_omarchy::hypr;
use serde_json::{json, Value};
use std::time::Duration;

pub const TOOLS: &[(&str, &str, &str)] = &[
    (
        "windows.list",
        "List open windows with address, class, title and workspace.",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    ),
    ("windows.active", "The focused window.", r#"{"type":"object","properties":{},"additionalProperties":false}"#),
    (
        "windows.focus",
        "Focus a window by address, class or title.",
        r#"{"type":"object","properties":{"window":{"type":"string","maxLength":256}},"required":["window"],"additionalProperties":false}"#,
    ),
    (
        "windows.move",
        "Move a window (address, class or title) to a workspace number or special:<name>.",
        r#"{"type":"object","properties":{"window":{"type":"string","maxLength":256},"workspace":{"type":"string","maxLength":40}},"required":["window","workspace"],"additionalProperties":false}"#,
    ),
    (
        "windows.close",
        "Close a window by address, class or title.",
        r#"{"type":"object","properties":{"window":{"type":"string","maxLength":256}},"required":["window"],"additionalProperties":false}"#,
    ),
    (
        "workspaces.list",
        "List workspaces and the active one.",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    ),
    (
        "workspaces.switch",
        "Switch to a workspace number or special:<name>.",
        r#"{"type":"object","properties":{"workspace":{"type":"string","maxLength":40}},"required":["workspace"],"additionalProperties":false}"#,
    ),
    (
        "monitors.list",
        "List monitors with resolution, scale and active workspace.",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    ),
    (
        "services.list",
        "List systemd user services and timers with their state.",
        r#"{"type":"object","properties":{"failed_only":{"type":"boolean"}},"additionalProperties":false}"#,
    ),
    (
        "services.status",
        "Status of one systemd user unit.",
        r#"{"type":"object","properties":{"unit":{"type":"string","maxLength":128}},"required":["unit"],"additionalProperties":false}"#,
    ),
    (
        "services.control",
        "Start, stop or restart a systemd user unit.",
        r#"{"type":"object","properties":{"unit":{"type":"string","maxLength":128},"action":{"enum":["start","stop","restart"]}},"required":["unit","action"],"additionalProperties":false}"#,
    ),
    (
        "network.status",
        "Connectivity, Wi-Fi radio and active connections.",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    ),
    (
        "network.wifi",
        "Turn the Wi-Fi radio on or off.",
        r#"{"type":"object","properties":{"on":{"type":"boolean"}},"required":["on"],"additionalProperties":false}"#,
    ),
    (
        "audio.devices",
        "List audio outputs and inputs with volume, mute and which is the default.",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    ),
    (
        "audio.set_volume",
        "Set an output's volume in percent (default output unless device is named).",
        r#"{"type":"object","properties":{"percent":{"type":"integer","minimum":0,"maximum":150},"device":{"type":"string","maxLength":256}},"required":["percent"],"additionalProperties":false}"#,
    ),
    (
        "audio.set_default",
        "Make an output (by name from audio.devices) the default.",
        r#"{"type":"object","properties":{"device":{"type":"string","maxLength":256}},"required":["device"],"additionalProperties":false}"#,
    ),
    (
        "system.power_profile",
        "Read, or set, the power profile: power-saver, balanced or performance.",
        r#"{"type":"object","properties":{"profile":{"enum":["power-saver","balanced","performance"]}},"additionalProperties":false}"#,
    ),
];

pub(crate) async fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(program).args(args).kill_on_drop(true).output(),
    )
    .await
    .with_context(|| format!("{program} timed out"))?
    .with_context(|| format!("{program} isn't available"))?;
    ensure!(out.status.success(), "{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Wait up to `ms` for `check` to hold, polling.
pub(crate) async fn settle<F, Fut>(ms: u64, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        if check().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn window_json(c: &hypr::Client) -> Value {
    json!({"address": c.address, "class": c.class, "title": c.title, "workspace": c.workspace.name, "floating": c.floating, "focused": c.focus_history_id == 0})
}

async fn one_window(query: &str) -> Result<hypr::Client> {
    let clients = hypr::clients().await?;
    let found = hypr::match_windows(&clients, query);
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!("no window matches {query:?}"),
        many => bail!(
            "{} windows match {query:?}; use an address: {}",
            many.len(),
            many.iter().map(|c| format!("{} ({}: {})", c.address, c.class, c.title)).collect::<Vec<_>>().join(", ")
        ),
    }
}

pub fn valid_unit(unit: &str) -> bool {
    let Some((name, kind)) = unit.rsplit_once('.') else { return false };
    !name.is_empty()
        && unit.len() <= 128
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "@._-".contains(c))
        && matches!(kind, "service" | "timer" | "socket" | "target")
}

fn own_unit(unit: &str) -> bool {
    unit.starts_with("nixe-assistant") || unit.starts_with("omarchy-assistant")
}

async fn unit_status(unit: &str) -> Result<Value> {
    let text =
        run("systemctl", &["--user", "show", unit, "-p", "LoadState,ActiveState,SubState,UnitFileState,Description"]).await?;
    let mut status = serde_json::Map::new();
    status.insert("unit".into(), json!(unit));
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            status.insert(k.to_lowercase(), json!(v));
        }
    }
    Ok(Value::Object(status))
}

fn device_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 256 && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-:@".contains(c))
}

fn first_percent(text: &str) -> Option<u64> {
    text.split_whitespace().find_map(|w| w.strip_suffix('%')?.parse().ok())
}

async fn audio_list(kind: &str) -> Result<Vec<Value>> {
    let all: Value = serde_json::from_str(&run("pactl", &["-f", "json", "list", kind]).await?)?;
    let default =
        run("pactl", &[if kind == "sinks" { "get-default-sink" } else { "get-default-source" }]).await?.trim().to_string();
    Ok(all
        .as_array()
        .context("unexpected pactl reply")?
        .iter()
        .filter(|d| !d["name"].as_str().unwrap_or("").ends_with(".monitor"))
        .map(|d| {
            let volume = d["volume"].as_object().and_then(|v| v.values().next()).and_then(|c| c["value_percent"].as_str()).and_then(first_percent);
            json!({"name": d["name"], "description": d["description"], "volume_percent": volume, "muted": d["mute"], "default": d["name"].as_str() == Some(default.as_str())})
        })
        .collect())
}

/// Run a system tool, or `None` if `name` isn't one.
pub async fn call(name: &str, args: &Value) -> Option<Result<Value>> {
    let s = |k: &str| args[k].as_str().unwrap_or("");
    Some(match name {
        "windows.list" => async { Ok(json!({"windows": hypr::clients().await?.iter().map(window_json).collect::<Vec<_>>()})) }.await,
        "windows.active" => async { Ok(json!({"window": hypr::active_window().await?.as_ref().map(window_json)})) }.await,
        "windows.focus" => async {
            let w = one_window(s("window")).await?;
            hypr::focus_window(&w.address).await?;
            let verified = settle(1500, || async { hypr::active_window().await.ok().flatten().is_some_and(|a| a.address == w.address) }).await;
            Ok(json!({"window": window_json(&w), "verified": verified}))
        }
        .await,
        "windows.move" => async {
            let w = one_window(s("window")).await?;
            let target = hypr::workspace_selector(s("workspace"))?;
            hypr::move_window(&w.address, &target).await?;
            let verified = settle(1500, || async {
                hypr::clients().await.ok().is_some_and(|cs| cs.iter().any(|c| c.address == w.address && (c.workspace.name == target || c.workspace.id.to_string() == target)))
            })
            .await;
            Ok(json!({"window": window_json(&w), "workspace": target, "verified": verified}))
        }
        .await,
        "windows.close" => async {
            let w = one_window(s("window")).await?;
            hypr::close_window(&w.address).await?;
            let verified = settle(2000, || async { hypr::clients().await.ok().is_some_and(|cs| cs.iter().all(|c| c.address != w.address)) }).await;
            Ok(json!({"window": window_json(&w), "verified": verified, "note": if verified { "" } else { "The window is still open; it may be asking to save." }}))
        }
        .await,
        "workspaces.list" => async { Ok(json!({"workspaces": hypr::workspaces().await?, "active": hypr::active_workspace().await?})) }.await,
        "workspaces.switch" => async {
            let target = hypr::workspace_selector(s("workspace"))?;
            hypr::focus_workspace(&target).await?;
            let verified = settle(1500, || async { hypr::active_workspace().await.ok().is_some_and(|a| a.name == target || a.id.to_string() == target) }).await;
            Ok(json!({"workspace": target, "verified": verified}))
        }
        .await,
        "monitors.list" => async { Ok(json!({"monitors": hypr::monitors().await?})) }.await,
        "services.list" => async {
            let all: Value = serde_json::from_str(&run("systemctl", &["--user", "list-units", "--type=service,timer", "--all", "--no-legend", "--plain", "-o", "json"]).await?)?;
            let units: Vec<Value> = all
                .as_array()
                .context("unexpected systemctl reply")?
                .iter()
                .filter(|u| args["failed_only"] != true || u["active"] == "failed")
                .take(200)
                .cloned()
                .collect();
            Ok(json!({"units": units}))
        }
        .await,
        "services.status" => async {
            let unit = s("unit");
            ensure!(valid_unit(unit), "invalid unit name");
            unit_status(unit).await
        }
        .await,
        "services.control" => async {
            let (unit, action) = (s("unit"), s("action"));
            ensure!(valid_unit(unit), "invalid unit name");
            ensure!(!own_unit(unit), "the assistant doesn't control its own services");
            ensure!(matches!(action, "start" | "stop" | "restart"), "action must be start, stop or restart");
            run("systemctl", &["--user", action, unit]).await?;
            let want = if action == "stop" { ["inactive", "failed"] } else { ["active", "active"] };
            let verified = settle(3000, || async { unit_status(unit).await.ok().is_some_and(|st| want.contains(&st["activestate"].as_str().unwrap_or(""))) }).await;
            Ok(json!({"status": unit_status(unit).await?, "verified": verified}))
        }
        .await,
        "network.status" => async {
            let general = run("nmcli", &["-t", "-f", "STATE,CONNECTIVITY,WIFI,NETWORKING", "general"]).await?;
            let f: Vec<&str> = general.trim().split(':').collect();
            let active = run("nmcli", &["-t", "-f", "NAME,TYPE,DEVICE", "connection", "show", "--active"]).await?;
            let connections: Vec<Value> = active
                .lines()
                .filter_map(|l| {
                    let p: Vec<&str> = l.rsplitn(3, ':').collect();
                    (p.len() == 3).then(|| json!({"name": p[2], "type": p[1], "device": p[0]}))
                })
                .collect();
            Ok(json!({"state": f.first(), "connectivity": f.get(1), "wifi": f.get(2), "networking": f.get(3), "connections": connections}))
        }
        .await,
        "network.wifi" => async {
            let on = args["on"].as_bool().context("on must be true or false")?;
            run("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]).await?;
            let want = if on { "enabled" } else { "disabled" };
            let verified = settle(3000, || async { run("nmcli", &["-t", "radio", "wifi"]).await.is_ok_and(|s| s.trim() == want) }).await;
            Ok(json!({"wifi": want, "verified": verified}))
        }
        .await,
        "audio.devices" => async { Ok(json!({"outputs": audio_list("sinks").await?, "inputs": audio_list("sources").await?})) }.await,
        "audio.set_volume" => async {
            let percent = args["percent"].as_u64().context("percent required")?.min(150);
            let device = match s("device") {
                "" => "@DEFAULT_SINK@".to_string(),
                d => {
                    ensure!(device_name_ok(d), "invalid device name");
                    d.to_string()
                }
            };
            run("pactl", &["set-sink-volume", &device, &format!("{percent}%")]).await?;
            let now = run("pactl", &["get-sink-volume", &device]).await.ok().and_then(|t| first_percent(&t));
            Ok(json!({"device": device, "percent": now, "verified": now == Some(percent)}))
        }
        .await,
        "audio.set_default" => async {
            let device = s("device");
            ensure!(device_name_ok(device), "invalid device name");
            ensure!(
                audio_list("sinks").await?.iter().any(|d| d["name"] == device),
                "no output named {device}; see audio.devices"
            );
            run("pactl", &["set-default-sink", device]).await?;
            let verified = run("pactl", &["get-default-sink"]).await.is_ok_and(|d| d.trim() == device);
            Ok(json!({"device": device, "verified": verified}))
        }
        .await,
        "system.power_profile" => async {
            match args["profile"].as_str() {
                None => Ok(json!({"profile": run("powerprofilesctl", &["get"]).await?.trim()})),
                Some(p) => {
                    ensure!(matches!(p, "power-saver" | "balanced" | "performance"), "unknown profile");
                    run("powerprofilesctl", &["set", p]).await?;
                    let now = run("powerprofilesctl", &["get"]).await?.trim().to_string();
                    Ok(json!({"profile": now, "verified": now == p}))
                }
            }
        }
        .await,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units() {
        for ok in ["foo.service", "app-dropbox@autostart.service", "backup.timer", "a_b-c.socket"] {
            assert!(valid_unit(ok), "{ok}");
        }
        for bad in ["foo", ".service", "foo.mount", "a b.service", "../x.service", "x;rm.service"] {
            assert!(!valid_unit(bad), "{bad}");
        }
        assert!(own_unit("nixe-assistantd.service") && own_unit("nixe-assistant-stt.service"));
    }

    #[test]
    fn schemas_parse_and_parsers() {
        for (name, _, schema) in TOOLS {
            assert!(serde_json::from_str::<Value>(schema).is_ok(), "{name}");
        }
        assert_eq!(first_percent("Volume: front-left: 22937 /  35% / -27.36 dB"), Some(35));
        assert!(device_name_ok("alsa_output.pci-0000_00_1f.3.analog-stereo") && !device_name_ok("a b"));
    }
}
