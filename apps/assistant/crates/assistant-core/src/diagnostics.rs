//! Bounded dependency and desktop probes shared by CLI and daemon.
use serde_json::{json, Value};
use std::time::Duration;
pub async fn collect() -> Value {
    let config = crate::Config::load();
    let dependencies: Vec<Value> = [
        "voxtype",
        "wpctl",
        "systemctl",
        "hyprctl",
        "omarchy",
        "qs",
        "git",
        "wtype",
        "omarchy-assistant-node",
    ]
    .iter()
    .map(|name| {
        let found = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
            .unwrap_or(false);
        json!({"name":name,"available":found})
    })
    .collect();
    let mut checks = serde_json::Map::new();
    for (name, bin, args) in [
        (
            "services",
            "systemctl",
            vec![
                "--user",
                "is-active",
                "omarchy-assistantd",
                "nyx-parakeet",
                "omarchy-assistant-hud",
            ],
        ),
        ("audio_devices", "wpctl", vec!["status"]),
        (
            "recent_errors",
            "journalctl",
            vec![
                "--user",
                "-u",
                "omarchy-assistantd",
                "-p",
                "warning",
                "-n",
                "20",
                "--no-pager",
            ],
        ),
    ] {
        let mut command = tokio::process::Command::new(bin);
        command.args(args).kill_on_drop(true);
        let value = match tokio::time::timeout(Duration::from_secs(3), command.output()).await {
            Ok(Ok(out)) => {
                json!({"ok":out.status.success(),"output":String::from_utf8_lossy(&out.stdout),"error":String::from_utf8_lossy(&out.stderr)})
            }
            result => json!({"ok":false,"error":format!("{result:?}")}),
        };
        checks.insert(name.into(), value);
    }
    json!({"config": config.as_ref().map(|_| "valid".to_string()).unwrap_or_else(|e|e.to_string()),
        "dependencies":dependencies,"checks":checks,"models":crate::Config::scan_local_models(),
        "configured_model":config.as_ref().ok().map(|c|c.resolve_model_path()),
        "location_enabled":config.as_ref().ok().map(|c|c.location.enabled),
        "location_override":config.as_ref().ok().map(|c|&c.location.city),
        "browser_worker":crate::paths::data_dir().join("browser/worker.mjs").is_file(),
        "browser_assets":crate::paths::cache_dir().join("chromium").is_dir(),
        "mcp_enabled":config.as_ref().ok().map(|c|c.mcp.enabled)})
}
