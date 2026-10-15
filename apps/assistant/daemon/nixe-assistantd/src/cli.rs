#[path = "bench.rs"]
mod bench;

use anyhow::Context;
use assistant_ipc::{
    request, send_and_listen, send_command, send_until_done, stream_events, Command, Control,
};
use std::io::{self, IsTerminal, Write};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".into());
    match cmd.as_str() {
        "plugins" => {
            use assistant_tools::plugins;
            let action = args.next().unwrap_or_else(|| "list".into());
            let rest: Vec<_> = args.collect();
            if action == "migrate" {
                let cfg = assistant_core::Config::load()?;
                let worker =
                    std::env::current_exe()?.with_file_name("nixe-assistant-plugin-worker");
                let checkout = rest
                    .first()
                    .map(std::path::PathBuf::from)
                    .unwrap_or(std::env::current_dir()?);
                plugins::migrate(&cfg, &worker, &checkout).await?;
                println!("Plugin migration complete; existing configuration retained.");
                return Ok(());
            }
            if action == "install" || action == "update" {
                let source = rest
                    .first()
                    .ok_or_else(|| anyhow::anyhow!("package source required"))?;
                let revision = rest
                    .iter()
                    .position(|a| a == "--revision")
                    .and_then(|i| rest.get(i + 1))
                    .cloned();
                let source_for_stage = source.clone();
                let staged = tokio::task::spawn_blocking(move || {
                    plugins::stage(&source_for_stage, revision.as_deref())
                })
                .await??;
                let manifest = plugins::Manifest::read(&staged.path().join("package"))?;
                println!("Source: {source}\nStartup: {} {:?}\nTools: {:?}\nDependencies: {:?}\nRuns with your account permissions. MCP approval is not an OS sandbox.\nNew third-party packages start disabled.",manifest.executable,manifest.args,manifest.tools,manifest.dependencies);
                if !rest.iter().any(|a| a == "--yes") {
                    anyhow::ensure!(
                        io::stdin().is_terminal(),
                        "review the package above and repeat with --yes"
                    );
                    eprint!("Install this reviewed package? [y/N] ");
                    io::stderr().flush()?;
                    let mut answer = String::new();
                    io::stdin().read_line(&mut answer)?;
                    anyhow::ensure!(
                        answer.trim().eq_ignore_ascii_case("y"),
                        "installation cancelled"
                    );
                }
                // Retain the installed package for rollback if live activation fails.
                let target = plugins::package_dir().join(&manifest.id);
                let backup = tempfile::tempdir()?;
                if action == "update" {
                    anyhow::ensure!(
                        std::process::Command::new("cp")
                            .args(["-a", "--"])
                            .arg(&target)
                            .arg(backup.path().join("previous"))
                            .status()?
                            .success(),
                        "could not back up existing package; update cancelled"
                    );
                }
                let old_state = plugins::saved()?;
                plugins::install(&staged.path().join("package"), source, action == "update")?;
                if action == "update" && old_state.get(&manifest.id).is_some_and(|s| s.enabled) {
                    if let Err(e) = request(
                        Control::Plugins {
                            action: "reload".into(),
                            id: Some(manifest.id.clone()),
                        },
                        30_000,
                    )
                    .await
                    {
                        std::fs::remove_dir_all(&target)?;
                        std::fs::rename(backup.path().join("previous"), &target)?;
                        plugins::save(&old_state)?;
                        return Err(e.context("update rolled back; prior package retained"));
                    }
                }
                println!("Installed {} {}", manifest.id, manifest.version);
                return Ok(());
            }
            if action == "list" || action == "inspect" {
                let value = match request(
                    Control::Plugins {
                        action: action.clone(),
                        id: None,
                    },
                    2000,
                )
                .await
                {
                    Ok(v) => v,
                    Err(_) => plugins::PluginManager::new().inspect(),
                };
                let value = if action == "inspect" {
                    let id = rest.first().context("plugin ID required")?;
                    value
                        .as_array()
                        .and_then(|a| a.iter().find(|p| p["id"] == id.as_str()))
                        .cloned()
                        .context("plugin not installed")?
                } else {
                    value
                };
                println!("{}", serde_json::to_string_pretty(&value)?);
                return Ok(());
            }
            anyhow::ensure!(
                ["enable", "disable", "reload", "remove"].contains(&action.as_str()),
                "unknown plugins command"
            );
            let id = rest.first().context("plugin ID required")?.clone();
            println!(
                "{}",
                request(
                    Control::Plugins {
                        action,
                        id: Some(id)
                    },
                    30_000
                )
                .await?
            );
        }
        "tool" => {
            let name = args.next().context("tool name required")?;
            let arguments = serde_json::from_str(&args.collect::<Vec<_>>().join(" "))?;
            println!(
                "{}",
                request(Control::ToolCall { name, arguments }, 90_000).await?
            );
        }
        "search-provider" => {
            let provider = args.next().unwrap_or_else(|| "show".into());
            let mut cfg = assistant_core::Config::load()?;
            match provider.as_str() {
                "show" => {
                    println!(
                        "{}",
                        if cfg.browser.search_tool.is_empty() {
                            "built-in"
                        } else {
                            &cfg.browser.search_tool
                        }
                    );
                    return Ok(());
                }
                "builtin" => cfg.browser.search_tool.clear(),
                "duckduckgo" => {
                    let path =
                        assistant_core::paths::data_dir().join("search/duckduckgo/dist/index.js");
                    anyhow::ensure!(path.is_file(), "run scripts/install-search.sh first");
                    let server = assistant_core::config::McpServerCfg {
                        command: cfg.browser.node.clone(),
                        args: vec![path.to_string_lossy().into()],
                        env: Default::default(),
                    };
                    if let Some(existing) = cfg.mcp.servers.get("duckduckgo") {
                        anyhow::ensure!(existing.command==server.command && existing.args==server.args, "an existing duckduckgo server has a different command; configure browser.search_tool manually");
                    } else {
                        cfg.mcp.servers.insert("duckduckgo".into(), server);
                    }
                    cfg.mcp.enabled = true;
                    cfg.mcp
                        .permissions
                        .entry("duckduckgo.search".into())
                        .or_insert_with(|| "allow".into());
                    cfg.browser.search_tool = "duckduckgo.search".into();
                }
                _ => anyhow::bail!("usage: search-provider show|builtin|duckduckgo"),
            }
            cfg.save()?;
            println!("Search provider saved. Restart the daemon to apply it.");
        }
        "location" => {
            let action = args.next().unwrap_or_else(|| "show".into());
            let city = if action == "set" {
                Some(args.collect::<Vec<_>>().join(" "))
            } else {
                None
            };
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &request(Control::Location { action, city }, 10_000).await?
                )?
            );
        }
        "conversation" => {
            let enabled = match args.next().as_deref() {
                Some("start" | "on") => true,
                Some("stop" | "off") => false,
                _ => anyhow::bail!("usage: conversation start|stop"),
            };
            println!(
                "{}",
                request(Control::Conversation { enabled }, 5000).await?
            );
        }
        "dictation" => {
            let action = args.next().unwrap_or_else(|| "start".into());
            println!("{}", request(Control::Dictation { action }, 5000).await?);
        }
        "browser-takeover" => println!("{}", request(Control::BrowserTakeover, 20_000).await?),
        "browser-resume" => println!("{}", request(Control::BrowserResume, 20_000).await?),
        "jobs" => {
            let action = args.next().unwrap_or_else(|| "status".into());
            let job_id = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: jobs status|follow|cancel ID"))?;
            if action == "cancel" {
                println!("{}", request(Control::JobCancel { job_id }, 2000).await?);
            } else {
                loop {
                    let job = request(
                        Control::JobStatus {
                            job_id: job_id.clone(),
                        },
                        2000,
                    )
                    .await?;
                    if action == "follow" {
                        print!("\x1b[2J\x1b[H");
                        println!(
                            "Job {}: {} · exit {}\n{}\n{}\n{}",
                            job_id,
                            job["status"],
                            job["exit_code"],
                            job["argv"],
                            job["cwd"],
                            job["output"]
                                .as_str()
                                .unwrap_or("")
                                .chars()
                                .filter(|c| !c.is_control() || matches!(c, '\n' | '\t' | '\r'))
                                .collect::<String>()
                        );
                    } else {
                        println!("{job}");
                    }
                    if action != "follow" || job["status"] != "running" {
                        if action == "follow" && io::stdin().is_terminal() {
                            eprintln!("Press Enter to close this job view.");
                            let mut line = String::new();
                            io::stdin().read_line(&mut line)?;
                        }
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
        "stt-daemon" => {
            use std::os::unix::process::CommandExt;
            let cfg = assistant_core::Config::load()?;
            let err = std::process::Command::new(&cfg.stt.voxtype_bin)
                .args(["-c"])
                .arg(assistant_core::paths::config_dir().join("voxtype-parakeet.toml"))
                .arg("daemon")
                .exec();
            return Err(err.into());
        }
        "ptt-start" => send_command(&Command::PttStart).await?,
        "ptt-stop" => send_command(&Command::PttStop).await?,
        "cancel" => send_command(&Command::Cancel).await?,
        "cancel-esc" => cancel_esc().await?,
        "reload" => println!("{}", request(Control::Reload, 30_000).await?),
        "doctor" => {
            let json_mode = args.any(|a| a == "--json");
            let data = match request(Control::Diagnostics, 15_000).await {
                Ok(value) => value,
                Err(_) => {
                    let mut value = assistant_core::diagnostics::collect().await;
                    if let Ok(cfg) = assistant_core::Config::load() {
                        let registry = assistant_tools::registry::Registry::load(&cfg).await;
                        value["mcp_tools"] = registry.catalog(&cfg);
                        value["mcp_errors"] = serde_json::json!(registry.errors);
                    }
                    value["daemon_online"] = false.into();
                    value
                }
            };
            println!(
                "{}",
                if json_mode {
                    data.to_string()
                } else {
                    serde_json::to_string_pretty(&data)?
                }
            );
        }
        "activity" => {
            let action = if args.next().as_deref() == Some("clear") {
                Control::ClearActivity
            } else {
                Control::Activity
            };
            println!("{}", request(action, 2000).await?);
        }
        "audit" => {
            let rest: Vec<_> = args.collect();
            let limit = rest
                .iter()
                .position(|a| a == "--limit")
                .and_then(|i| rest.get(i + 1))
                .map(|n| n.parse::<usize>())
                .transpose()?
                .unwrap_or(50);
            let path =
                assistant_policy::store::default_path(&assistant_core::paths::state_dir());
            let rows = assistant_policy::store::recent(&path, limit)?;
            if rest.iter().any(|a| a == "--json") {
                println!("{}", serde_json::to_string(&rows)?);
                return Ok(());
            }
            if rows.is_empty() {
                println!("No audit records in {}", path.display());
            }
            for r in &rows {
                let when = chrono::DateTime::from_timestamp_millis(r.at_ms as i64)
                    .map(|t| t.with_timezone(&chrono::Local).format("%F %T").to_string())
                    .unwrap_or_default();
                println!(
                    "{when}  {:<8} {:<9} {:<28} {:<8} {}{}  {}",
                    r.event,
                    r.outcome,
                    r.tool,
                    r.decision,
                    r.rule,
                    r.duration_ms.map(|d| format!(" {d}ms")).unwrap_or_default(),
                    r.args
                );
                if let Some(detail) = &r.detail {
                    println!("    {detail}");
                }
            }
        }
        "install-helper" => install_helper()?,
        "bench" if std::env::args().nth(2).as_deref() == Some("computer") => {
            bench::computer(std::env::args().nth(3)).await?;
        }
        "bench" if std::env::args().nth(2).as_deref() == Some("vision") => {
            match (std::env::args().nth(3).as_deref(), std::env::args().nth(4)) {
                (Some("--one"), Some(model)) => {
                    bench::vision_one(std::path::Path::new(&model), &std::env::args().nth(5).unwrap_or("tags".into())).await?
                }
                _ => bench::vision().await?,
            }
        }
        "bench" if std::env::args().nth(2).as_deref() == Some("duplex") => {
            let live = !std::env::args().any(|a| a == "--echo-only");
            bench_duplex(live).await?;
        }
        // Bare `models` opens the TUI's model page (below).
        "models" if std::env::args().nth(2).as_deref() == Some("fetch") => {
            let rest: Vec<_> = args.collect();
            anyhow::ensure!(
                rest.iter().map(String::as_str).eq(["fetch", "asr"]),
                "usage: nixe-assistant models fetch asr"
            );
            fetch_asr_model()?;
        }
        "admin-log" => {
            let rest: Vec<_> = args.collect();
            let limit = rest
                .iter()
                .position(|a| a == "--limit")
                .and_then(|i| rest.get(i + 1))
                .map(|n| n.parse::<usize>())
                .transpose()?
                .unwrap_or(50);
            let text = match std::fs::read_to_string(assistant_privileged::LOG) {
                Ok(text) => text,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    println!("No admin actions yet ({})", assistant_privileged::LOG);
                    return Ok(());
                }
                Err(e) => return Err(e).context(assistant_privileged::LOG),
            };
            let lines: Vec<&str> = text.lines().collect();
            for line in &lines[lines.len().saturating_sub(limit)..] {
                let Ok(r) = serde_json::from_str::<serde_json::Value>(line) else { continue };
                let when = chrono::DateTime::from_timestamp_millis(r["at_ms"].as_i64().unwrap_or(0))
                    .map(|t| t.with_timezone(&chrono::Local).format("%F %T").to_string())
                    .unwrap_or_default();
                let args: Vec<&str> =
                    r["args"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
                println!(
                    "{when}  {:<6} {:>7}ms  {}",
                    if r["ok"] == true { "ok" } else { "failed" },
                    r["duration_ms"].as_u64().unwrap_or(0),
                    args.join(" ")
                );
                if let Some(e) = r["error"].as_str() {
                    println!("    {e}");
                }
            }
        }
        "tasks" => {
            let rest: Vec<_> = args.collect();
            let path =
                assistant_policy::store::default_path(&assistant_core::paths::state_dir());
            let json_mode = rest.iter().any(|a| a == "--json");
            let when = |ms: u64| {
                chrono::DateTime::from_timestamp_millis(ms as i64)
                    .map(|t| t.with_timezone(&chrono::Local).format("%F %T").to_string())
                    .unwrap_or_default()
            };
            match rest.first().map(String::as_str) {
                Some("cancel") => {
                    let task_id = rest.get(1).context("task ID required")?.clone();
                    println!("{}", request(Control::TaskCancel { task_id }, 5000).await?);
                }
                Some("show") => {
                    let id = rest.get(1).context("task ID required")?;
                    let (task, steps) = assistant_policy::store::task(&path, id)?
                        .with_context(|| format!("no task {id}"))?;
                    if json_mode {
                        println!("{}", serde_json::json!({"task":task,"steps":steps}));
                    } else {
                        println!(
                            "{}  {}  {}/{} calls  {:.1}s\n{}{}",
                            task.id,
                            task.state,
                            task.calls,
                            task.max_calls,
                            task.active_ms as f64 / 1000.0,
                            task.request,
                            task.detail.map(|d| format!("\n  {d}")).unwrap_or_default()
                        );
                        for s in steps {
                            println!(
                                "  {}  {:<28} {:<11} {}",
                                s.index, s.tool, s.verification, s.summary
                            );
                        }
                    }
                }
                _ => {
                    let limit = rest
                        .iter()
                        .position(|a| a == "--limit")
                        .and_then(|i| rest.get(i + 1))
                        .map(|n| n.parse::<usize>())
                        .transpose()?
                        .unwrap_or(20);
                    let rows = assistant_policy::store::tasks(&path, limit)?;
                    if json_mode {
                        println!("{}", serde_json::to_string(&rows)?);
                    } else if rows.is_empty() {
                        println!("No tasks in {}", path.display());
                    }
                    for t in rows.iter().filter(|_| !json_mode) {
                        let request: String = t.request.chars().take(60).collect();
                        println!(
                            "{}  {}  {:<20} {}/{}  {request}",
                            when(t.created_ms),
                            t.id,
                            t.state,
                            t.calls,
                            t.max_calls
                        );
                    }
                }
            }
        }
        "preview" => {
            let cfg = assistant_core::Config::load()?;
            let voice = args.next().unwrap_or(cfg.tts.voice);
            let speed = args
                .next()
                .map(|s| s.parse::<f32>())
                .transpose()?
                .unwrap_or(cfg.tts.speed);
            println!(
                "{}",
                request(Control::Preview { voice, speed }, 30_000).await?
            );
        }
        "calibrate" => {
            eprintln!("Five seconds of quiet, then five seconds of normal speech. HUD shows each phase. Suggestions are not saved.");
            println!("{}", request(Control::Calibrate, 30_000).await?);
        }
        "tool-mode" => {
            let enabled = match args.next().as_deref() {
                Some("on") => true,
                Some("off") => false,
                _ => anyhow::bail!("usage: tool-mode on|off"),
            };
            println!("{}", request(Control::ToolMode { enabled }, 2000).await?);
        }
        "approve" | "deny" => {
            let approval_id = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("approval ID required"))?;
            let scope = if args.any(|a| a == "--task") {
                assistant_ipc::ApprovalScope::Task
            } else {
                assistant_ipc::ApprovalScope::Once
            };
            println!(
                "{}",
                request(
                    Control::Approve {
                        approval_id,
                        allow: cmd == "approve",
                        scope,
                    },
                    2000
                )
                .await?
            );
        }
        "migrate" => {
            let mut cfg = assistant_core::Config::load()?;
            if cfg.assistant.persona_id.is_empty() {
                if let Some(p) = assistant_core::Persona::load_all()?
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(&cfg.assistant.name))
                {
                    cfg.assistant.persona_id = p.id.clone();
                }
            }
            cfg.save()?;
        }
        "shutdown" => send_command(&Command::Shutdown).await?,
        "status" if args.next().as_deref() == Some("--json") => {
            println!("{}", request(Control::Status, 1500).await?)
        }
        "status" => {
            let events = send_and_listen(Some(Command::Status), 400).await?;
            print_events(events);
        }
        "listen" => {
            stream_events(Some(Command::Status), |ev| {
                print_event(&ev);
            })
            .await?;
        }
        "say" => {
            let text = args.collect::<Vec<_>>().join(" ");
            if text.trim().is_empty() {
                anyhow::bail!("usage: nixe-assistant say <text>");
            }
            print_events(send_until_done(Command::Say { text }, 120_000).await?);
        }
        "demo" => {
            print_events(send_until_done(Command::Demo, 180_000).await?);
        }
        "mute" => {
            send_command(&Command::Mute { muted: Some(true) }).await?;
            println!("Assistant muted.");
        }
        "unmute" => {
            send_command(&Command::Mute { muted: Some(false) }).await?;
            println!("Assistant unmuted.");
        }
        "toggle-mute" => {
            send_command(&Command::Mute { muted: None }).await?;
            println!("Assistant mute toggled.");
        }
        "tui" | "ui" | "settings" | "assistant-tui" | "model" | "models" => {
            let tui_bin = dirs::home_dir()
                .map(|h| h.join(".local/bin/nixe-assistant-tui"))
                .unwrap_or_else(|| std::path::PathBuf::from("nixe-assistant-tui"));
            let bin_path = if tui_bin.exists() {
                tui_bin
            } else {
                std::path::PathBuf::from("nixe-assistant-tui")
            };
            let status = std::process::Command::new(bin_path).status()?;
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        "start" | "enable" => {
            let status = std::process::Command::new("systemctl")
                .args([
                    "--user",
                    "start",
                    "nixe-assistant-stt.service",
                    "nixe-assistantd.service",
                ])
                .status()?;
            if status.success() {
                println!("Started assistant daemon & loaded models.");
            } else {
                eprintln!("Failed to start services.");
                std::process::exit(1);
            }
        }
        "stop" | "disable" => {
            let status = std::process::Command::new("systemctl")
                .args([
                    "--user",
                    "stop",
                    "nixe-assistantd.service",
                    "nixe-assistant-stt.service",
                ])
                .status()?;
            if status.success() {
                println!("Stopped assistant daemon & unloaded models from VRAM/RAM.");
            } else {
                eprintln!("Failed to stop services.");
                std::process::exit(1);
            }
        }
        "toggle-service" | "toggle-daemon" => {
            let is_active = std::process::Command::new("systemctl")
                .args([
                    "--user",
                    "is-active",
                    "--quiet",
                    "nixe-assistantd.service",
                ])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if is_active {
                let status = std::process::Command::new("systemctl")
                    .args([
                        "--user",
                        "stop",
                        "nixe-assistantd.service",
                        "nixe-assistant-stt.service",
                    ])
                    .status()?;
                anyhow::ensure!(status.success(), "Failed to stop services");
                println!("Assistant disabled (models unloaded).");
            } else {
                let status = std::process::Command::new("systemctl")
                    .args([
                        "--user",
                        "start",
                        "nixe-assistant-stt.service",
                        "nixe-assistantd.service",
                    ])
                    .status()?;
                anyhow::ensure!(status.success(), "Failed to start services");
                println!("Assistant enabled (models loaded).");
            }
        }
        "persona" | "personas" => {
            let sub = args.next();
            match sub.as_deref() {
                Some("set") | Some("use") | Some("switch") => {
                    let target = args.next().unwrap_or_default();
                    if target.is_empty() {
                        eprintln!("usage: nixe-assistant persona set <name>");
                        std::process::exit(1);
                    }
                    let personas = assistant_core::Persona::load_all()?;
                    if let Some(p) = personas.iter().find(|p| {
                        p.name.eq_ignore_ascii_case(&target) || p.id.eq_ignore_ascii_case(&target)
                    }) {
                        let mut cfg = assistant_core::Config::load()?;
                        cfg.activate_persona(p);
                        cfg.save()?;
                        assistant_core::sync_plugin_manifest(p)?;
                        println!("{}", request(Control::Reload, 30_000).await?);
                        println!(
                            "Activated persona: {} (voice: {}, {} aliases)",
                            p.name,
                            p.voice,
                            p.aliases.len()
                        );
                    } else {
                        eprintln!("Persona '{}' not found. Available personas:", target);
                        for p in personas {
                            println!("  - {} (id: {})", p.name, p.id);
                        }
                        std::process::exit(1);
                    }
                }
                Some("new") | Some("create") => {
                    let name = args.next().unwrap_or_default();
                    if name.is_empty() {
                        eprintln!("usage: nixe-assistant persona new <name>");
                        std::process::exit(1);
                    }
                    let p = assistant_core::Persona::new_with_generated(&name);
                    if assistant_core::paths::personas_dir()
                        .join(format!("{}.toml", p.id))
                        .exists()
                    {
                        anyhow::bail!("persona ID already exists");
                    }
                    p.save()?;
                    println!(
                        "Created persona '{}' (id: {}) with {} generated aliases.",
                        p.name,
                        p.id,
                        p.aliases.len()
                    );
                }
                Some("aliases") => {
                    let target = args.next();
                    let personas = assistant_core::Persona::load_all()?;
                    let p = if let Some(t) = target {
                        personas.iter().find(|p| {
                            p.name.eq_ignore_ascii_case(&t) || p.id.eq_ignore_ascii_case(&t)
                        })
                    } else {
                        let cfg = assistant_core::Config::load()?;
                        personas.iter().find(|p| {
                            p.name.eq_ignore_ascii_case(&cfg.assistant.name)
                                || p.id.eq_ignore_ascii_case(&cfg.assistant.name)
                        })
                    };
                    if let Some(p) = p {
                        println!("Aliases for persona {}:", p.name);
                        for (i, a) in p.aliases.iter().enumerate() {
                            println!("  {:02}: {}", i + 1, a);
                        }
                    } else {
                        eprintln!("Persona not found.");
                        std::process::exit(1);
                    }
                }
                Some("add-alias") => {
                    let alias = args.collect::<Vec<_>>().join(" ");
                    if alias.trim().is_empty() {
                        eprintln!("usage: nixe-assistant persona add-alias <alias>");
                        std::process::exit(1);
                    }
                    let mut personas = assistant_core::Persona::load_all()?;
                    let mut cfg = assistant_core::Config::load()?;
                    let pos = personas.iter().position(|p| {
                        p.name.eq_ignore_ascii_case(&cfg.assistant.name)
                            || p.id.eq_ignore_ascii_case(&cfg.assistant.name)
                    });
                    if let Some(idx) = pos {
                        let p = &mut personas[idx];
                        let clean = alias.trim().to_lowercase();
                        if !p.aliases.contains(&clean) {
                            p.aliases.push(clean.clone());
                            p.save()?;
                            cfg.wake_word.phrases = p.aliases.clone();
                            cfg.save()?;
                            println!("{}", request(Control::Reload, 30_000).await?);
                            println!("Added alias '{}' to active persona {}.", clean, p.name);
                        } else {
                            println!("Alias '{}' already exists for {}.", clean, p.name);
                        }
                    }
                }
                _ => {
                    let personas = assistant_core::Persona::load_all()?;
                    let cfg = assistant_core::Config::load()?;
                    println!("Saved Assistant Personas:");
                    for p in personas {
                        let is_active = p.name.eq_ignore_ascii_case(&cfg.assistant.name)
                            || p.id.eq_ignore_ascii_case(&cfg.assistant.name);
                        let badge = if is_active { "[ACTIVE] " } else { "         " };
                        println!(
                            "  {}{} (id: {:<7} voice: {:<11} aliases: {})",
                            badge,
                            p.name,
                            p.id,
                            p.voice,
                            p.aliases.len()
                        );
                    }
                    println!("\nCommands: nixe-assistant persona set <name>");
                    println!("          nixe-assistant persona new <name>");
                    println!("          nixe-assistant persona aliases [name]");
                    println!("          nixe-assistant persona add-alias <alias>");
                }
            }
        }
        "help" | "-h" | "--help" => print_help(),
        other => {
            eprintln!("unknown command: {other}");
            print_help();
            std::process::exit(1);
        }
    }
    Ok(())
}

fn print_event(ev: &assistant_core::Event) {
    let mut out = io::stdout();
    if let Ok(line) = serde_json::to_string(ev) {
        let _ = writeln!(out, "{line}");
    }
    let _ = out.flush();
}

fn print_events(events: Vec<assistant_core::Event>) {
    for ev in events {
        print_event(&ev);
    }
}

async fn cancel_esc() -> anyhow::Result<()> {
    let events = send_and_listen(Some(Command::Status), 80).await?;
    let busy = events.iter().any(|ev| match ev {
        assistant_core::Event::Health { state, .. }
        | assistant_core::Event::State { state, .. } => !matches!(
            state,
            assistant_core::AssistantState::Idle | assistant_core::AssistantState::Interrupted
        ),
        _ => false,
    });
    if busy {
        send_command(&Command::Cancel).await?;
        return Ok(());
    }
    let status = std::process::Command::new("hyprctl")
        .args(["dispatch", "sendshortcut", ",Escape,"])
        .status()?;
    anyhow::ensure!(status.success(), "Failed to dispatch Escape");
    Ok(())
}

/// Installs the root helper for the admin tools and its polkit policy, from the binary
/// next to this one. Run with sudo.
fn install_helper() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    anyhow::ensure!(
        unsafe { libc::geteuid() } == 0,
        "run this as root: sudo {} install-helper",
        std::env::current_exe()?.display()
    );
    let source = std::env::current_exe()?.canonicalize()?.with_file_name("nixe-assistant-helper");
    anyhow::ensure!(source.is_file(), "{} not found next to nixe-assistant", source.display());
    let helper = Path::new(assistant_privileged::HELPER);
    let dir = helper.parent().context("no helper directory")?;
    std::fs::create_dir_all(dir)?;
    // The directory must be root's too, or its owner could swap the helper out later.
    std::os::unix::fs::chown(dir, Some(0), Some(0))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))?;
    let status = std::process::Command::new("install")
        .args(["-m", "755", "-o", "root", "-g", "root"])
        .arg(&source)
        .arg(helper)
        .status()?;
    anyhow::ensure!(status.success(), "could not install the helper");
    std::fs::write(assistant_privileged::POLICY, assistant_privileged::POLICY_XML)?;
    std::fs::set_permissions(assistant_privileged::POLICY, std::fs::Permissions::from_mode(0o644))?;
    println!("Installed {} and its polkit policy.", helper.display());
    Ok(())
}

