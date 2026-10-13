use crate::{
    mcp::{McpClient, McpTool},
    typed::execute_typed_tool,
};
use anyhow::{Context, Result};
use assistant_core::Config;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;

pub struct Registry {
    pub plugins: Option<std::sync::Arc<crate::plugins::PluginManager>>,
    pub tools: BTreeMap<String, McpTool>,
    clients: BTreeMap<String, McpClient>,
    owners: BTreeMap<String, String>,
    pub jobs: std::sync::Arc<crate::jobs::Jobs>,
    pub location: std::sync::Arc<assistant_core::location::LocationService>,
    pub errors: Vec<String>,
}
impl Registry {
    pub async fn load_worker(cfg: &Config) -> Self {
        let mut registry = Self {
            plugins: None,
            tools: BTreeMap::new(),
            clients: BTreeMap::new(),
            owners: BTreeMap::new(),
            jobs: Default::default(),
            location: Default::default(),
            errors: Vec::new(),
        };
        for (name, description, schema) in [
            (
                "weather.lookup",
                "Get current weather-model conditions and seven-day forecast from Open-Meteo. Optional city overrides device location; US locations always use Fahrenheit/mph; celsius selects Celsius outside the US.",
                json!({"type":"object","properties":{"city":{"type":"string","maxLength":256},"celsius":{"type":"boolean"}},"additionalProperties":false}),
            ),
            (
                "location.current",
                "Get saved or approximate city-level IP location; never claim GPS precision.",
                json!({"type":"object","properties":{},"additionalProperties":false}),
            ),
            (
                "files.read",
                "Read up to 64 KiB and 500 lines of a file inside HOME",
                json!({"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1}},"required":["path"],"additionalProperties":false}),
            ),
            (
                "git.status",
                "Inspect git status inside HOME",
                json!({"type":"object","properties":{"directory":{"type":"string"}},"additionalProperties":false}),
            ),
            (
                "system.status",
                "Inspect memory, load and battery",
                json!({"type":"object","additionalProperties":false}),
            ),
        ] {
            registry.tools.insert(
                name.into(),
                McpTool {
                    name: name.into(),
                    description: Some(description.into()),
                    input_schema: schema,
                    output_schema: None,
                    execution: Value::Null,
                },
            );
        }
        let string = json!({"type":"string","maxLength":16384});
        let object = |properties: Value, required: Vec<&str>| json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
        for (name,description,schema) in [
            ("terminal.run","Run an exact argument array as a managed terminal job. Generated commands require confirmation.",object(json!({"argv":{"type":"array","items":string,"minItems":1,"maxItems":128},"cwd":string,"show":{"type":"boolean"}}),vec!["argv"])),
            ("terminal.configured","Run an approved configured command by its exact name.",object(json!({"name":string,"cwd":string,"show":{"type":"boolean"}}),vec!["name"])),
            ("terminal.shell","Run an explicit shell script. Always requires confirmation.",object(json!({"script":string,"cwd":string,"show":{"type":"boolean"}}),vec!["script"])),
            ("terminal.status","Read terminal job output and exit status.",object(json!({"job_id":string}),vec!["job_id"])),
            ("terminal.cancel","Cancel a terminal job and its descendants.",object(json!({"job_id":string}),vec!["job_id"])),
            ("terminal.open","Open a terminal or a live view of a managed job.",object(json!({"job_id":string,"cwd":string}),vec![])),
            ("agent.handoff","Open a coding agent with a prompt and working directory.",object(json!({"agent":string,"prompt":string,"cwd":string}),vec!["prompt"])),
            ("apps.launch","Launch an application using one exact configured app phrase.",object(json!({"phrase":string}),vec!["phrase"])),
        ] { registry.tools.insert(name.into(),McpTool {name:name.into(),description:Some(description.into()),input_schema:schema,output_schema:None,execution:Value::Null}); }
        if cfg.browser.enabled {
            let path = if cfg.browser.worker == "auto" {
                assistant_core::paths::data_dir().join("browser/worker.mjs")
            } else {
                std::path::PathBuf::from(&cfg.browser.worker)
            };
            let result = async {
                anyhow::ensure!(
                    path.is_file(),
                    "browser worker missing: run scripts/install.sh"
                );
                let client = McpClient::spawn(
                    ":browser",
                    &cfg.browser.node,
                    &[path.display().to_string()],
                    &std::collections::HashMap::from([(
                        "PLAYWRIGHT_BROWSERS_PATH".into(),
                        assistant_core::paths::cache_dir()
                            .join("chromium")
                            .display()
                            .to_string(),
                    )]),
                )
                .await?;
                for tool in client.list_tools().await? {
                    anyhow::ensure!(
                        tool.name.starts_with("browser.") || tool.name.starts_with("web."),
                        "invalid browser tool"
                    );
                    jsonschema::validator_for(&tool.input_schema)?;
                    registry.owners.insert(tool.name.clone(), ":browser".into());
                    registry.tools.insert(tool.name.clone(), tool);
                }
                registry.clients.insert(":browser".into(), client);
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(err) = result {
                registry.errors.push(format!("browser: {err:#}"));
            }
        }
        if cfg.mcp.enabled {
            for (name, server) in &cfg.mcp.servers {
                let result = async {
                    let client =
                        McpClient::spawn(name, &server.command, &server.args, &server.env).await?;
                    let tools = client.list_tools().await?;
                    let mut discovered = Vec::new();
                    if tools.len() > 2048 {
                        anyhow::bail!("MCP tool count limit exceeded");
                    }
                    for tool in tools {
                        if tool.execution["taskSupport"] == "required" {
                            anyhow::bail!("{} requires unsupported task execution", tool.name);
                        }
                        if tool.name.is_empty()
                            || tool.name.len() > 128
                            || !tool
                                .name
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
                        {
                            anyhow::bail!("invalid tool name");
                        }
                        jsonschema::validator_for(&tool.input_schema)
                            .context("invalid input schema")?;
                        if let Some(schema) = &tool.output_schema {
                            jsonschema::validator_for(schema).context("invalid output schema")?;
                        }
                        let key = format!("{name}.{}", tool.name);
                        if discovered.iter().any(|(n, _)| n == &key)
                            || registry.tools.contains_key(&key)
                        {
                            anyhow::bail!("duplicate tool {key}");
                        }
                        discovered.push((key, tool));
                    }
                    for (key, _) in &discovered {
                        registry.owners.insert(key.clone(), name.clone());
                    }
                    registry.tools.extend(discovered);
                    registry.clients.insert(name.clone(), client);
                    Ok::<(), anyhow::Error>(())
                }
                .await;
                if let Err(e) = result {
                    registry.errors.push(format!("{name}: {e:#}"));
                }
            }
        }
        if !cfg.browser.search_tool.is_empty()
            && !registry.tools.contains_key(&cfg.browser.search_tool)
        {
            registry.errors.push(format!(
                "configured search tool {} is unavailable",
                cfg.browser.search_tool
            ));
        }
        registry
    }
    pub async fn load(cfg: &Config) -> Self {
        let plugins = std::sync::Arc::new(crate::plugins::PluginManager::new());
        let errors = plugins.load(cfg).await;
        Self {
            plugins: Some(plugins),
            tools: BTreeMap::new(),
            clients: BTreeMap::new(),
            owners: BTreeMap::new(),
            jobs: Default::default(),
            location: Default::default(),
            errors,
        }
    }
    pub fn system_prompt(&self, cfg: &Config) -> String {
        let mut prompt_cfg = cfg.clone();
        if self.plugins.is_some() && self.policy("clock.now", cfg) != "allow" {
            prompt_cfg.clock.inject_prompt = false;
        }
        prompt_cfg.system_prompt()
    }
    pub async fn cancel_all_jobs(&self) {
        self.jobs.cancel_all().await;
        if let Some(plugins) = &self.plugins {
            plugins.cancel_jobs().await;
        }
    }
    pub fn policy<'a>(&self, name: &str, cfg: &'a Config) -> &'a str {
        if self.plugins.as_ref().is_some_and(|p| !p.available(name)) {
            return "deny";
        }
        if let Some(policy) = cfg.mcp.permissions.get(name) {
            return policy;
        }
        if self.plugins.as_ref().is_some_and(|p| !p.trusted_tool(name)) {
            return "confirm";
        }
        // Destructive edits, captures, coordinated effects, and resource changes always
        // start with confirmation. Explicit user grants still apply above.
        if name == "history.undo" {
            return "allow";
        }
        if name.ends_with(".undo")
            || matches!(
                name,
                "notes.edit"
                    | "notes.delete"
                    | "routines.save"
                    | "routines.delete"
                    | "routines.run"
                    | "projects.save"
                    | "projects.delete"
                    | "projects.action"
                    | "calendar.create"
                    | "calendar.edit"
                    | "calendar.delete"
                    | "capture.clipboard"
                    | "capture.screen"
                    | "file_search.index"
                    | "file_search.clear"
                    | "power.battery"
                    | "power.normal"
                    | "power.configure"
                    | "power.unload"
            )
        {
            return "confirm";
        }
        if [
            "routines.",
            "projects.",
            "calendar.",
            "file_search.",
            "media.",
            "power.",
            "history.",
        ]
        .iter()
        .any(|p| name.starts_with(p))
        {
            return "allow";
        }
        if name.starts_with("notes.") || name.starts_with("reminders.") || name == "clock.now" {
            return "allow";
        }
        if crate::builtin_tools().iter().any(|t| t.name == name) {
            if name.starts_with("audio.") {
                return &cfg.permissions.volume;
            }
            if name == "app.launch" {
                return &cfg.permissions.launch_app;
            }
            if name == "system.shutdown" {
                return &cfg.permissions.shutdown;
            }
            return "allow";
        }
        match name {
            "files.read" | "git.status" => &cfg.permissions.files_read,
            "weather.lookup"
            | "location.current"
            | "system.status"
            | "terminal.configured"
            | "terminal.status"
            | "terminal.cancel"
            | "terminal.open"
            | "browser.search"
            | "browser.navigate"
            | "browser.remember_sources"
            | "browser.open_source"
            | "browser.snapshot"
            | "browser.tabs"
            | "browser.scroll"
            | "browser.takeover"
            | "browser.resume"
            | "web.search"
            | "web.read" => "allow",
            "agent.handoff" | "apps.launch" => &cfg.permissions.launch_app,
            _ => "confirm",
        }
    }
    pub fn validate(&self, name: &str, args: &Value) -> Result<()> {
        if let Some(plugins) = &self.plugins {
            return plugins.validate(name, args);
        }
        let tool = self.tools.get(name).context("unregistered tool")?;
        jsonschema::validator_for(&tool.input_schema)?
            .validate(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments: {e}"))
    }
    pub async fn call(
        &self,
        name: &str,
        args: Value,
        cfg: &Config,
        cancel: &CancellationToken,
        approved: bool,
    ) -> Result<Value> {
        self.validate(name, &args)?;
        let policy = self.policy(name, cfg);
        if policy == "deny" || (policy == "confirm" && !approved) {
            anyhow::bail!("tool permission not granted");
        }
        if cancel.is_cancelled() {
            anyhow::bail!("cancelled before dispatch");
        }
        if let Some(plugins) = &self.plugins {
            return tokio::time::timeout(
                std::time::Duration::from_secs(15),
                plugins.call(name, args, cfg, cancel),
            )
            .await
            .context("plugin call deadline exceeded")?;
        }
        let result = if let Some(server) = self.owners.get(name) {
            self.clients[server]
                .call_tool(&self.tools[name].name, args, cancel)
                .await?
        } else if name == "weather.lookup" {
            let value = tokio::select! {_=cancel.cancelled()=>anyhow::bail!("weather lookup cancelled"),r=assistant_core::weather::lookup(&self.location,cfg,args["city"].as_str(),args["celsius"].as_bool().unwrap_or(false))=>r?};
            json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":false})
        } else if name == "location.current" {
            let location = tokio::select! {_=cancel.cancelled()=>anyhow::bail!("location lookup cancelled"),r=self.location.resolve(&cfg.location,false)=>r?};
            let value = serde_json::to_value(location)?;
            json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":false})
        } else if name.starts_with("terminal.") {
            let home = dirs_home();
            let cwd = args["cwd"].as_str().unwrap_or(&home);
            let value = match name {
                "terminal.run" | "terminal.shell" | "terminal.configured" => {
                    let argv = match name {
                        "terminal.shell" => vec![
                            "sh".into(),
                            "-c".into(),
                            args["script"].as_str().context("missing script")?.into(),
                        ],
                        "terminal.configured" => cfg
                            .terminal
                            .commands
                            .get(args["name"].as_str().unwrap_or(""))
                            .context("unknown configured command")?
                            .clone(),
                        _ => serde_json::from_value(args["argv"].clone())?,
                    };
                    let value = self
                        .jobs
                        .start(argv, cwd, cfg.terminal.timeout_secs)
                        .await?;
                    if args["show"].as_bool().unwrap_or(true) {
                        if let Err(err) = open_job(value["job_id"].as_str().unwrap()).await {
                            tracing::warn!(%err,"terminal view failed");
                        }
                    }
                    value
                }
                "terminal.status" => self.jobs.status(args["job_id"].as_str().unwrap()).await?,
                "terminal.cancel" => self.jobs.cancel(args["job_id"].as_str().unwrap()).await?,
                "terminal.open" => {
                    if let Some(id) = args["job_id"].as_str() {
                        self.jobs.status(id).await?;
                        open_job(id).await?;
                    } else if let Some(directory) = args["cwd"].as_str() {
                        let cwd = crate::typed::home_path(directory)?;
                        anyhow::ensure!(cwd.is_dir(), "working directory is not a directory");
                        let command = format!(
                            "xdg-terminal-exec {}",
                            assistant_omarchy::shell_quote(&format!("--dir={}", cwd.display()))
                        );
                        assistant_omarchy::hyprctl(&["dispatch", "exec", &command]).await?;
                    } else {
                        assistant_omarchy::omarchy(&["launch", "terminal"]).await?;
                    }
                    json!({"opened":true})
                }
                _ => anyhow::bail!("unknown terminal tool"),
            };
            json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":false})
        } else if name == "apps.launch" {
            let apps = assistant_core::AppsFile::load()?;
            let phrase = args["phrase"].as_str().unwrap_or("");
            let app = apps
                .app
                .iter()
                .find(|a| a.phrases.iter().any(|p| p.eq_ignore_ascii_case(phrase)))
                .context("unknown app phrase")?;
            let argv = app.args();
            let refs = argv.iter().map(String::as_str).collect::<Vec<_>>();
            let output = assistant_omarchy::omarchy(&refs).await?;
            json!({"content":[{"type":"text","text":output}],"isError":false})
        } else if name == "agent.handoff" {
            anyhow::ensure!(cfg.agent.enabled, "agent handoff is disabled");
            let agent = args["agent"].as_str().unwrap_or(&cfg.agent.default_agent);
            let prompt = args["prompt"].as_str().unwrap_or("");
            let home = dirs_home();
            let cwd = crate::typed::home_path(args["cwd"].as_str().unwrap_or(&home))?;
            let (cmd, spoken) = crate::agent::build_launch_cmd(agent, prompt, &cfg.agent.terminal);
            assistant_omarchy::hypr_exec(&format!(
                "cd {} && {cmd}",
                assistant_omarchy::shell_quote(&cwd.display().to_string())
            ))
            .await?;
            json!({"content":[{"type":"text","text":spoken}],"isError":false})
        } else {
            let mut authorized = cfg.clone();
            if ["files.read", "git.status"].contains(&name) {
                authorized.permissions.files_read = "allow".into();
            }
            let text = tokio::select! {
                _ = cancel.cancelled() => anyhow::bail!("cancelled"),
                result = tokio::time::timeout(std::time::Duration::from_secs(15),execute_typed_tool(name,&args,&authorized.permissions,&cfg.agent)) => result??,
            };
            json!({"content":[{"type":"text","text":text}],"isError":false})
        };
        if let Some(schema) = &self.tools[name].output_schema {
            if !result["isError"].as_bool().unwrap_or(false) {
                jsonschema::validator_for(schema)?
                    .validate(&result["structuredContent"])
                    .map_err(|e| anyhow::anyhow!("invalid structured result: {e}"))?;
            }
        }
        Ok(result)
    }
    pub fn catalog(&self, cfg: &Config) -> Value {
        if let Some(plugins) = &self.plugins {
            return Value::Array(plugins.tools().iter().map(|tool| json!({"name":tool.name,"description":tool.description,"inputSchema":tool.input_schema,"permission":self.policy(&tool.name,cfg)})).collect());
        }
        Value::Array(self.tools.iter().map(|(name,tool)| json!({"name":name,"description":tool.description,"inputSchema":tool.input_schema,"permission":self.policy(name,cfg)})).collect())
    }
}

fn dirs_home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| ".".into())
}
async fn open_job(id: &str) -> Result<()> {
    let cmd = assistant_omarchy::shell_join(&[
        "omarchy-launch-terminal",
        "--",
        "nixe-assistant",
        "jobs",
        "follow",
        id,
    ]);
    assistant_omarchy::hypr_exec(&cmd).await?;
    Ok(())
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[tokio::test]
    async fn permissions_cancellation_results_and_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("events");
        let mut cfg = Config::default();
        cfg.browser.enabled = false;
        cfg.mcp.enabled = true;
        cfg.mcp.servers.insert(
            "fixture".into(),
            assistant_core::config::McpServerCfg {
                command: "python3".into(),
                args: vec![
                    "-u".into(),
                    format!("{}/tests/mcp_fixture.py", env!("CARGO_MANIFEST_DIR")),
                ],
                env: std::collections::HashMap::from([(
                    "FIXTURE_LOG".into(),
                    log.display().to_string(),
                )]),
            },
        );
        let registry = Registry::load_worker(&cfg).await;
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        assert!(registry.tools.contains_key("fixture.echo"));
        let cancel = CancellationToken::new();
        let args = json!({"text":"hello"});
        assert!(registry
            .call("fixture.echo", args.clone(), &cfg, &cancel, false)
            .await
            .is_err());
        cfg.mcp
            .permissions
            .insert("fixture.echo".into(), "deny".into());
        assert!(registry
            .call("fixture.echo", args.clone(), &cfg, &cancel, true)
            .await
            .is_err());
        let raw = std::fs::read_to_string(&log).unwrap();
        assert!(!raw.contains("tools/call"));
        cfg.mcp
            .permissions
            .insert("fixture.echo".into(), "allow".into());
        assert!(registry
            .call("fixture.echo", json!({"text":42}), &cfg, &cancel, false)
            .await
            .is_err());
        let result = registry
            .call("fixture.echo", args, &cfg, &cancel, false)
            .await
            .unwrap();
        assert_eq!(result["structuredContent"]["text"], "hello");
        let result = registry
            .call(
                "fixture.echo",
                json!({"text":"error"}),
                &cfg,
                &cancel,
                false,
            )
            .await
            .unwrap();
        assert_eq!(result["isError"], true);
        let waiting = registry.call("fixture.echo", json!({"text":"wait"}), &cfg, &cancel, false);
        let trigger = async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            cancel.cancel();
        };
        let (result, _) = tokio::join!(waiting, trigger);
        assert!(result.is_err());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(std::fs::read_to_string(&log)
            .unwrap()
            .contains("notifications/cancelled"));
        assert!(registry
            .call(
                "fixture.echo",
                json!({"text":"disconnect"}),
                &cfg,
                &CancellationToken::new(),
                false
            )
            .await
            .is_err());
        drop(registry);
        cfg.mcp
            .servers
            .get_mut("fixture")
            .unwrap()
            .env
            .insert("VERSION".into(), "unsupported".into());
        let bad = Registry::load_worker(&cfg).await;
        assert_eq!(bad.errors.len(), 1);
        assert!(!bad.tools.contains_key("fixture.echo"));
    }
}
