use crate::{
    mcp::{McpClient, McpTool},
    typed::execute_typed_tool,
};
use anyhow::{Context, Result};
use assistant_core::Config;
use assistant_policy::{store::AuditLog, store::Record, Decision, Grant, GrantKind, Verdict};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;

/// The outcome of `Registry::authorize`.
pub enum Authorization {
    Granted(Grant),
    NeedsApproval(Verdict),
}

pub struct Registry {
    pub audit: std::sync::Arc<AuditLog>,
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
            audit: std::sync::Arc::new(AuditLog::disabled()),
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
            ("terminal.run","Run an exact argument array as a managed terminal job, sandboxed to cwd (a project folder) unless sandbox is false. Always requires confirmation.",object(json!({"argv":{"type":"array","items":string,"minItems":1,"maxItems":128},"cwd":string,"show":{"type":"boolean"},"sandbox":{"type":"boolean"}}),vec!["argv"])),
            ("terminal.configured","Run an approved configured command by its exact name.",object(json!({"name":string,"cwd":string,"show":{"type":"boolean"}}),vec!["name"])),
            ("terminal.shell","Run an explicit shell script, sandboxed to cwd (a project folder) unless sandbox is false. Always requires confirmation.",object(json!({"script":string,"cwd":string,"show":{"type":"boolean"},"sandbox":{"type":"boolean"}}),vec!["script"])),
            ("terminal.status","Read terminal job output and exit status.",object(json!({"job_id":string}),vec!["job_id"])),
            ("terminal.cancel","Cancel a terminal job and its descendants.",object(json!({"job_id":string}),vec!["job_id"])),
            ("terminal.open","Open a terminal or a live view of a managed job.",object(json!({"job_id":string,"cwd":string}),vec![])),
            ("agent.handoff","Open a coding agent with a prompt and working directory.",object(json!({"agent":string,"prompt":string,"cwd":string}),vec!["prompt"])),
            ("apps.launch","Launch an application using one exact configured app phrase.",object(json!({"phrase":string}),vec!["phrase"])),
        ] { registry.tools.insert(name.into(),McpTool {name:name.into(),description:Some(description.into()),input_schema:schema,output_schema:None,execution:Value::Null}); }
        let path = json!({"type":"string","maxLength":4096});
        let file_tools = [
            ("files.list", "List a folder inside HOME (up to 200 entries).", object(json!({"path":path}), vec!["path"])),
            ("files.search", "Find files under a folder by name, or by text inside them.", object(json!({"path":path,"name":{"type":"string","maxLength":256},"text":{"type":"string","maxLength":256}}), vec!["path"])),
            ("files.write", "Create, overwrite or append to a text file inside HOME (up to 1 MiB). Keeps a backup to undo.", object(json!({"path":path,"content":{"type":"string","maxLength":crate::fs::MAX_WRITE},"mode":{"enum":["create","overwrite","append"]}}), vec!["path","content","mode"])),
            ("files.move", "Move or rename a file or folder inside HOME.", object(json!({"from":path,"to":path,"overwrite":{"type":"boolean"}}), vec!["from","to"])),
            ("files.trash", "Move a file or folder to the Trash.", object(json!({"path":path}), vec!["path"])),
            ("files.undo", "Undo a file change from its backup ID.", object(json!({"backup":{"type":"string","maxLength":64}}), vec!["backup"])),
        ];
        for (name, description, schema) in file_tools {
            registry.tools.insert(name.into(), McpTool { name: name.into(), description: Some(description.into()), input_schema: schema, output_schema: None, execution: Value::Null });
        }
        for (name, description, schema) in crate::system::TOOLS.iter().chain(crate::admin::TOOLS) {
            let input_schema = serde_json::from_str(schema).expect("system tool schema");
            registry.tools.insert((*name).into(), McpTool { name: (*name).into(), description: Some((*description).into()), input_schema, output_schema: None, execution: Value::Null });
        }
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
            audit: std::sync::Arc::new(AuditLog::disabled()),
            plugins: Some(plugins),
            tools: BTreeMap::new(),
            clients: BTreeMap::new(),
            owners: BTreeMap::new(),
            jobs: Default::default(),
            location: Default::default(),
            errors,
        }
    }
    /// Journal decisions and calls to `log` (the daemon passes `agent.db`).
    pub fn with_audit(mut self, log: AuditLog) -> Self {
        self.audit = std::sync::Arc::new(log);
        self
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
    /// The decision point for one tool under the running config, without a resource.
    pub fn verdict(&self, name: &str, cfg: &Config) -> Verdict {
        self.verdict_for(name, None, cfg)
    }
    /// The decision for a call: the arguments can name a resource the user pre-approved.
    pub fn verdict_for(&self, name: &str, args: Option<&Value>, cfg: &Config) -> Verdict {
        let plugins = self.plugins.as_ref();
        let rules = cfg.permissions.rules();
        assistant_policy::decide(
            name,
            &assistant_policy::Context {
                available: plugins.is_none_or(|p| p.available(name)),
                trusted: plugins.is_none_or(|p| p.trusted_tool(name)),
                tool_rule: cfg.mcp.permissions.get(name).and_then(|s| Decision::parse(s)),
                rules: &rules,
                mode: cfg.agent.mode,
                resource_approved: args.is_some_and(|a| resource_approved(name, a, cfg)),
            },
        )
    }
    /// `allow`, `confirm` or `deny`, as Diagnostics, the TUI and IPC show it.
    pub fn policy(&self, name: &str, cfg: &Config) -> &'static str {
        self.verdict(name, cfg).decision().as_str()
    }
    /// Decide a call: a grant when it is allowed, the verdict when it needs approval.
    /// Denials are journalled here; approvals and calls where they happen.
    pub fn authorize(&self, name: &str, args: &Value, cfg: &Config) -> Result<Authorization> {
        let verdict = self.verdict_for(name, Some(args), cfg);
        match verdict.decision() {
            Decision::Allow => Ok(Authorization::Granted(Grant::allowed(&verdict, args)?)),
            Decision::Deny => Err(self.refuse(&verdict, args)),
            _ => Ok(Authorization::NeedsApproval(verdict)),
        }
    }
    /// Journal a denial and return the error to report.
    pub fn refuse(&self, verdict: &Verdict, args: &Value) -> anyhow::Error {
        let mut record = self.record("decision", verdict, args);
        record.outcome = "denied".into();
        self.journal(record);
        anyhow::anyhow!("tool denied or plugin disabled")
    }
    /// For calls that must not wait for a person: allowed, or refused.
    pub fn grant_now(&self, name: &str, args: &Value, cfg: &Config) -> Result<Grant> {
        match self.authorize(name, args, cfg)? {
            Authorization::Granted(grant) => Ok(grant),
            Authorization::NeedsApproval(_) => anyhow::bail!("{name} needs approval"),
        }
    }
    /// Call a tool the policy allows outright; never waits for approval.
    pub async fn call_allowed(
        &self,
        name: &str,
        args: Value,
        cfg: &Config,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let grant = self.grant_now(name, &args, cfg)?;
        self.call(name, args, cfg, cancel, &grant).await
    }
    pub fn record(&self, event: &str, verdict: &Verdict, args: &Value) -> Record {
        let mut record = Record::new(event, verdict.tool(), args);
        record.decision = verdict.decision().as_str().into();
        record.rule = verdict.rule().into();
        record
    }
    pub fn journal(&self, record: Record) {
        self.audit.record(record);
    }
    fn input_schema(&self, name: &str) -> Option<Value> {
        match &self.plugins {
            Some(plugins) => plugins
                .tools()
                .into_iter()
                .find(|t| t.name == name)
                .map(|t| t.input_schema),
            None => self.tools.get(name).map(|t| t.input_schema.clone()),
        }
    }
    /// Drop `null` for optional arguments the schema doesn't allow to be null. Small models
    /// write `"cwd": null` for "not given", which would otherwise fail validation.
    pub fn normalize_args(&self, name: &str, args: Value) -> Value {
        match self.input_schema(name) {
            Some(schema) => drop_null_optionals(&schema, args),
            None => args,
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
        grant: &Grant,
    ) -> Result<Value> {
        self.validate(name, &args)?;
        // The enforcement point: a grant for exactly this call, and a policy that still
        // permits it (a reload may have tightened it since the grant was made).
        grant.check(name, &args)?;
        let verdict = self.verdict_for(name, Some(&args), cfg);
        match (verdict.decision(), grant.kind()) {
            (Decision::Deny, _) => anyhow::bail!("tool permission not granted"),
            (d, GrantKind::Allowed) if d != Decision::Allow => {
                anyhow::bail!("tool permissions changed; retry")
            }
            _ => {}
        }
        if cancel.is_cancelled() {
            anyhow::bail!("cancelled before dispatch");
        }
        let mut record = self.record("call", &verdict, &args);
        record.turn = grant.turn().map(String::from);
        record.approval_id = grant.approval_id().map(String::from);
        let started = std::time::Instant::now();
        let result = self.dispatch(name, args, cfg, cancel, grant).await;
        record.duration_ms = Some(started.elapsed().as_millis() as u64);
        record.outcome = match &result {
            Ok(v) if v["isError"] == true => "error".into(),
            Ok(_) => "ok".into(),
            Err(_) if cancel.is_cancelled() => "cancelled".into(),
            Err(_) => "error".into(),
        };
        record.detail = result.as_ref().err().map(|e| e.to_string());
        self.journal(record);
        result
    }
    async fn dispatch(
        &self,
        name: &str,
        args: Value,
        cfg: &Config,
        cancel: &CancellationToken,
        grant: &Grant,
    ) -> Result<Value> {
        if let Some(plugins) = &self.plugins {
            return tokio::time::timeout(
                crate::admin::deadline(name),
                plugins.call(name, args, cfg, cancel, grant),
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
                    // Commands the assistant wrote run sandboxed unless the call says otherwise
                    // (it still asks either way); configured commands are the user's own.
                    let sandbox = (matches!(name, "terminal.run" | "terminal.shell") && args["sandbox"] != false)
                        .then(|| crate::fs::home().map(|home| crate::sandbox::Sandbox::from_config(&cfg.terminal, &home)))
                        .transpose()?;
                    let value = self
                        .jobs
                        .start_in(argv, cwd, cfg.terminal.timeout_secs, sandbox.as_ref())
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
        } else if crate::system::TOOLS.iter().any(|(tool, ..)| *tool == name) {
            let value = tokio::select! {
                _ = cancel.cancelled() => anyhow::bail!("cancelled"),
                r = crate::system::call(name, &args) => r.context("unknown system tool")??,
            };
            json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":false})
        } else if crate::admin::TOOLS.iter().any(|(tool, ..)| *tool == name) {
            // Cancelling during the password prompt stops pkexec; once the helper runs as
            // root it can't be signalled and finishes on its own.
            let value = tokio::select! {
                _ = cancel.cancelled() => anyhow::bail!("cancelled; if the admin action had already started it finishes (see nixe-assistant admin-log)"),
                r = crate::admin::call(name, &args) => r.context("unknown admin tool")??,
            };
            json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":false})
        } else if name.starts_with("files.") && name != "files.read" {
            let args = args.clone();
            let name = name.to_string();
            let value = tokio::task::spawn_blocking(move || file_tool(&name, &args)).await??;
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
            // The grant was checked above; the typed tool's own files_read check must not
            // refuse a call the user approved.
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

fn file_tool(name: &str, args: &Value) -> Result<Value> {
    let home = crate::fs::home()?;
    let s = |k: &str| args[k].as_str().unwrap_or("");
    match name {
        "files.list" => crate::fs::list(&home, s("path")),
        "files.search" => {
            if !s("text").is_empty() {
                search_text(&home, s("path"), s("text"))
            } else {
                anyhow::ensure!(!s("name").is_empty(), "give a name or text to search for");
                crate::fs::find_names(&home, s("path"), s("name"))
            }
        }
        "files.write" => crate::fs::write(&home, s("path"), s("content"), s("mode")),
        "files.move" => crate::fs::rename(&home, s("from"), s("to"), args["overwrite"] == true),
        "files.trash" => crate::fs::trash(&home, s("path")),
        "files.undo" => crate::fs::undo(&home, s("backup")),
        _ => anyhow::bail!("unknown file tool"),
    }
}

/// Files under a folder containing `text`, via ripgrep (skips hidden and ignored files).
fn search_text(home: &std::path::Path, path: &str, text: &str) -> Result<Value> {
    let root = crate::fs::guard(home, path, crate::fs::Access::Read)?;
    anyhow::ensure!(root.is_dir(), "not a folder");
    let out = std::process::Command::new("rg")
        .args(["-l", "-i", "-F", "--max-filesize", "1M", "--max-depth", "8", "--", text])
        .arg(&root)
        .output()
        .context("ripgrep (rg) isn't available")?;
    let matches: Vec<_> = String::from_utf8_lossy(&out.stdout).lines().take(50).map(String::from).collect();
    Ok(json!({"path": root, "text": text, "matches": matches, "truncated": matches.len() == 50}))
}

/// A service unit listed in `[system] user_units` may be controlled without asking.
fn resource_approved(name: &str, args: &Value, cfg: &Config) -> bool {
    name == "services.control"
        && args["unit"].as_str().is_some_and(|unit| cfg.system.user_units.iter().any(|u| u == unit))
}

fn drop_null_optionals(schema: &Value, args: Value) -> Value {
    let Value::Object(mut map) = args else {
        return args;
    };
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    map.retain(|key, value| {
        let allows_null = match &schema["properties"][key]["type"] {
            Value::String(t) => t == "null",
            Value::Array(types) => types.iter().any(|t| t == "null"),
            _ => false,
        };
        !value.is_null() || required.contains(&key.as_str()) || allows_null
    });
    Value::Object(map)
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
    #[test]
    fn null_optionals_are_dropped() {
        let schema = json!({"type":"object","properties":{"argv":{"type":"array"},"cwd":{"type":"string"},"note":{"type":["string","null"]}},"required":["argv"]});
        let out = drop_null_optionals(&schema, json!({"argv":null,"cwd":null,"note":null,"show":false}));
        assert_eq!(out, json!({"argv":null,"note":null,"show":false}));
        assert_eq!(drop_null_optionals(&schema, json!([1])), json!([1]));
    }
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
        // External tools ask by default: no grant without approval.
        assert!(matches!(
            registry.authorize("fixture.echo", &args, &cfg).unwrap(),
            Authorization::NeedsApproval(_)
        ));
        assert!(registry
            .call_allowed("fixture.echo", args.clone(), &cfg, &cancel)
            .await
            .is_err());
        let verdict = registry.verdict("fixture.echo", &cfg);
        assert!(Grant::allowed(&verdict, &args).is_err());
        // An approval for other arguments doesn't cover these.
        let other = Grant::approved(&verdict, &json!({"text":"other"}), "t1").unwrap();
        assert!(registry
            .call("fixture.echo", args.clone(), &cfg, &cancel, &other)
            .await
            .is_err());
        // A deny wins over an approval made before it.
        let approved = Grant::approved(&verdict, &args, "t2").unwrap();
        cfg.mcp
            .permissions
            .insert("fixture.echo".into(), "deny".into());
        assert!(registry.authorize("fixture.echo", &args, &cfg).is_err());
        assert!(registry
            .call("fixture.echo", args.clone(), &cfg, &cancel, &approved)
            .await
            .is_err());
        let raw = std::fs::read_to_string(&log).unwrap();
        assert!(!raw.contains("tools/call"));
        cfg.mcp
            .permissions
            .insert("fixture.echo".into(), "allow".into());
        assert!(registry
            .call_allowed("fixture.echo", json!({"text":42}), &cfg, &cancel)
            .await
            .is_err());
        let result = registry
            .call_allowed("fixture.echo", args, &cfg, &cancel)
            .await
            .unwrap();
        assert_eq!(result["structuredContent"]["text"], "hello");
        let result = registry
            .call_allowed(
                "fixture.echo",
                json!({"text":"error"}),
                &cfg,
                &cancel,
            )
            .await
            .unwrap();
        assert_eq!(result["isError"], true);
        let waiting = registry.call_allowed("fixture.echo", json!({"text":"wait"}), &cfg, &cancel);
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
            .call_allowed(
                "fixture.echo",
                json!({"text":"disconnect"}),
                &cfg,
                &CancellationToken::new()
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
