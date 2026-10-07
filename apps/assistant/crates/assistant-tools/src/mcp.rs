//! Model Context Protocol (MCP) stdio client implementation.
//!
//! Connects to local MCP servers over stdio via JSON-RPC 2.0.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Value,
}

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: i64,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcNotification<'a> {
    jsonrpc: &'static str,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<i64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub struct McpClient {
    name: String,
    stdin: Arc<Mutex<ChildStdin>>,
    req_id: AtomicI64,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>,
    _reader_task: tokio::task::JoinHandle<()>,
    _child: Arc<Mutex<Child>>,
}

impl McpClient {
    pub async fn spawn(
        name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().with_context(|| {
            format!("failed to spawn MCP server '{name}' via command '{command}'")
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to open stdin for MCP server '{name}'"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to open stdout for MCP server '{name}'"))?;

        let stdin = Arc::new(Mutex::new(stdin));
        let pending = Arc::new(Mutex::new(HashMap::<i64, oneshot::Sender<Result<Value>>>::new()));
        let pending_reader = pending.clone();

        let reader_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(trimmed) {
                    if let Some(id) = resp.id {
                        let mut map = pending_reader.lock().await;
                        if let Some(tx) = map.remove(&id) {
                            if let Some(err) = resp.error {
                                let _ = tx.send(Err(anyhow::anyhow!(
                                    "MCP error {}: {}",
                                    err.code,
                                    err.message
                                )));
                            } else {
                                let _ = tx.send(Ok(resp.result.unwrap_or(Value::Null)));
                            }
                        }
                    }
                }
            }
        });

        let client = Self {
            name: name.to_string(),
            stdin,
            req_id: AtomicI64::new(1),
            pending,
            _reader_task: reader_task,
            _child: Arc::new(Mutex::new(child)),
        };

        client.initialize().await?;
        Ok(client)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.req_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };
        let mut raw = serde_json::to_vec(&req)?;
        raw.push(b'\n');

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(&raw).await.context("write to MCP stdin")?;
            stdin.flush().await.context("flush MCP stdin")?;
        }

        match tokio::time::timeout(Duration::from_secs(15), rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => anyhow::bail!("MCP server response channel dropped"),
            Err(_) => {
                let mut pending = self.pending.lock().await;
                pending.remove(&id);
                anyhow::bail!("MCP request '{method}' timed out after 15s");
            }
        }
    }

    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0",
            method,
            params,
        };
        let mut raw = serde_json::to_vec(&notif)?;
        raw.push(b'\n');

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&raw).await.context("write to MCP stdin")?;
        stdin.flush().await.context("flush MCP stdin")?;
        Ok(())
    }

    async fn initialize(&self) -> Result<()> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "nyx",
                "version": "0.1.0"
            }
        });

        self.send_request("initialize", Some(params)).await?;
        self.send_notification("notifications/initialized", None).await?;
        tracing::info!(server = %self.name, "MCP server initialized");
        Ok(())
    }

    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let res = self.send_request("tools/list", Some(serde_json::json!({}))).await?;
        let tools_val = res.get("tools").cloned().unwrap_or(Value::Array(vec![]));
        let tools: Vec<McpTool> = serde_json::from_value(tools_val).context("parse MCP tools")?;
        Ok(tools)
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });
        let res = self.send_request("tools/call", Some(params)).await?;
        if let Some(content) = res.get("content").and_then(|c| c.as_array()) {
            let mut out = String::new();
            for item in content {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
            }
            Ok(out)
        } else {
            Ok(res.to_string())
        }
    }
}