/// One long reply, with the interruption events and first-audio time it produced.
async fn bench_turn(prompt: &str) -> anyhow::Result<(Vec<(String, String, u64, u64)>, Option<u64>)> {
    use assistant_core::event::Event;
    let events = send_until_done(Command::Say { text: prompt.into() }, 240_000).await?;
    let mut seen = Vec::new();
    let mut first_audio = None;
    for ev in events {
        match ev {
            Event::Interruption { verdict, text, decide_ms, react_us } => seen.push((verdict, text, decide_ms, react_us)),
            Event::Timing { total_ms, .. } => first_audio = first_audio.or(Some(total_ms)),
            _ => {}
        }
    }
    Ok((seen, first_audio))
}

/// `bench duplex`: false interruptions with nobody talking, then prompted live trials.
/// Every number comes from the daemon's events; nothing is estimated.
async fn bench_duplex(live: bool) -> anyhow::Result<()> {
    // About eight seconds of speech: time enough to speak over, short enough to repeat.
    const COUNT: &str = "Count slowly from one to twelve in words, one number per sentence, and say nothing else.";
    const ECHO: [&str; 2] = [COUNT, "Describe a lighthouse at night in three short sentences."];
    let notify = |text: &str| {
        let _ = std::process::Command::new("notify-send").args(["-u", "critical", "-t", "6000", "Duplex bench", text]).status();
    };
    println!("## Echo: two short replies, nobody talking\n");
    let (mut onsets, mut cut, mut audio) = (0, 0, Vec::new());
    for (i, prompt) in ECHO.iter().enumerate() {
        let i = i + 1;
        notify("Stay quiet while she talks.");
        let (seen, first) = bench_turn(prompt).await?;
        onsets += seen.len();
        cut += seen.iter().filter(|s| s.0 == "stop" || s.0 == "redirect").count();
        audio.extend(first);
        println!("reply {i}: {} onsets, verdicts {:?}, first audio {}", seen.len(), seen.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), first.map(|f| format!("{f} ms")).unwrap_or("not measured".into()));
    }
    println!("\nFalse onsets: {onsets}; false interruptions: {cut} (target 0)\n");
    if !live {
        return Ok(());
    }
    anyhow::ensure!(io::stdin().is_terminal(), "live trials need a terminal (or pass --echo-only)");
    println!("## Live: she counts to twelve; say the phrase shown once she has started\n");
    println!("| Said | Verdicts | Pause/duck after onset | Decided after onset |\n|---|---|---|---|");
    for (said, expect) in [("mm-hm", "backchannel"), ("stop", "stop"), ("actually, what's the weather in Paris?", "redirect")] {
        eprint!("Next: say \"{said}\" (expect {expect}). Press Enter to start… ");
        io::stderr().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        notify(&format!("When she starts speaking, say: {said}"));
        let (seen, _) = bench_turn(COUNT).await?;
        if seen.is_empty() {
            println!("| {said} | (no onset heard) | not measured | not measured |");
        }
        for (verdict, text, decide_ms, react_us) in seen {
            println!("| {said} | {verdict} “{text}” | {:.2} ms | {decide_ms} ms |", react_us as f64 / 1000.0);
        }
        // Let a redirected or stopped reply settle before the next trial.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let _ = send_command(&Command::Cancel).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    if !audio.is_empty() {
        audio.sort();
        println!("\nFirst audio for the echo prompts (text turns, routing included): median {} ms of {}", audio[audio.len() / 2], audio.len());
    }
    Ok(())
}

