//! First-party controls use owner-only Unix IPC, independently of HTTP enablement.
use anyhow::{Context, Result};
use assistant_ipc::{request, send_command, Command, Control};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonStatus {
    pub ok: bool,
    pub state: String,
    pub name: String,
    pub model: String,
    pub model_loaded: bool,
    pub silero_loaded: bool,
    pub tts_device: String,
    pub tts_variant: String,
    pub tts_voice: String,
    pub muted: bool,
    pub running_settings: Value,
    pub restart_required: Vec<String>,
    pub pending_approvals: std::collections::BTreeMap<String, Value>,
    pub tool_mode: bool,
    pub session: Value,
    pub plugins: Value,
    pub mcp_tools: Value,
}
#[derive(Clone)]
pub struct DaemonClient;
impl DaemonClient {
    pub fn new(_: u16) -> Self {
        Self
    }
    pub async fn fetch_status(&self) -> Result<DaemonStatus> {
        Ok(serde_json::from_value(
            request(Control::Status, 1200).await?,
        )?)
    }
    pub async fn reload(&self) -> Result<Value> {
        request(Control::Reload, 30_000).await
    }
    pub async fn toggle_mute(&self, target: Option<bool>) -> Result<()> {
        send_command(&Command::Mute { muted: target }).await
    }
    pub async fn is_service_active() -> bool {
        Self::service("is-active").await.is_ok()
    }
    pub async fn start_service() -> Result<()> {
        Self::service("start").await
    }
    pub async fn stop_service() -> Result<()> {
        Self::service("stop").await
    }
    pub async fn restart_service() -> Result<()> {
        Self::service("restart").await
    }
    async fn service(action: &str) -> Result<()> {
        let mut cmd = tokio::process::Command::new("systemctl");
        cmd.args(["--user", action, "nixe-assistantd.service"])
            .kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(20), cmd.output())
            .await
            .context("systemctl timed out")??;
        if !output.status.success() {
            anyhow::bail!(
                "systemctl {action} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
}
