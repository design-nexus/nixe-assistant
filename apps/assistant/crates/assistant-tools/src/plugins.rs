//! Versioned local MCP packages. Package code has the user's OS permissions.
use crate::mcp::{McpClient, McpTool};
use anyhow::{Context, Result};
use assistant_core::{paths, Config};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub manifest_version: u32,
    pub id: String,
    pub version: String,
    pub description: String,
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub optional_dependencies: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default = "empty_schema")]
    pub settings_schema: Value,
}
fn empty_schema() -> Value {
    json!({"type":"object","additionalProperties":false})
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub pattern: String,
    pub tool: String,
    #[serde(default)]
    pub template: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub trusted: bool,
    #[serde(default)]
    pub source: String,
    #[serde(default = "empty_settings")]
    pub settings: Value,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            enabled: false,
            trusted: false,
            source: String::new(),
            settings: empty_settings(),
        }
    }
}
fn empty_settings() -> Value {
    json!({})
}
pub fn package_dir() -> PathBuf {
    paths::data_dir().join("abilities")
}
pub fn state_path() -> PathBuf {
    paths::config_dir().join("plugins.json")
}
pub fn data_dir(id: &str) -> PathBuf {
    paths::data_dir().join("plugin-data").join(id)
}
pub fn saved() -> Result<BTreeMap<String, Saved>> {
    match std::fs::read(state_path()) {
        Ok(b) => Ok(serde_json::from_slice(&b)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e.into()),
    }
}
pub fn save(state: &BTreeMap<String, Saved>) -> Result<()> {
    assistant_core::config::atomic_write(&state_path(), &serde_json::to_vec_pretty(state)?)?;
    Ok(())
}
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        && s != "."
        && s != ".."
}
impl Manifest {
    pub fn read(path: &Path) -> Result<Self> {
        let m: Self = toml::from_str(&std::fs::read_to_string(path.join("plugin.toml"))?)?;
        m.validate()?;
        Ok(m)
    }
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.manifest_version == 1, "unsupported manifest version");
        anyhow::ensure!(
            valid_id(&self.id) && !self.version.is_empty(),
            "invalid plugin ID/version"
        );
        anyhow::ensure!(
            !self.executable.is_empty()
                && !self.executable.contains('\0')
                && self.args.iter().all(|s| !s.contains('\0')),
            "invalid startup command"
        );
        anyhow::ensure!(
            self.dependencies
                .iter()
                .chain(&self.optional_dependencies)
                .all(|d| valid_id(d) && d != &self.id),
            "invalid dependency"
        );
        anyhow::ensure!(
            self.settings_schema.is_object(),
            "settings schema must be an object"
        );
        jsonschema::validator_for(&self.settings_schema)?;
        let mut names = HashSet::new();
        for name in &self.tools {
            anyhow::ensure!(
                valid_id(name) && names.insert(name),
                "invalid/duplicate tool"
            );
        }
        let mut routes = HashSet::new();
        for r in &self.routes {
            anyhow::ensure!(self.tools.contains(&r.tool), "route tool is not declared");
            anyhow::ensure!(routes.insert(r.pattern.to_lowercase()), "duplicate route");
            if r.template {
                anyhow::ensure!(
                    r.pattern.starts_with('^') && r.pattern.ends_with('$'),
                    "template must be anchored"
                );
                regex::Regex::new(&r.pattern)?;
            } else {
                anyhow::ensure!(!r.pattern.trim().is_empty(), "empty route");
            }
        }
        Ok(())
    }
}
struct Running {
    manifest: Manifest,
    saved: Saved,
    client: Arc<McpClient>,
    tools: Vec<McpTool>,
    wire_names: BTreeMap<String, String>,
    cancel: CancellationToken,
    error: Option<String>,
}
#[derive(Default)]
pub struct PluginManager {
    entries: Mutex<BTreeMap<String, Running>>,
    failures: Mutex<BTreeMap<String, String>>,
    changes: tokio::sync::Mutex<()>,
}
impl PluginManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn load(&self, cfg: &Config) -> Vec<String> {
        let mut errors = Vec::new();
        let state = match saved() {
            Ok(s) => s,
            Err(e) => return vec![e.to_string()],
        };
        let mut todo: BTreeMap<_, _> = state.into_iter().filter(|(_, s)| s.enabled).collect();
        for _ in 0..todo.len() + 1 {
            let ready: Vec<_> = todo
                .keys()
                .filter(|id| {
                    Manifest::read(&package_dir().join(id)).is_err()
                        || Manifest::read(&package_dir().join(id)).is_ok_and(|m| {
                            m.dependencies
                                .iter()
                                .all(|d| self.entries.lock().unwrap().contains_key(d))
                        })
                })
                .cloned()
                .collect();
            if ready.is_empty() {
                break;
            }
            for id in ready {
                todo.remove(&id);
                if let Err(e) = self.change("reload", &id, cfg).await {
                    errors.push(format!("{id}: {e:#}"));
                }
            }
        }
        for id in todo.keys() {
            let error = "missing, disabled, or cyclic dependency".to_string();
            self.failures
                .lock()
                .unwrap()
                .insert(id.clone(), error.clone());
            errors.push(format!("{id}: {error}"));
        }
        errors
    }
    pub fn tools(&self) -> Vec<McpTool> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.error.is_none() && r.client.healthy())
            .flat_map(|r| r.tools.clone())
            .collect()
    }
    pub fn owner(&self, name: &str) -> Option<String> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .find(|(_, r)| r.tools.iter().any(|t| t.name == name))
            .map(|(id, _)| id.clone())
    }
    pub fn available(&self, name: &str) -> bool {
        self.tools().iter().any(|t| t.name == name)
    }
    pub fn trusted_tool(&self, name: &str) -> bool {
        self.entries
            .lock()
            .unwrap()
            .values()
            .any(|r| r.saved.trusted && r.tools.iter().any(|t| t.name == name))
    }
    pub fn validate(&self, name: &str, args: &Value) -> Result<()> {
        let tool = self
            .tools()
            .into_iter()
            .find(|t| t.name == name)
            .context("tool disabled or unavailable")?;
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
        grant: &assistant_policy::Grant,
    ) -> Result<Value> {
        self.validate(name, &args)?;
        let (id, client, own, schema, wire_name) = {
            let entries = self.entries.lock().unwrap();
            let (id, r) = entries
                .iter()
                .find(|(_, r)| r.tools.iter().any(|t| t.name == name))
                .context("plugin disabled")?;
            (
                id.clone(),
                r.client.clone(),
                r.cancel.clone(),
                r.tools
                    .iter()
                    .find(|t| t.name == name)
                    .unwrap()
                    .output_schema
                    .clone(),
                r.wire_names
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.into()),
            )
        };
        // Implicit weather location is resolved by the enabled location provider in the host.
        let mut args = args;
        if name == "weather.lookup" && args["city"].as_str().is_none_or(|s| s.is_empty()) {
            anyhow::ensure!(
                self.available("location.current") && cfg.location.enabled,
                "please specify a city"
            );
            anyhow::ensure!(
                cfg.mcp
                    .permissions
                    .get("location.current")
                    .is_none_or(|p| p == "allow"),
                "location permission requires confirmation; please specify a city"
            );
            // Part of the checked weather call; the location rule was checked just above.
            let meta = grant.attest("location.current", &json!({}))?;
            let location = self.call_raw("location.current", json!({}), meta, cancel).await?;
            anyhow::ensure!(
                cfg.mcp
                    .permissions
                    .get("location.current")
                    .is_none_or(|p| p == "allow"),
                "location permission requires confirmation; please specify a city"
            );
            args["city"] = if cfg.browser.weather_location.is_empty() {
                location["structuredContent"]["label"].clone()
            } else {
                json!(cfg.browser.weather_location)
            };
        }
        let result = tokio::select! { _=own.cancelled()=>anyhow::bail!("plugin disabled during call"), _=cancel.cancelled()=>anyhow::bail!("cancelled"), r=async { let meta = grant.attest(&wire_name, &args)?; client.call_tool_meta(&wire_name,args,Some(meta),cancel).await }=>r };
        match result {
            Ok(value) => {
                if value["isError"] != true {
                    if let Some(schema) = schema {
                        jsonschema::validator_for(&schema)?
                            .validate(&value["structuredContent"])
                            .map_err(|e| anyhow::anyhow!("invalid plugin output: {e}"))?;
                    }
                }
                Ok(value)
            }
            Err(e) => {
                if !client.healthy() {
                    if let Some(r) = self.entries.lock().unwrap().get_mut(&id) {
                        r.error = Some(e.to_string());
                        r.cancel.cancel();
                        r.client.terminate();
                    }
                }
                Err(e)
            }
        }
    }
    async fn call_raw(
        &self,
        name: &str,
        args: Value,
        meta: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        self.validate(name, &args)?;
        let (client, own) = {
            let entries = self.entries.lock().unwrap();
            let r = entries
                .values()
                .find(|r| r.tools.iter().any(|t| t.name == name))
                .context("provider disabled")?;
            (r.client.clone(), r.cancel.clone())
        };
        tokio::select! {_=own.cancelled()=>anyhow::bail!("provider disabled"),r=client.call_tool_meta(name,args,Some(meta),cancel)=>r}
    }
    pub async fn route(&self, request: &str, cancel: &CancellationToken) -> Result<Option<Value>> {
        let entries: Vec<_> = self
            .entries
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.error.is_none() && r.client.healthy())
            .map(|r| {
                (
                    r.manifest.clone(),
                    r.client.clone(),
                    r.cancel.clone(),
                    r.saved.trusted,
                )
            })
            .collect();
        let mut matches = Vec::new();
        for (m, client, own, trusted) in entries {
            if trusted && m.executable.ends_with("nixe-assistant-plugin-worker") {
                let route = tokio::select! {_=own.cancelled()=>continue,r=client.extension("assistant/route",json!({"version":1,"request":request}),cancel)=>r?};
                if !route.is_null() {
                    matches.push(route);
                }
            }
            for r in m.routes {
                let mut args = json!({"request":request});
                let hit = if r.template {
                    let re = regex::Regex::new(&r.pattern)?;
                    if let Some(c) = re.captures(request) {
                        for name in re.capture_names().flatten() {
                            if let Some(v) = c.name(name) {
                                args[name] = v.as_str().into();
                            }
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    r.pattern.eq_ignore_ascii_case(request.trim())
                };
                if hit {
                    matches.push(json!({"tool":r.tool,"arguments":args}));
                }
            }
        }
        anyhow::ensure!(
            matches.len() <= 1,
            "conflicting voice routes match this request; disable one of the conflicting plugins"
        );
        Ok(matches.pop())
    }
    pub fn inspect(&self) -> Value {
        let state = saved().unwrap_or_default();
        let entries = self.entries.lock().unwrap();
        let failures = self.failures.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(dirs) = std::fs::read_dir(package_dir()) {
            for dir in dirs.flatten() {
                let id = dir.file_name().to_string_lossy().into_owned();
                let manifest = Manifest::read(&dir.path());
                let running = entries.get(&id);
                out.push(json!({"id":id,"manifest":manifest.as_ref().ok(),"saved":state.get(&id),"running_enabled":running.is_some(),"health":running.map(|r|if r.error.is_none() && r.client.healthy(){"running"}else{"failed"}).unwrap_or("disabled"),"error":running.and_then(|r|r.error.clone()).or_else(||failures.get(&id).cloned()).or_else(||manifest.err().map(|e|e.to_string())),"data_directory":data_dir(&id)}));
            }
        }
        Value::Array(out)
    }
    pub async fn change(&self, action: &str, id: &str, cfg: &Config) -> Result<Value> {
        anyhow::ensure!(valid_id(id), "invalid plugin ID");
        let _change = self.changes.lock().await;
        let mut state = saved()?;
        if action == "disable" || action == "remove" {
            {
                let entries = self.entries.lock().unwrap();
                anyhow::ensure!(
                    !entries
                        .values()
                        .any(|r| r.manifest.dependencies.iter().any(|d| d == id)),
                    "disable dependents first"
                );
            }
            let removed = self.entries.lock().unwrap().remove(id);
            if let Some(r) = removed {
                r.cancel.cancel();
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(250),
                    r.client.extension(
                        "assistant/shutdown",
                        json!({"version":1}),
                        &CancellationToken::new(),
                    ),
                )
                .await;
                r.client.terminate();
            }
            state.entry(id.into()).or_default().enabled = false;
            save(&state)?;
            if action == "remove" {
                let path = package_dir().join(id);
                if path.exists() {
                    std::fs::remove_dir_all(path)?;
                }
            }
            return Ok(self.inspect());
        }
        anyhow::ensure!(
            ["enable", "reload"].contains(&action),
            "unknown plugin action"
        );
        let root = package_dir().join(id);
        let m = tokio::task::spawn_blocking(move || Manifest::read(&root)).await??;
        anyhow::ensure!(m.id == id, "package ID mismatch");
        let settings = state.entry(id.into()).or_default().clone();
        jsonschema::validator_for(&m.settings_schema)?
            .validate(&settings.settings)
            .map_err(|e| anyhow::anyhow!("invalid settings: {e}"))?;
        {
            let entries = self.entries.lock().unwrap();
            anyhow::ensure!(
                m.dependencies.iter().all(|d| entries
                    .get(d)
                    .is_some_and(|r| r.error.is_none() && r.client.healthy())),
                "required dependency is disabled"
            );
            for (other, r) in entries.iter().filter(|(other, _)| other.as_str() != id) {
                anyhow::ensure!(
                    !m.tools.iter().any(|t| r.tools.iter().any(|s| s.name == *t)),
                    "tool collision with {other}"
                );
                anyhow::ensure!(
                    !m.routes.iter().any(|route| r
                        .manifest
                        .routes
                        .iter()
                        .any(|s| s.pattern.eq_ignore_ascii_case(&route.pattern))),
                    "route collision with {other}"
                );
                anyhow::ensure!(
                    !m.providers
                        .iter()
                        .any(|role| r.manifest.providers.contains(role)),
                    "provider collision with {other}"
                );
            }
        }
        let directory = data_dir(id);
        std::fs::create_dir_all(&directory)?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut env = HashMap::from([
            (
                "ASSISTANT_PLUGIN_DATA".into(),
                directory.display().to_string(),
            ),
            (
                "ASSISTANT_PLUGIN_SETTINGS".into(),
                settings.settings.to_string(),
            ),
            ("ASSISTANT_PLUGIN_ID".into(), id.into()),
            ("ASSISTANT_CONFIG".into(), serde_json::to_string(cfg)?),
        ]);
        if let Some(server) = cfg.mcp.servers.get(id) {
            env.extend(server.env.clone());
        }
        if id == "browser" || id == "web-search" {
            env.insert(
                "PLAYWRIGHT_BROWSERS_PATH".into(),
                paths::cache_dir().join("chromium").display().to_string(),
            );
        }
        let args: Vec<_> = m
            .args
            .iter()
            .map(|arg| {
                if arg.starts_with("./") {
                    package_dir().join(id).join(arg).display().to_string()
                } else {
                    arg.clone()
                }
            })
            .collect();
        let executable = if m.executable.starts_with("./") {
            package_dir()
                .join(id)
                .join(&m.executable)
                .display()
                .to_string()
        } else {
            m.executable.clone()
        };
        let attempt = async {
            let client = Arc::new(McpClient::spawn(id, &executable, &args, &env).await?);
            let mut tools = client.list_tools().await?;
            if id == "browser" || id == "web-search" {
                tools.retain(|t| {
                    if id == "browser" {
                        t.name.starts_with("browser.")
                    } else {
                        t.name.starts_with("web.")
                    }
                });
            }
            let mut wire_names = BTreeMap::new();
            if cfg.mcp.servers.contains_key(id) {
                for tool in &mut tools {
                    let raw = tool.name.clone();
                    tool.name = format!("{id}.{raw}");
                    wire_names.insert(tool.name.clone(), raw);
                }
            }
            anyhow::ensure!(tools.len() <= 2048, "too many tools");
            let mut seen = HashSet::new();
            for t in &tools {
                anyhow::ensure!(
                    seen.insert(t.name.clone()) && m.tools.contains(&t.name),
                    "undeclared/duplicate tool"
                );
                anyhow::ensure!(t.input_schema.is_object(), "invalid schema");
                jsonschema::validator_for(&t.input_schema)?;
                if let Some(s) = &t.output_schema {
                    jsonschema::validator_for(s)?;
                }
            }
            anyhow::ensure!(tools.len() == m.tools.len(), "declared tools missing");
            Ok::<_, anyhow::Error>((client, tools, wire_names))
        }
        .await;
        let (client, tools, wire_names) = match attempt {
            Ok(v) => v,
            Err(e) => {
                self.failures
                    .lock()
                    .unwrap()
                    .insert(id.into(), format!("{e:#}"));
                return Err(e);
            }
        };
        state.entry(id.into()).or_default().enabled = true;
        save(&state)?;
        let old = self.entries.lock().unwrap().insert(
            id.into(),
            Running {
                manifest: m,
                saved: settings,
                client,
                tools,
                wire_names,
                cancel: CancellationToken::new(),
                error: None,
            },
        );
        if let Some(old) = old {
            old.cancel.cancel();
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(250),
                old.client.extension(
                    "assistant/shutdown",
                    json!({"version":1}),
                    &CancellationToken::new(),
                ),
            )
            .await;
            old.client.terminate();
        }
        self.failures.lock().unwrap().remove(id);
        Ok(self.inspect())
    }
    pub fn token(&self, name: &str) -> CancellationToken {
        self.entries
            .lock()
            .unwrap()
            .values()
            .find(|r| r.tools.iter().any(|t| t.name == name))
            .map(|r| r.cancel.clone())
            .unwrap_or_default()
    }
    pub fn generation(&self, name: &str) -> Option<u32> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .find(|r| r.tools.iter().any(|t| t.name == name))
            .and_then(|r| r.client.pid())
    }
}

