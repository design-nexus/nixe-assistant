//! Newline-delimited MCP stdio transport. Each request has a deadline and cancellation.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicI64, Ordering},
    Arc, Mutex,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(default, rename = "outputSchema")]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub execution: Value,
}
pub struct McpClient {
    name: String,
    stdin: Arc<AsyncMutex<ChildStdin>>,
    req_id: AtomicI64,
    pending: Pending,
    reader: tokio::task::JoinHandle<()>,
    child: Mutex<Child>,
    alive: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<Value>>>,
}
struct RequestGuard {
    id: i64,
    pending: Pending,
    stdin: Arc<AsyncMutex<ChildStdin>>,
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if self.pending.lock().unwrap().remove(&self.id).is_some() {
            let stdin = self.stdin.clone();
            let id = self.id;
            tokio::spawn(async move {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1), write_message(&stdin, &json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"cancelled or timed out"}}))).await;
            });
        }
    }
}
async fn read_message(
    reader: &mut BufReader<tokio::process::ChildStdout>,
) -> Result<Option<String>> {
    let mut message = Vec::new();
    loop {
        let (part, complete, eof) = {
            let buffer = reader.fill_buf().await?;
            if buffer.is_empty() {
                (Vec::new(), false, true)
            } else {
                let len = buffer
                    .iter()
                    .position(|b| *b == b'\n')
                    .map(|i| i + 1)
                    .unwrap_or(buffer.len());
                if message.len() + len > 1024 * 1024 {
                    anyhow::bail!("MCP frame exceeds 1 MiB");
                }
                (buffer[..len].to_vec(), buffer[len - 1] == b'\n', false)
            }
        };
        if eof {
            if message.is_empty() {
                return Ok(None);
            }
            anyhow::bail!("incomplete MCP frame");
        }
        reader.consume(part.len());
        message.extend(part);
        if complete {
            return Ok(Some(String::from_utf8(message)?));
        }
    }
}
async fn write_message(stdin: &AsyncMutex<ChildStdin>, value: &Value) -> Result<()> {
    let mut raw = serde_json::to_vec(value)?;
    raw.push(b'\n');
    let mut input = stdin.lock().await;
    input.write_all(&raw).await?;
    input.flush().await?;
    Ok(())
}
impl McpClient {
    pub async fn spawn(
        name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .envs(env)
            .process_group(0)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawn MCP server {name}"))?;
        let stdin = Arc::new(AsyncMutex::new(child.stdin.take().context("MCP stdin")?));
        let stdout = child.stdout.take().context("MCP stdout")?;
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let pending_reader = pending.clone();
        let output = stdin.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let alive_reader = alive.clone();
        let events = Arc::new(Mutex::new(Vec::new()));
        let event_reader = events.clone();
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout);
            while let Ok(Some(line)) = read_message(&mut lines).await {
                if line.len() > 1024 * 1024 {
                    break;
                }
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    break;
                };
                if value["jsonrpc"] != "2.0" {
                    break;
                }
                if value["method"] == "notifications/assistant/event" && value.get("id").is_none() {
                    let mut events = event_reader.lock().unwrap();
                    if events.len() < 100 && value["params"]["version"] == 1 {
                        events.push(value["params"].clone());
                    }
                    continue;
                }
                if let (Some(id), Some(method)) = (value.get("id"), value.get("method")) {
                    let response = if method == "ping" {
                        json!({"jsonrpc":"2.0","id":id,"result":{}})
                    } else {
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Client capability not supported"}})
                    };
                    if write_message(&output, &response).await.is_err() {
                        break;
                    }
                    continue;
                }
                if let Some(id) = value["id"].as_i64() {
                    if value.get("method").is_some() {
                        let _ = write_message(&output, &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Client capability not supported"}})).await;
                    } else if let Some(tx) = pending_reader.lock().unwrap().remove(&id) {
                        let result = if let Some(err) = value.get("error") {
                            Err(anyhow::anyhow!("MCP protocol error: {err}"))
                        } else {
                            value.get("result").cloned().context("missing MCP result")
                        };
                        let _ = tx.send(result);
                    }
                }
            }
            alive_reader.store(false, Ordering::SeqCst);
            for (_, tx) in pending_reader.lock().unwrap().drain() {
                let _ = tx.send(Err(anyhow::anyhow!("MCP server disconnected")));
            }
        });
        let client = Self {
            name: name.into(),
            stdin,
            req_id: AtomicI64::new(1),
            pending,
            reader,
            child: Mutex::new(child),
            alive,
            events,
        };
        let init = client.request("initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"nixe-assistant","version":env!("CARGO_PKG_VERSION")}}), &CancellationToken::new()).await?;
        let version = init["protocolVersion"]
            .as_str()
            .context("missing MCP protocolVersion")?;
        if !["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"].contains(&version) {
            anyhow::bail!("unsupported MCP protocol version {version}");
        }
        if !init["capabilities"]["tools"].is_object() {
            anyhow::bail!("MCP server does not support tools");
        }
        write_message(
            &client.stdin,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await?;
        Ok(client)
    }
    pub fn healthy(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
    pub fn pid(&self) -> Option<u32> {
        self.child.lock().ok().and_then(|c| c.id())
    }
    pub fn events(&self) -> Vec<Value> {
        self.events.lock().unwrap().drain(..).collect()
    }
    pub fn terminate(&self) {
        self.alive.store(false, Ordering::SeqCst);
        if let Ok(mut child) = self.child.lock() {
            if let Some(pid) = child.id() {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.start_kill();
        }
    }
    pub async fn extension(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        self.request(method, params, cancel).await
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    async fn request(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        self.request_within(method, params, std::time::Duration::from_secs(15), cancel).await
    }
    async fn request_within(
        &self,
        method: &str,
        params: Value,
        deadline: std::time::Duration,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        if cancel.is_cancelled() {
            anyhow::bail!("cancelled");
        }
        let id = self.req_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let _guard = RequestGuard {
            id,
            pending: self.pending.clone(),
            stdin: self.stdin.clone(),
        };
        tokio::select! {
            _ = cancel.cancelled() => anyhow::bail!("MCP request cancelled"),
            result = tokio::time::timeout(deadline, async {
                write_message(&self.stdin,&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})).await?;
                rx.await.context("MCP response channel closed")?
            }) => result.context("MCP request timed out")?,
        }
    }
    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let mut out = Vec::new();
        let mut cursor = Value::Null;
        for _ in 0..32 {
            let params = if cursor.is_null() {
                json!({})
            } else {
                json!({"cursor":cursor})
            };
            let page = self
                .request("tools/list", params, &CancellationToken::new())
                .await?;
            out.extend(serde_json::from_value::<Vec<McpTool>>(
                page["tools"].clone(),
            )?);
            cursor = page.get("nextCursor").cloned().unwrap_or(Value::Null);
            if cursor.is_null() {
                return Ok(out);
            }
        }
        anyhow::bail!("MCP tool pagination exceeded limit")
    }
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        self.call_tool_meta(name, arguments, None, cancel).await
    }

    /// `tools/call` with request `_meta` (the host's grant for a first-party worker).
    pub async fn call_tool_meta(
        &self,
        name: &str,
        arguments: Value,
        meta: Option<Value>,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let mut params = json!({"name":name,"arguments":arguments});
        if let Some(meta) = meta {
            params["_meta"] = meta;
        }
        let result = self.request_within("tools/call", params, crate::admin::deadline(name), cancel).await?;
        if !result["content"].is_array() || result.get("isError").is_some_and(|v| !v.is_boolean()) {
            anyhow::bail!("invalid MCP tool result");
        }
        for block in result["content"].as_array().unwrap() {
            let valid = match block["type"].as_str() {
                Some("text") => block["text"].is_string(),
                Some("image" | "audio") => {
                    block["data"].is_string() && block["mimeType"].is_string()
                }
                Some("resource_link") => block["uri"].is_string() && block["name"].is_string(),
                Some("resource") => {
                    block["resource"]["uri"].is_string()
                        && (block["resource"]["text"].is_string()
                            || block["resource"]["blob"].is_string())
                }
                _ => false,
            };
            anyhow::ensure!(valid, "invalid MCP content block");
        }
        Ok(result)
    }
}
impl Drop for McpClient {
    fn drop(&mut self) {
        self.terminate();
        self.reader.abort();
        if let Ok(mut child) = self.child.lock() {
            let _ = child.start_kill();
        }
    }
}
