//! Bounded, read-only system probes. Missing sensors are reported rather than guessed.

use crate::system::{run, valid_unit};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{path::Path, time::{SystemTime, UNIX_EPOCH}};

pub const TOOLS: &[(&str, &str, &str)] = &[
    ("diagnostics.snapshot", "Read CPU load, RAM, GPU, temperatures, filesystems, busiest processes and network state for troubleshooting.", r#"{"type":"object","properties":{},"additionalProperties":false}"#),
    ("diagnostics.service_status", "Read the status of one named user or system service without changing it.", r#"{"type":"object","properties":{"unit":{"type":"string","maxLength":128},"scope":{"enum":["user","system"]}},"required":["unit"],"additionalProperties":false}"#),
    ("diagnostics.journal", "Read up to 100 recent journal entries for one named user or system service; explain findings before proposing changes.", r#"{"type":"object","properties":{"unit":{"type":"string","maxLength":128},"scope":{"enum":["user","system"]},"lines":{"type":"integer","minimum":1,"maximum":100}},"required":["unit"],"additionalProperties":false}"#),
    ("diagnostics.network", "Diagnose NetworkManager connectivity, the default route and DNS servers; optionally request an active connectivity check.", r#"{"type":"object","properties":{"active_check":{"type":"boolean"}},"additionalProperties":false}"#),
];

fn cpu_ticks() -> Option<(u64, u64)> {
    let text = read("/proc/stat", 4096)?;
    let line = text.lines().next()?.strip_prefix("cpu ")?;
    let ticks: Vec<u64> = line.split_whitespace().take(8).map(str::parse).collect::<std::result::Result<_, _>>().ok()?;
    if ticks.len() < 5 { return None; }
    let total = ticks.iter().sum();
    Some((total, ticks[3] + ticks[4]))
}

async fn cpu_usage() -> Option<f64> {
    let before = cpu_ticks()?;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let after = cpu_ticks()?;
    let total = after.0.checked_sub(before.0)?;
    let idle = after.1.checked_sub(before.1)?;
    (total > 0).then(|| (100.0 * (total.saturating_sub(idle)) as f64 / total as f64).clamp(0.0,100.0))
}

fn read(path: &str, limit: usize) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    (data.len() <= limit).then(|| String::from_utf8_lossy(&data).into_owned())
}

fn meminfo() -> Value {
    let Some(text) = read("/proc/meminfo", 16 * 1024) else { return Value::Null };
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        if let Some(n) = line.strip_prefix("MemTotal:") { total = n.split_whitespace().next().and_then(|v| v.parse::<u64>().ok()); }
        if let Some(n) = line.strip_prefix("MemAvailable:") { available = n.split_whitespace().next().and_then(|v| v.parse::<u64>().ok()); }
    }
    json!({"total_bytes": total.map(|n| n * 1024), "available_bytes": available.map(|n| n * 1024)})
}

fn filesystem(path: &Path) -> Option<Value> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let cpath = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(cpath.as_ptr(), &mut stat) } != 0 { return None; }
    let block = stat.f_frsize as u64;
    Some(json!({"mount": path, "total_bytes": (stat.f_blocks as u64).saturating_mul(block), "available_bytes": (stat.f_bavail as u64).saturating_mul(block), "readonly": stat.f_flag & libc::ST_RDONLY as u64 != 0}))
}

fn temperatures() -> Vec<Value> {
    let Ok(zones) = std::fs::read_dir("/sys/class/thermal") else { return Vec::new() };
    zones.flatten().filter_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("thermal_zone") { return None; }
        let root = entry.path();
        let milli = std::fs::read_to_string(root.join("temp")).ok()?.trim().parse::<i64>().ok()?;
        if !(-40_000..=150_000).contains(&milli) { return None; }
        let kind = std::fs::read_to_string(root.join("type")).unwrap_or(name);
        Some(json!({"sensor":kind.trim(),"celsius":milli as f64 / 1000.0}))
    }).take(32).collect()
}

fn bounded(text: String, bytes: usize) -> String {
    if text.len() <= bytes { return text; }
    let mut end = bytes;
    while !text.is_char_boundary(end) { end -= 1; }
    format!("{}\n[truncated]", &text[..end])
}