/// Copy only ordinary files/directories. Never follow package symlinks or execute hooks.
fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let e = entry?;
        let kind = e.file_type()?;
        anyhow::ensure!(!kind.is_symlink(), "package symlinks are not accepted");
        if kind.is_dir() {
            copy_tree(&e.path(), &destination.join(e.file_name()))?;
        } else {
            anyhow::ensure!(kind.is_file(), "special package file");
            std::fs::copy(e.path(), destination.join(e.file_name()))?;
        }
    }
    Ok(())
}
pub fn stage(source: &str, revision: Option<&str>) -> Result<tempfile::TempDir> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path().join("package");
    if source.starts_with("https://") || source.starts_with("ssh://") {
        let rev =
            revision.context("Git sources require --revision with an exact 40-character commit")?;
        anyhow::ensure!(
            rev.len() == 40 && rev.bytes().all(|c| c.is_ascii_hexdigit()),
            "exact Git commit required"
        );
        let repo = temporary.path().join("repository");
        anyhow::ensure!(
            std::process::Command::new("git")
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "clone",
                    "--no-checkout",
                    "--",
                    source
                ])
                .arg(&repo)
                .status()?
                .success(),
            "clone failed"
        );
        anyhow::ensure!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "checkout",
                    "--detach",
                    rev
                ])
                .status()?
                .success(),
            "checkout failed"
        );
        copy_tree(&repo, &root)?;
    } else if Path::new(source).is_dir() {
        copy_tree(Path::new(source), &root)?;
    } else {
        std::fs::create_dir_all(&root)?;
        let file = std::fs::File::open(source)?;
        let reader: Box<dyn std::io::Read> = if source.ends_with(".gz") || source.ends_with(".tgz")
        {
            Box::new(flate2::read::GzDecoder::new(file))
        } else {
            Box::new(file)
        };
        let mut archive = tar::Archive::new(reader);
        let mut total = 0u64;
        for item in archive.entries()? {
            let mut entry = item?;
            let path = entry.path()?.into_owned();
            anyhow::ensure!(
                path.components().all(|c| matches!(
                    c,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )),
                "archive traversal"
            );
            let kind = entry.header().entry_type();
            anyhow::ensure!(
                kind.is_file() || kind.is_dir(),
                "archive links/special files rejected"
            );
            total = total
                .checked_add(entry.size())
                .context("archive overflow")?;
            anyhow::ensure!(total <= 128 * 1024 * 1024, "package exceeds 128 MiB");
            anyhow::ensure!(entry.unpack_in(&root)?, "unsafe archive path");
        }
    }
    Manifest::read(&root)?;
    Ok(temporary)
}
pub fn install(staged: &Path, source: &str, update: bool) -> Result<Manifest> {
    let m = Manifest::read(staged)?;
    let target = package_dir().join(&m.id);
    anyhow::ensure!(
        target.exists() == update,
        if update {
            "plugin is not installed"
        } else {
            "plugin ID is already installed"
        }
    );
    let mut state = saved()?;
    if update {
        let old = Manifest::read(&target)?;
        anyhow::ensure!(
            old.tools == m.tools
                && old.providers == m.providers
                && old.dependencies == m.dependencies,
            "permission surface changed: install under a new ID for review"
        );
    }
    std::fs::create_dir_all(package_dir())?;
    let replacement = tempfile::tempdir_in(package_dir())?;
    copy_tree(staged, &replacement.path().join("package"))?;
    let previous = replacement.path().join("previous");
    if target.exists() {
        std::fs::rename(&target, &previous)?;
    }
    if let Err(e) = std::fs::rename(replacement.path().join("package"), &target) {
        if previous.exists() {
            std::fs::rename(previous, &target)?;
        }
        return Err(e.into());
    }
    let settings = state.entry(m.id.clone()).or_default();
    settings.source = source.into();
    if !update {
        settings.enabled = false;
        settings.trusted = false;
    }
    save(&state)?;
    Ok(m)
}

