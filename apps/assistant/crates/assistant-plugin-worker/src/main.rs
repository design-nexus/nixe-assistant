//! Independently replaceable executable: one process per bundled ability.
use anyhow::Result;
#[allow(dead_code)]
#[path = "../../assistant-agent/src/router.rs"]
mod router;
use assistant_core::{AppsFile, CommandsFile, Config};
use assistant_policy::Grant;
use assistant_tools::{mcp::McpTool, registry::Registry};
use router::{route_with, Intent};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
fn group(name: &str) -> &str {
    match name.split('.').next().unwrap_or("") {
        "agent" => "coding-agents",
        "system" if name == "system.status" => "system-status",
        "weather" => "weather",
        "location" => "location",
        "files" => "files",
        "git" => "git",
        "terminal" => "terminal",
        "apps" => "apps",
        "windows" | "workspaces" | "monitors" => "windows",
        "services" => "services",
        "network" => "network",
        "bluetooth" if name != "bluetooth.toggle" => "devices",
        "battery" => "devices",
        "admin" | "packages" => "admin",
        _ => "desktop-controls",
    }
}
fn route(request: &str, id: &str, cfg: &Config) -> Result<Value> {
    if id == "clock" {
        return Ok(cfg
            .clock
            .spoken_reply(request)
            .map(|reply| json!({"tool":"clock.now","arguments":{"request":request},"reply":reply}))
            .unwrap_or(Value::Null));
    }
    let apps = AppsFile::load()?;
    let cmds = CommandsFile::load()?;
    Ok(match route_with(request, &apps.app, &cmds) {
        Intent::Desktop { tool, spoken, .. }
            if (id == "apps" && tool == "app.launch")
                || (id == "desktop-controls" && tool != "app.launch") =>
        {
            json!({"tool":tool,"arguments":{"request":request},"reply":spoken})
        }
        Intent::AgentHandoff {
            agent,
            prompt,
            spoken,
        } if id == "coding-agents" => {
            // "Open the coding agent" names none: the configured default, said by name.
            let (agent, spoken) = if agent.is_empty() {
                let agent = cfg.agent.default_agent.clone();
                let (_, spoken) = assistant_tools::agent::build_launch_cmd(&agent, &prompt, "");
                (agent, spoken)
            } else {
                (agent, spoken)
            };
            json!({"tool":"agent.handoff","arguments":{"agent":agent,"prompt":prompt},"reply":spoken})
        }
        _ => Value::Null,
    })
}
/// A desktop phrase's command: a typed Hyprland action (`hypr …`), one of a few `hyprctl`
/// commands that can't start programs, or an Omarchy command. Lua from a phrase file is
/// never run.
async fn run_phrase(argv: &[&str]) -> Result<String> {
    match argv {
        ["hypr", action @ ..] => assistant_omarchy::hypr::run_action(action).await,
        ["hyprctl", rest @ ..] => {
            phrase_hyprctl(rest)?;
            assistant_omarchy::hyprctl(rest).await
        }
        _ => assistant_omarchy::omarchy(argv).await,
    }
}

/// The `hyprctl` commands a phrase may use: `dispatch` (but no exec dispatcher),
/// `switchxkblayout`, `notify`, `dismissnotify`, `setcursor`. No options before them
/// (`--batch` runs several), no `eval`, `keyword`, `plugin` or `reload`.
fn phrase_hyprctl(args: &[&str]) -> Result<()> {
    match args {
        ["eval", ..] => anyhow::bail!("phrase commands can't run hyprctl eval; use a typed action such as [\"hypr\", \"close_window\"] (see config/commands.toml)"),
        ["dispatch", dispatcher, ..] if dispatcher.to_lowercase().contains("exec") => {
            anyhow::bail!("phrase commands can't start programs through hyprctl; name the program as an app in apps.toml")
        }
        ["dispatch", _, ..] | ["switchxkblayout" | "notify" | "dismissnotify" | "setcursor", ..] => Ok(()),
        _ => anyhow::bail!("phrase commands can use hyprctl dispatch, switchxkblayout, notify, dismissnotify or setcursor only; use a typed action (see config/commands.toml)"),
    }
}