/// Downloads the streaming speech model into the models folder (about 650 MB), through a
/// temporary folder so an interrupted download never looks installed.
fn fetch_asr_model() -> anyhow::Result<()> {
    use assistant_core::asr::{ENCODER, MODEL_NAME, MODEL_URL};
    let dir = assistant_core::paths::models_dir();
    let target = dir.join(MODEL_NAME);
    if target.join(ENCODER).is_file() {
        println!("Already installed: {}", target.display());
        return Ok(());
    }
    std::fs::create_dir_all(&dir)?;
    let tmp = tempfile::tempdir_in(&dir)?;
    println!("Downloading {MODEL_NAME} (about 650 MB; NVIDIA Open Model License)…");
    let archive = tmp.path().join("model.tar.bz2");
    let ok = std::process::Command::new("curl")
        .args(["-fL", "--progress-bar", "-o"])
        .arg(&archive)
        .arg(MODEL_URL)
        .status()?
        .success();
    anyhow::ensure!(ok, "the download failed");
    let ok = std::process::Command::new("tar").arg("xjf").arg(&archive).arg("-C").arg(tmp.path()).status()?.success();
    anyhow::ensure!(ok, "the archive could not be unpacked");
    let unpacked = tmp.path().join(MODEL_NAME);
    anyhow::ensure!(unpacked.join(ENCODER).is_file(), "the archive doesn't hold {MODEL_NAME}");
    let _ = std::fs::remove_dir_all(unpacked.join("test_wavs"));
    std::fs::rename(&unpacked, &target)?;
    println!("Installed {}. Restart the assistant to use it.", target.display());
    Ok(())
}