fn scope(args: &Value) -> Result<&str> {
    let value = args["scope"].as_str().unwrap_or("user");
    ensure!(matches!(value, "user" | "system"), "invalid service scope");
    Ok(value)
}

async fn optional(program: &str, args: &[&str], bytes: usize) -> Value {
    match run(program, args).await {
        Ok(value) => json!({"available":true,"output":bounded(value,bytes)}),
        Err(err) => json!({"available":false,"reason":err.to_string()}),
    }
}

pub async fn call(name: &str, args: &Value) -> Option<Result<Value>> {
    Some(match name {
        "diagnostics.snapshot" => async {
            let load = read("/proc/loadavg", 256).and_then(|s| {
                let values: Vec<f64> = s.split_whitespace().take(3).filter_map(|v| v.parse().ok()).collect();
                (values.len() == 3).then_some(values)
            });
            let home = dirs_home();
            let filesystems = [Path::new("/"), home.as_path()].into_iter().filter_map(filesystem).collect::<Vec<_>>();
            let gpu = optional("nvidia-smi", &["--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu", "--format=csv,noheader,nounits"], 4096).await;
            let processes = match run("ps", &["-eo", "pid,comm,%cpu,%mem,rss", "--sort=-%cpu"]).await {
                Ok(output) => json!({"available":true,"top_cpu":output.lines().take(21).collect::<Vec<_>>()}),
                Err(err) => json!({"available":false,"reason":err.to_string()}),
            };
            let network = optional("nmcli", &["-t", "-f", "STATE,CONNECTIVITY", "general"], 1024).await;
            let routes = optional("ip", &["-j", "route", "show", "default"], 4096).await;
            Ok(json!({"at_ms":SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),"cpu_percent":cpu_usage().await,"cpu_load_1_5_15":load,"ram":meminfo(),"gpu":gpu,"temperatures":temperatures(),"filesystems":filesystems,"processes":processes,"network":network,"default_routes":routes}))
        }.await,
        "diagnostics.service_status" => async {
            let unit = args["unit"].as_str().context("unit required")?;
            ensure!(valid_unit(unit), "invalid unit name");
            let scope = scope(args)?;
            let mut argv = vec!["show", unit, "-p", "LoadState,ActiveState,SubState,UnitFileState,Description,Result,ExecMainStatus"];
            if scope == "user" { argv.insert(0, "--user"); }
            let output = run("systemctl", &argv).await?;
            Ok(json!({"unit":unit,"scope":scope,"status":bounded(output,8192)}))
        }.await,
        "diagnostics.journal" => async {
            let unit = args["unit"].as_str().context("unit required")?;
            ensure!(valid_unit(unit), "invalid unit name");
            let scope = scope(args)?;
            let lines = args["lines"].as_u64().unwrap_or(40).clamp(1,100).to_string();
            let mut argv = vec!["--no-pager", "--output=short-iso", "-u", unit, "-n", lines.as_str()];
            if scope == "user" { argv.insert(0, "--user"); }
            let output = run("journalctl", &argv).await?;
            Ok(json!({"unit":unit,"scope":scope,"lines":bounded(output,64*1024)}))
        }.await,
        "diagnostics.network" => async {
            let active = args["active_check"] == true;
            let check = if active { optional("nmcli", &["networking", "connectivity", "check"], 1024).await } else { optional("nmcli", &["networking", "connectivity"], 1024).await };
            Ok(json!({"active_check":active,"connectivity":check,"devices":optional("nmcli", &["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"], 4096).await,"default_routes":optional("ip", &["-j", "route", "show", "default"], 4096).await,"dns":optional("resolvectl", &["dns"], 4096).await}))
        }.await,
        _ => return None,
    })
}

fn dirs_home() -> std::path::PathBuf { std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| "/".into()) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_output_and_schemas() {
        assert_eq!(bounded("ééé".into(),3), "é\n[truncated]");
        for (_,_,schema) in TOOLS { serde_json::from_str::<Value>(schema).unwrap(); }
    }
}