#[tokio::main]
async fn main() -> Result<()> {
    let id = std::env::args().nth(1).unwrap_or_default();
    let mut cfg: Config = serde_json::from_str(&std::env::var("ASSISTANT_CONFIG")?)?;
    cfg.browser.enabled = false;
    cfg.mcp.enabled = false;
    let registry = Arc::new(Registry::load_worker(&cfg).await);
    let mut tools: Vec<_> = registry
        .tools
        .values()
        .filter(|t| group(&t.name) == id)
        .cloned()
        .collect();
    let request_schema = json!({"type":"object","properties":{"request":{"type":"string","maxLength":16384}},"required":["request"],"additionalProperties":false});
    if id == "desktop-controls" || id == "apps" {
        let mut names: Vec<_> = if id == "apps" {
            vec!["app.launch".into()]
        } else {
            CommandsFile::load()?
                .command
                .into_iter()
                .filter(|c| c.tool != "app.launch")
                .map(|c| c.tool)
                .collect()
        };
        if id == "desktop-controls" {
            names.extend(["hypr.workspace".into(), "hypr.workspace_move".into()]);
        }
        names.sort();
        names.dedup();
        for name in names {
            tools.push(McpTool {
                name,
                description: Some("Execute a configured desktop voice phrase".into()),
                input_schema: request_schema.clone(),
                output_schema: None,
                execution: Value::Null,
            });
        }
    }
    if id == "clock" {
        tools.push(McpTool {
            name: "clock.now".into(),
            description: Some("Speak local time/date for a clock request".into()),
            input_schema: request_schema,
            output_schema: None,
            execution: Value::Null,
        });
    }
    let output = Arc::new(Mutex::new(tokio::io::stdout()));
    let pending = Arc::new(Mutex::new(HashMap::<i64, CancellationToken>::new()));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        if line.len() > 1024 * 1024 {
            break;
        }
        let v: Value = serde_json::from_str(&line)?;
        if v["method"] == "notifications/cancelled" {
            if let Some(id) = v["params"]["requestId"].as_i64() {
                if let Some(token) = pending.lock().await.get(&id) {
                    token.cancel();
                }
            }
            continue;
        }
        let Some(request_id) = v["id"].as_i64() else {
            continue;
        };
        let token = CancellationToken::new();
        pending.lock().await.insert(request_id, token.clone());
        let registry = registry.clone();
        let cfg = cfg.clone();
        let id = id.clone();
        let output = output.clone();
        let pending = pending.clone();
        let tools = tools.clone();
        tokio::spawn(async move {
            let result:Result<Value>=async {match v["method"].as_str().unwrap_or("") {
                "initialize"=>Ok(json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{},"experimental":{"assistant":{"version":1}}},"serverInfo":{"name":id,"version":env!("CARGO_PKG_VERSION")}})),
                "tools/list"=>Ok(json!({"tools":tools})),
                "ping"=>Ok(json!({})),
                "assistant/route"=>{anyhow::ensure!(v["params"]["version"]==1,"unsupported extension");route(v["params"]["request"].as_str().unwrap_or(""),&id,&cfg)},
                "assistant/shutdown" | "assistant/cancel_jobs"=>{registry.jobs.cancel_all().await;Ok(json!({}))},
                "tools/call"=> {
                    let name=v["params"]["name"].as_str().unwrap_or("");let args=v["params"]["arguments"].clone();
                    let tool=tools.iter().find(|t|t.name==name).ok_or_else(||anyhow::anyhow!("unknown tool"))?;
                    // The host decides and asks; the worker refuses any call without the host's
                    // grant for exactly this tool and these arguments, and non-owned tools.
                    let _=tool;
                    let grant=Grant::from_attestation(&v["params"]["_meta"])?;
                    if name=="clock.now" || args["request"].is_string() {grant.check(name,&args)?;}
                    if name=="clock.now" {let text=cfg.clock.spoken_reply(args["request"].as_str().unwrap_or("")).ok_or_else(||anyhow::anyhow!("unknown clock request"))?;return Ok(json!({"content":[{"type":"text","text":text}],"isError":false}));}
                    if args["request"].is_string() {
                        let request=args["request"].as_str().unwrap();
                        let hit=route(request,&id,&cfg)?;anyhow::ensure!(hit["tool"]==name,"phrase does not match tool");
                        if let Intent::Desktop{omarchy,..}=route_with(request,&AppsFile::load()?.app,&CommandsFile::load()?) {
                            let argv:Vec<_>=omarchy.iter().map(String::as_str).collect();
                            let text=tokio::select! {_=token.cancelled()=>anyhow::bail!("cancelled"),r=run_phrase(&argv)=>r?};
                            return Ok(json!({"content":[{"type":"text","text":text}],"isError":false}));
                        }
                    }
                    registry.call(name,args,&cfg,&token,&grant).await
                },
                _=>anyhow::bail!("unsupported method"),
            }}.await;
            let response = match result {
                Ok(result) => json!({"jsonrpc":"2.0","id":request_id,"result":result}),
                Err(e) => {
                    json!({"jsonrpc":"2.0","id":request_id,"error":{"code":-32603,"message":e.to_string()}})
                }
            };
            let mut raw = response.to_string();
            raw.push('\n');
            let _ = output.lock().await.write_all(raw.as_bytes()).await;
            pending.lock().await.remove(&request_id);
        });
    }
    registry.jobs.cancel_all().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::phrase_hyprctl;

    #[test]
    fn phrase_hyprctl_cant_start_programs() {
        for ok in [&["dispatch", "workspace", "3"][..], &["switchxkblayout", "all", "next"], &["dismissnotify"]] {
            assert!(phrase_hyprctl(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            &["eval", "hl.exec('x')"][..],
            &["dispatch", "exec", "foot"],
            &["dispatch", "execr", "foot"],
            &["dispatch", "exec,foot"],
            &["--batch", "dispatch exec foot"],
            &["-j", "dispatch", "exec", "foot"],
            &["plugin", "load", "/tmp/x.so"],
            &["keyword", "bind", "SUPER,x,exec,foot"],
            &["dispatch"],
            &[],
        ] {
            assert!(phrase_hyprctl(bad).is_err(), "{bad:?}");
        }
    }
}