fn print_help() {
    eprintln!(
        "nixe-assistant — Local voice assistant client & persona manager\n\n\
           plugins list|inspect|install|enable|disable|reload|update|remove\n\
           tool NAME JSON  invoke through enabled-state and approval checks\n\
           doctor [--json] diagnostics (works offline)\n\
           activity [clear] latest 100 turns\n\
           audit [--limit N] [--json] tool decisions, approvals and calls (works offline)\n\
           admin-log [--limit N] packages and system services changed through the helper\n\
           install-helper  (as root) install the polkit helper for admin tools\n\
           models fetch asr  download the streaming speech model (about 650 MB)\n\
           bench duplex [--echo-only]  false interruptions and live interruption trials\n\
           bench computer [TASK]  computer-use tasks in real apps, each checked\n\
           bench vision     which vision model picks the right mark (Gemma vs GLM)\n\
           preview [voice speed] TTS only, without saving\n\
           calibrate      quiet/speech measurement; suggestions only\n\
           search-provider [show|builtin|duckduckgo]\n\
           location [show|refresh|set CITY|clear|on|off]\n\
           conversation start|stop, dictation start|pause|resume|stop|insert\n\
           browser-takeover, browser-resume, jobs status|follow|cancel ID\n\
           tool-mode on|off, approve ID [--task], deny ID\n\
           tasks [--limit N] [--json], tasks show ID, tasks cancel ID\n\
         Commands:\n\
           persona [set/new/aliases/add-alias]  manage saved assistant personas\n\
           ptt-start      start push-to-talk capture\n\
           ptt-stop       stop capture (key-up)\n\
           cancel         interrupt the current turn\n\
           cancel-esc     Escape: stop assistant, or pass Escape through when idle\n\
           mute           pause audio capture and wake detection\n\
           unmute         resume audio capture\n\
           toggle-mute    toggle capture mute state\n\
           start/enable   start daemon and load models into memory\n\
           stop/disable   stop daemon and unload models to free RAM/VRAM\n\
           toggle-service toggle daemon active/disabled state\n\
           tui            open interactive settings and model manager TUI\n\
           status         print a health event\n\
           say TEXT       skip STT; speak an LLM reply\n\
           demo           Kokoro → Parakeet → LLM → speakers\n\
           listen         print events until idle/timeout\n\
           reload         reread apps.toml, commands.toml, and config.toml\n\
           shutdown       stop the daemon\n"
    );
}
