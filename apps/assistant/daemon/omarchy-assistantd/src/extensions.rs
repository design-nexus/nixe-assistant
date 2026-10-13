//! Host-owned coordination. Plugin results cannot independently dispatch tools.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ActionRecord {
    id: String,
    tool: String,
    arguments: Value,
    #[serde(default)]
    owner: Option<String>,
    undo: Option<Value>,
    undone: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PowerSettings {
    pub profile: String,
    pub idle_seconds: u64,
}
impl Default for PowerSettings {
    fn default() -> Self {
        Self {
            profile: "normal".into(),
            idle_seconds: 300,
        }
    }
}
impl PowerSettings {
    pub fn load() -> Self {
        std::fs::read(assistant_core::paths::config_dir().join("power.json"))
            .ok()
            .and_then(|v| serde_json::from_slice::<Self>(&v).ok())
            .filter(|s| {
                matches!(s.profile.as_str(), "normal" | "battery") && s.idle_seconds <= 86400
            })
            .unwrap_or_default()
    }
}
pub(super) fn load_actions() -> Vec<ActionRecord> {
    std::fs::read(assistant_core::paths::data_dir().join("actions.json"))
        .ok()
        .and_then(|v| serde_json::from_slice(&v).ok())
        .unwrap_or_default()
}
struct ForwardCancellation(tokio::task::JoinHandle<()>);
impl Drop for ForwardCancellation {
    fn drop(&mut self) {
        self.0.abort();
    }
}
fn forward_cancel(owner: CancellationToken, target: CancellationToken) -> ForwardCancellation {
    ForwardCancellation(tokio::spawn(async move {
        owner.cancelled().await;
        target.cancel();
    }))
}
impl App {
    pub(super) fn remember_action(&self, name: &str, args: &Value, result: &Value) -> Result<()> {
        if result["isError"] == true
            || is_read_only(name)
            || name.ends_with(".undo")
            || name.starts_with("history.")
            || matches!(name, "routines.run" | "projects.action")
        {
            return Ok(());
        }
        let owner = self.registry.plugins.as_ref().and_then(|p| p.owner(name));
        let mut undo = result["structuredContent"]["undo"].clone();
        let inverse_owner = undo["tool"]
            .as_str()
            .and_then(|tool| self.registry.plugins.as_ref().and_then(|p| p.owner(tool)));
        if !valid_undo(name, &undo) || owner.is_none() || owner != inverse_owner {
            undo = Value::Null;
        }
        let mut records = self.actions.lock().unwrap();
        let id = format!("{}-{}", now_ms(), records.len());
        records.push(ActionRecord {
            id,
            tool: name.into(),
            arguments: args.clone(),
            owner,
            undo: (!undo.is_null()).then_some(undo),
            undone: false,
        });
        let excess = records.len().saturating_sub(100);
        records.drain(..excess);
        assistant_core::config::atomic_write(
            &assistant_core::paths::data_dir().join("actions.json"),
            &serde_json::to_vec_pretty(&*records)?,
        )?;
        Ok(())
    }
    pub(super) async fn host_extensions(
        &self,
        name: &str,
        mut result: Value,
        cancel: &CancellationToken,
        depth: usize,
        budget: usize,
    ) -> Result<Value> {
        if result["isError"] == true {
            return Ok(result);
        }
        // Only these bundled, trusted tools can request host operations. Retrieved text is never interpreted.
        if !self
            .registry
            .plugins
            .as_ref()
            .is_some_and(|p| p.trusted_tool(name))
        {
            return Ok(result);
        }
        if matches!(name, "routines.run" | "projects.action") {
            anyhow::ensure!(depth == 0, "nested coordinated actions are unsupported");
            let directive = &result["structuredContent"]["host_actions"];
            anyhow::ensure!(directive["version"] == 1, "unsupported action extension");
            let steps = directive["steps"]
                .as_array()
                .context("missing routine steps")?
                .clone();
            anyhow::ensure!(
                !steps.is_empty() && steps.len() <= budget,
                "one to four actions required"
            );
            // Validate the entire routine before the first effect. Never inherit the parent's grant.
            for step in &steps {
                let tool = step["tool"].as_str().context("missing action tool")?;
                anyhow::ensure!(
                    !matches!(tool, "routines.run" | "projects.action" | "history.undo")
                        && !tool.starts_with("power."),
                    "nested host actions are unsupported"
                );
                self.registry.validate(tool, &step["arguments"])?;
                anyhow::ensure!(
                    self.registry.policy(tool, &self.cfg()) != "deny",
                    "routine step denied: {tool}"
                );
            }
            let owner_generation = self
                .registry
                .plugins
                .as_ref()
                .and_then(|p| p.generation(name));
            let routine_cancel = cancel.child_token();
            let owner = self.registry.plugins.as_ref().unwrap().token(name);
            let _forward = forward_cancel(owner, routine_cancel.clone());
            let mut completed = Vec::new();
            for step in steps {
                anyhow::ensure!(
                    owner_generation
                        == self
                            .registry
                            .plugins
                            .as_ref()
                            .and_then(|p| p.generation(name)),
                    "routine plugin changed or disabled"
                );
                let value = Box::pin(self.dispatch_plugin_depth(
                    step["tool"].as_str().unwrap(),
                    step["arguments"].clone(),
                    &routine_cancel,
                    depth + 1,
                    1,
                ))
                .await;
                match value {
                    Ok(value) if value["isError"] != true => {
                        completed.push(json!({"tool":step["tool"],"result":value}))
                    }
                    other => {
                        let error = match other {
                            Ok(value) => value.to_string(),
                            Err(e) => e.to_string(),
                        };
                        return Ok(
                            json!({"isError":true,"structuredContent":{"completed":completed,"error":error,"acknowledgement":"Routine stopped after a failed or denied step. Completed actions remain in history."},"content":[{"type":"text","text":error}]}),
                        );
                    }
                }
            }
            result["structuredContent"]["completed"] = json!(completed);
        } else if name.starts_with("power.") {
            let _guard = self.reload_lock.lock().await;
            if matches!(name, "power.configure" | "power.battery" | "power.normal") {
                let directive = &result["structuredContent"]["host_power"];
                anyhow::ensure!(directive["version"] == 1, "unsupported power extension");
                let settings = PowerSettings {
                    profile: directive["profile"]
                        .as_str()
                        .context("missing power profile")?
                        .into(),
                    idle_seconds: directive["idle_seconds"]
                        .as_u64()
                        .context("missing idle time")?,
                };
                anyhow::ensure!(
                    matches!(settings.profile.as_str(), "normal" | "battery")
                        && settings.idle_seconds <= 86400,
                    "invalid power settings"
                );
                self.llm.set_battery(settings.profile == "battery").await?;
                assistant_core::config::atomic_write(
                    &assistant_core::paths::config_dir().join("power.json"),
                    &serde_json::to_vec_pretty(&settings)?,
                )?;
                *self.power.lock().unwrap() = settings;
            } else if name == "power.unload" {
                self.llm.unload().await?;
            }
            result["structuredContent"]["host"] = self.power_json();
        } else if name == "history.list" {
            result["structuredContent"]["actions"] =
                serde_json::to_value(&*self.actions.lock().unwrap())?;
        } else if name == "history.undo" {
            let _guard = self.undo_lock.lock().await;
            let action = self
                .actions
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|r| !r.undone)
                .cloned()
                .context("no reversible action to undo")?;
            let undo = action.undo.as_ref().context(
                "the most recent recorded change cannot be undone; inspect action history",
            )?;
            anyhow::ensure!(valid_undo(&action.tool, undo), "invalid saved undo");
            let undo_owner = self
                .registry
                .plugins
                .as_ref()
                .and_then(|p| p.owner(undo["tool"].as_str().unwrap()));
            anyhow::ensure!(
                action.owner.is_some() && action.owner == undo_owner,
                "original undo owner is unavailable or changed; enable the original plugin"
            );
            let value = Box::pin(self.dispatch_plugin_depth(
                undo["tool"].as_str().unwrap(),
                undo["arguments"].clone(),
                cancel,
                depth + 1,
                1,
            ))
            .await?;
            anyhow::ensure!(value["isError"] != true, "undo failed: {value}");
            let mut records = self.actions.lock().unwrap();
            if let Some(record) = records.iter_mut().find(|r| r.id == action.id) {
                record.undone = true;
            }
            assistant_core::config::atomic_write(
                &assistant_core::paths::data_dir().join("actions.json"),
                &serde_json::to_vec_pretty(&*records)?,
            )?;
            result["structuredContent"] =
                json!({"undone":action.id,"tool":action.tool,"acknowledgement":"Action undone."});
        }
        // Keep text and structured content consistent after host processing.
        result["content"] = json!([{"type":"text","text":result["structuredContent"].to_string()}]);
        Ok(result)
    }
    pub(super) fn power_json(&self) -> Value {
        json!({"settings":*self.power.lock().unwrap(),"model_loaded":self.llm.is_loaded(),"language_model_device":if self.llm.is_battery(){"cpu"}else{"configured"},"idle_seconds_elapsed":self.last_activity.lock().unwrap().elapsed().as_secs()})
    }
    pub(super) async fn power_tick(&self) {
        let Ok(_guard) = self.reload_lock.try_lock() else {
            return;
        };
        let seconds = self.power.lock().unwrap().idle_seconds;
        if seconds > 0
            && self.llm.is_loaded()
            && self.last_activity.lock().unwrap().elapsed().as_secs() >= seconds
            && !self.turn_busy.load(Ordering::SeqCst)
            && self.pending.lock().unwrap().is_empty()
            && self.session.lock().unwrap().mode == Mode::Wake
            && self
                .state
                .try_lock()
                .is_ok_and(|s| s.current() == AssistantState::Idle)
        {
            if let Err(e) = self.llm.unload().await {
                tracing::warn!(%e, "idle model unload failed");
            }
        }
    }
}
fn is_read_only(name: &str) -> bool {
    name.starts_with("capture.")
        || matches!(
            name,
            "clock.now"
                | "weather.lookup"
                | "location.current"
                | "system.status"
                | "git.status"
                | "files.read"
                | "web.search"
                | "web.read"
                | "power.status"
        )
        || [
            ".list",
            ".read",
            ".search",
            ".status",
            ".current",
            ".resolve",
            ".preview",
            ".players",
            ".applications",
            ".snapshot",
            ".tabs",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
        || name == "calendar.next"
}
fn valid_undo(source: &str, value: &Value) -> bool {
    value["version"] == 1
        && source
            .split_once('.')
            .is_some_and(|(prefix, _)| value["tool"].as_str() == Some(&format!("{prefix}.undo")))
        && value["arguments"].is_object()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn undo_cannot_dispatch_another_ability() {
        assert!(valid_undo(
            "notes.edit",
            &json!({"version":1,"tool":"notes.undo","arguments":{"token":"abc"}})
        ));
        assert!(!valid_undo(
            "notes.edit",
            &json!({"version":1,"tool":"terminal.run","arguments":{"argv":["sh"]}})
        ));
        assert!(!valid_undo(
            "notes.edit",
            &json!({"version":2,"tool":"notes.undo","arguments":{}})
        ));
    }
}