/// Additive migration. Existing TOML files, grants, server IDs and runtime dependencies
/// remain untouched. Package descriptors and separate state are the only writes.
pub async fn migrate(cfg: &Config, worker: &Path, checkout: &Path) -> Result<()> {
    let registry = crate::registry::Registry::load_worker(cfg).await;
    let mut state = saved()?;
    std::fs::create_dir_all(package_dir())?;
    let groups = [
        "desktop-controls",
        "apps",
        "terminal",
        "coding-agents",
        "files",
        "git",
        "system-status",
        "clock",
        "location",
        "weather",
        "windows",
        "services",
        "network",
        "admin",
    ];
    for id in groups {
        let mut tools: Vec<_> = registry
            .tools
            .keys()
            .filter(|name| match id {
                "terminal" => name.starts_with("terminal."),
                "coding-agents" => name.starts_with("agent."),
                "files" => name.starts_with("files."),
                "git" => name.starts_with("git."),
                "system-status" => name.as_str() == "system.status",
                "location" => name.starts_with("location."),
                "weather" => name.starts_with("weather."),
                "apps" => name.starts_with("apps."),
                "windows" => ["windows.", "workspaces.", "monitors."].iter().any(|p| name.starts_with(p)),
                "services" => name.starts_with("services."),
                "network" => name.starts_with("network."),
                "admin" => name.starts_with("admin.") || name.starts_with("packages."),
                "desktop-controls" => name.starts_with("audio.") || name.as_str() == "system.power_profile",
                _ => false,
            })
            .cloned()
            .collect();
        if id == "desktop-controls" {
            tools.extend(
                assistant_core::CommandsFile::load()?
                    .command
                    .into_iter()
                    .filter(|c| c.tool != "app.launch")
                    .map(|c| c.tool),
            );
            tools.extend(["hypr.workspace".into(), "hypr.workspace_move".into()]);
        }
        if id == "apps" {
            tools.push("app.launch".into());
        }
        if id == "clock" {
            tools.push("clock.now".into());
        }
        tools.sort();
        tools.dedup();
        let m = Manifest {
            manifest_version: 1,
            id: id.into(),
            version: "1.0.0".into(),
            description: id.replace('-', " "),
            executable: worker.display().to_string(),
            args: vec![id.into()],
            dependencies: vec![],
            optional_dependencies: if id == "weather" {
                vec!["location".into()]
            } else {
                vec![]
            },
            tools,
            routes: vec![],
            providers: if ["clock", "location", "weather"].contains(&id) {
                vec![id.into()]
            } else {
                vec![]
            },
            settings_schema: empty_schema(),
        };
        write_migrated(
            &m,
            &mut state,
            match id {
                "location" => cfg.location.enabled,
                _ => true,
            },
        )?;
    }
    // Browser and configured MCP servers are adopted without dependency installation.
    if cfg.browser.enabled {
        let executable = cfg.browser.node.clone();
        let script = if cfg.browser.worker == "auto" {
            paths::data_dir()
                .join("browser/worker.mjs")
                .display()
                .to_string()
        } else {
            cfg.browser.worker.clone()
        };
        for id in ["browser", "web-search"] {
            let m = Manifest {
                manifest_version: 1,
                id: id.into(),
                version: "1.0.0".into(),
                description: "Browser and visible Google navigation".into(),
                executable: executable.clone(),
                args: vec![script.clone()],
                dependencies: vec![],
                optional_dependencies: vec![],
                tools: registry
                    .tools
                    .keys()
                    .filter(|n| {
                        if id == "browser" {
                            n.starts_with("browser.")
                        } else {
                            n.starts_with("web.")
                        }
                    })
                    .cloned()
                    .collect(),
                routes: vec![],
                providers: vec![],
                settings_schema: empty_schema(),
            };
            if !m.tools.is_empty() {
                write_migrated(&m, &mut state, true)?;
            }
        }
    }
    for (id, server) in &cfg.mcp.servers {
        let client = McpClient::spawn(id, &server.command, &server.args, &server.env).await?;
        let m = Manifest {
            manifest_version: 1,
            id: id.clone(),
            version: "1.0.0".into(),
            description: format!("Adopted configured MCP server {id}"),
            executable: server.command.clone(),
            args: server.args.clone(),
            dependencies: vec![],
            optional_dependencies: vec![],
            tools: client
                .list_tools()
                .await?
                .into_iter()
                .map(|t| format!("{id}.{}", t.name))
                .collect(),
            routes: vec![],
            providers: vec![],
            settings_schema: empty_schema(),
        };
        write_migrated(&m, &mut state, cfg.mcp.enabled)?;
    }
    for id in [
        "notes",
        "reminders",
        "routines",
        "projects",
        "capture",
        "calendar",
        "file-search",
        "media",
        "power",
        "history",
    ] {
        let folder = package_dir().join(id);
        if !folder.exists() {
            copy_tree(&checkout.join("abilities").join(id), &folder)?;
        }
        let m = Manifest::read(&folder)?;
        state.entry(m.id).or_insert(Saved {
            enabled: true,
            trusted: true,
            source: "bundled".into(),
            settings: empty_settings(),
        });
    }
    save(&state)
}
fn write_migrated(m: &Manifest, state: &mut BTreeMap<String, Saved>, enabled: bool) -> Result<()> {
    m.validate()?;
    let dir = package_dir().join(&m.id);
    let file = dir.join("plugin.toml");
    let text = toml::to_string_pretty(m)?;
    // First-party manifests are generated: refresh them so new tools reach existing installs.
    // A package the user installed under the same ID is left alone.
    let first_party = state.get(&m.id).is_none_or(|s| s.source == "migrated");
    if !dir.exists() || (first_party && std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str())) {
        std::fs::create_dir_all(&dir)?;
        assistant_core::config::atomic_write(&file, text.as_bytes())?;
    }
    state.entry(m.id.clone()).or_insert(Saved {
        enabled,
        trusted: true,
        source: "migrated".into(),
        settings: empty_settings(),
    });
    Ok(())
}
impl PluginManager {
    pub fn events(&self) -> Vec<(String, Value)> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, r)| r.saved.trusted && r.error.is_none() && r.client.healthy())
            .flat_map(|(id, r)| {
                r.client
                    .events()
                    .into_iter()
                    .map(|e| (id.clone(), e))
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    pub async fn ack_event(&self, id: &str, event: &str) -> Result<()> {
        let client = self
            .entries
            .lock()
            .unwrap()
            .get(id)
            .context("plugin disabled")?
            .client
            .clone();
        client
            .extension(
                "assistant/event_ack",
                json!({"version":1,"id":event}),
                &CancellationToken::new(),
            )
            .await?;
        Ok(())
    }
}
impl PluginManager {
    pub async fn apply(&self, next: BTreeMap<String, Saved>, cfg: &Config) -> Result<Value> {
        let previous = saved()?;
        anyhow::ensure!(
            next.keys()
                .all(|id| valid_id(id) && package_dir().join(id).is_dir()),
            "unknown plugin in draft"
        );
        for (id, s) in &next {
            let m = Manifest::read(&package_dir().join(id))?;
            jsonschema::validator_for(&m.settings_schema)?
                .validate(&s.settings)
                .map_err(|e| anyhow::anyhow!("{id}: {e}"))?;
            if s.enabled {
                anyhow::ensure!(
                    m.dependencies
                        .iter()
                        .all(|d| next.get(d).is_some_and(|s| s.enabled)),
                    "include dependents when disabling dependencies"
                );
            }
        }
        // Trust and source are host metadata, never accepted from a client draft.
        let next: BTreeMap<_, _> = next
            .into_iter()
            .map(|(id, mut s)| {
                s.trusted = previous.get(&id).is_some_and(|s| s.trusted);
                s.source = previous
                    .get(&id)
                    .map(|s| s.source.clone())
                    .unwrap_or_default();
                (id, s)
            })
            .collect();
        let mut disable: Vec<_> = previous
            .keys()
            .filter(|id| next.get(*id).is_none_or(|s| !s.enabled))
            .cloned()
            .collect();
        while !disable.is_empty() {
            let ready = disable
                .iter()
                .position(|id| {
                    !self
                        .entries
                        .lock()
                        .unwrap()
                        .values()
                        .any(|r| r.manifest.dependencies.contains(id))
                })
                .context("dependency cycle")?;
            let id = disable.remove(ready);
            self.change("disable", &id, cfg).await?;
        }
        save(&next)?;
        let mut enable: Vec<_> = next
            .iter()
            .filter(|(id, s)| {
                s.enabled
                    && previous
                        .get(*id)
                        .is_none_or(|p| !p.enabled || p.settings != s.settings)
            })
            .map(|(id, _)| id.clone())
            .collect();
        while !enable.is_empty() {
            let ready = enable
                .iter()
                .position(|id| {
                    Manifest::read(&package_dir().join(id)).is_ok_and(|m| {
                        m.dependencies
                            .iter()
                            .all(|d| self.entries.lock().unwrap().contains_key(d))
                    })
                })
                .context("dependency cycle")?;
            let id = enable.remove(ready);
            if let Err(e) = self.change("reload", &id, cfg).await {
                save(&previous)?;
                return Err(e);
            }
        }
        Ok(self.inspect())
    }
}

impl PluginManager {
    /// Report disconnections without automatic restarts.
    pub fn refresh_health(&self) -> bool {
        let mut changed = false;
        for r in self.entries.lock().unwrap().values_mut() {
            if r.error.is_none() && !r.client.healthy() {
                r.error = Some("MCP process disconnected; reload to retry".into());
                r.cancel.cancel();
                r.client.terminate();
                changed = true;
            }
        }
        changed
    }
    pub async fn cancel_jobs(&self) {
        let clients: Vec<_> = self
            .entries
            .lock()
            .unwrap()
            .values()
            .filter(|r| {
                r.saved.trusted
                    && r.manifest
                        .executable
                        .ends_with("nixe-assistant-plugin-worker")
            })
            .map(|r| r.client.clone())
            .collect();
        for client in clients {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                client.extension(
                    "assistant/cancel_jobs",
                    json!({"version":1}),
                    &CancellationToken::new(),
                ),
            )
            .await;
        }
    }
}
