//! Explicit restart continuation. Never replay a call whose outcome is unknown.
use super::*;
pub(super) fn path() -> std::path::PathBuf {
    assistant_core::paths::state_dir().join("task-checkpoint.json")
}
fn files(value: &Value, paths: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                if ["path", "from", "to", "cwd"].contains(&key.as_str()) {
                    if let Some(p) = v.as_str().filter(|s| s.starts_with('/') || s.starts_with("~/")) {
                        paths.insert(p.to_string());
                    }
                }
                files(v, paths);
            }
        }
        Value::Array(values) => {
            for v in values {
                files(v, paths)
            }
        }
        _ => {}
    }
}
fn environment(cfg: &Config, catalog: Value, results: &[Value]) -> Value {
    let mut paths = std::collections::BTreeSet::new();
    files(&json!(results), &mut paths);
    let metadata:Vec<_>=paths.into_iter().map(|p|{
        let path=if let Some(tail)=p.strip_prefix("~/"){dirs::home_dir().unwrap_or_default().join(tail)}else{p.clone().into()};
        let m=std::fs::metadata(path).ok();json!({"path":p,"size":m.as_ref().map(|m|m.len()),"modified":m.and_then(|m|m.modified().ok()).and_then(|t|t.duration_since(UNIX_EPOCH).ok()).map(|d|d.as_nanos().to_string())})
    }).collect();
    let mut tools = catalog.as_array().cloned().unwrap_or_default();
    tools.sort_by_key(|t| t["name"].as_str().unwrap_or("").to_string());
    json!({"config":assistant_policy::digest(&serde_json::to_value(cfg).unwrap_or_default()),"catalog":assistant_policy::digest(&json!(tools)),"files":metadata})
}
impl App {
    pub(super) fn save_checkpoint(&self, task: &TaskRef, results: &[Value], in_flight: bool) -> Result<()> {
        let cfg = self.cfg();
        let data = json!({"version":1,"at_ms":now_ms(),"task":task.lock().unwrap().clone(),"results":results,"in_flight":in_flight,"environment":environment(&cfg,self.registry.catalog(&cfg),results)});
        let bytes = serde_json::to_vec(&data)?;
        anyhow::ensure!(bytes.len() < 1024 * 1024, "task checkpoint exceeds limit");
        assistant_core::config::atomic_write(&path(), &bytes)?;
        Ok(())
    }
    pub(super) async fn resume_checkpoint(&self, id: &str) -> Result<Value> {
        anyhow::ensure!(!self.turn_busy.load(Ordering::SeqCst), "Assistant is busy");
        let bytes = std::fs::read(path()).context("No resumable task checkpoint")?;
        anyhow::ensure!(bytes.len() < 1024 * 1024, "checkpoint too large");
        let data: Value = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            data["version"] == 1 && data["in_flight"] == false,
            "The last action's outcome is unknown; inspect it before starting a new request. It will not be replayed."
        );
        anyhow::ensure!(
            now_ms().saturating_sub(data["at_ms"].as_u64().unwrap_or(0)) < 24 * 3600 * 1000,
            "Task checkpoint expired"
        );
        let mut task: Task = serde_json::from_value(data["task"].clone())?;
        anyhow::ensure!(task.id == id, "No checkpoint for this task");
        let results: Vec<Value> = serde_json::from_value(data["results"].clone())?;
        let cfg = self.cfg();
        anyhow::ensure!(
            environment(&cfg, self.registry.catalog(&cfg), &results) == data["environment"],
            "Settings, capabilities or relevant files changed; start a new request after reviewing completed steps"
        );
        task.prepare_resume(cfg.agent.budget());
        let ask = task.request.clone();
        *self.paused_task.lock().unwrap() = Some((task, results));
        // The normal command path owns serialization, routing and all new approval checks.
        self.command_tx.send(Command::Say { text: ask }).await?;
        Ok(json!({"queued":true,"task_id":id,"approvals":"new approvals required by current policy"}))
    }
}
