//! Machine readings for `perf`, Settings and the benches: the NVIDIA card (asleep or its
//! temperature, power, use and memory), the CPU package temperature, the power source and a
//! process's CPU time and memory. All from sysfs and `/proc`, plus `nvidia-smi` only while the
//! card is awake: querying it would wake a runtime-suspended card.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// The NVIDIA display controller's sysfs folder.
pub fn nvidia_device() -> Option<PathBuf> {
    std::fs::read_dir("/sys/bus/pci/devices").ok()?.flatten().map(|e| e.path()).find(|d| {
        read(d.join("vendor")).as_deref() == Some("0x10de") && read(d.join("class")).is_some_and(|c| c.starts_with("0x03"))
    })
}

/// `active` or `suspended` (runtime power management), if the card has it.
pub fn gpu_runtime_status() -> Option<String> {
    read(nvidia_device()?.join("power/runtime_status"))
}

/// The card's readings, and how much memory `pid` holds on it. Never wakes a sleeping card.
pub fn gpu(pid: Option<u64>) -> Value {
    let status = gpu_runtime_status();
    if status.as_deref() == Some("suspended") {
        return json!({"asleep": true, "runtime_status": status});
    }
    let run = |args: &[&str]| {
        std::process::Command::new("nvidia-smi")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    };
    let Some(out) = run(&[
        "--query-gpu=name,memory.used,memory.total,temperature.gpu,power.draw,utilization.gpu,pstate",
        "--format=csv,noheader,nounits",
    ]) else {
        return Value::Null;
    };
    let f: Vec<&str> = out.lines().next().unwrap_or("").split(", ").map(str::trim).collect();
    let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok());
    let mine = pid.and_then(|pid| {
        run(&["--query-compute-apps=pid,used_memory", "--format=csv,noheader,nounits"])?
            .lines()
            .filter_map(|l| l.split_once(", "))
            .find(|(p, _)| p.trim().parse::<u64>().ok() == Some(pid))
            .and_then(|(_, m)| m.trim().parse::<u64>().ok())
    });
    json!({
        "asleep": false,
        "runtime_status": status,
        "name": f.first().unwrap_or(&""),
        "used_mib": num(1).map(|v| v as u64),
        "total_mib": num(2).map(|v| v as u64),
        "assistant_mib": mine,
        "temp_c": num(3),
        "power_w": num(4),
        "util_pct": num(5),
        "pstate": f.get(6),
    })
}

/// The CPU package temperature (`coretemp`, "Package id 0"), in °C.
pub fn cpu_temp_c() -> Option<f64> {
    for dir in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten().map(|e| e.path()) {
        if read(dir.join("name")).as_deref() != Some("coretemp") {
            continue;
        }
        for i in 1..=64 {
            if read(dir.join(format!("temp{i}_label"))).is_some_and(|l| l.starts_with("Package")) {
                return read(dir.join(format!("temp{i}_input")))?.parse::<f64>().ok().map(|m| m / 1000.0);
            }
        }
    }
    None
}

/// Whether the machine runs on battery: a mains supply exists and none is online.
pub fn on_battery() -> Option<bool> {
    let mains: Vec<PathBuf> = std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|d| read(d.join("type")).as_deref() == Some("Mains"))
        .collect();
    if mains.is_empty() {
        return None;
    }
    Some(!mains.iter().any(|d| read(d.join("online")).as_deref() == Some("1")))
}

/// A process's CPU time (ms, user + system) and resident memory (MiB).
pub fn process(pid: u64) -> Value {
    let stat = read(format!("/proc/{pid}/stat")).unwrap_or_default();
    // Fields after the command name, which is in parentheses and may hold spaces.
    let rest: Vec<&str> = stat.rsplit_once(')').map(|(_, r)| r.split_whitespace().collect()).unwrap_or_default();
    let ticks = |i: usize| rest.get(i).and_then(|v| v.parse::<u64>().ok());
    // utime and stime are fields 14 and 15 of the whole line; 12 and 13 after the name.
    let cpu_ms = ticks(11).zip(ticks(12)).map(|(u, s)| (u + s) * 1000 / clock_ticks());
    let rss_mib = read(format!("/proc/{pid}/status")).and_then(|s| {
        s.lines().find_map(|l| l.strip_prefix("VmRSS:")).and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
    });
    json!({"cpu_ms": cpu_ms, "rss_mib": rss_mib.map(|k| k / 1024)})
}

fn clock_ticks() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let t = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if t > 0 {
        t as u64
    } else {
        100
    }
}

/// Everything above in one object, for `perf`.
pub fn snapshot(pid: Option<u64>) -> Value {
    json!({
        "gpu": gpu(pid),
        "cpu_temp_c": cpu_temp_c(),
        "on_battery": on_battery(),
        "process": pid.map(process),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_process() {
        let me = process(std::process::id() as u64);
        assert!(me["rss_mib"].as_u64().is_some());
        assert!(me["cpu_ms"].as_u64().is_some());
        assert!(process(u64::MAX)["cpu_ms"].is_null());
    }
}
