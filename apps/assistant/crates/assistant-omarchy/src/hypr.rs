//! Hyprland over its request socket, and window dispatches built from validated values.
//!
//! Reads (`j/clients`, `j/activewindow`, …) go straight to the socket. Changes go through
//! `hyprctl eval` with the Lua dispatchers Omarchy uses; every value spliced into the Lua is
//! checked first (window addresses are hex, workspaces are numbers or `special:<name>`).

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceRef {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Client {
    pub address: String,
    pub class: String,
    pub title: String,
    pub workspace: WorkspaceRef,
    #[serde(default)]
    pub monitor: i64,
    #[serde(default)]
    pub pid: i64,
    #[serde(rename = "focusHistoryID", default)]
    pub focus_history_id: i64,
    #[serde(default)]
    pub floating: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub monitor: String,
    #[serde(default)]
    pub windows: i64,
}

/// `$XDG_RUNTIME_DIR/hypr/<instance>/.socket.sock`, using the signature from the
/// environment or else the most recently started instance.
pub fn socket_path() -> Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR unset")?;
    let base = PathBuf::from(runtime).join("hypr");
    if let Some(sig) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
        let path = base.join(sig).join(".socket.sock");
        if path.exists() {
            return Ok(path);
        }
    }
    let newest = std::fs::read_dir(&base)
        .context("Hyprland isn't running")?
        .flatten()
        .map(|e| e.path().join(".socket.sock"))
        .filter(|p| p.exists())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());
    newest.context("no Hyprland socket")
}

/// One request on the socket; the reply is everything Hyprland writes before closing.
pub async fn request(cmd: &str) -> Result<String> {
    let mut stream = tokio::net::UnixStream::connect(socket_path()?).await.context("connect to Hyprland")?;
    stream.write_all(cmd.as_bytes()).await?;
    let mut out = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), stream.read_to_end(&mut out))
        .await
        .context("Hyprland didn't answer")??;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

pub async fn json(what: &str) -> Result<Value> {
    let text = request(&format!("j/{what}")).await?;
    serde_json::from_str(&text).with_context(|| format!("unexpected Hyprland reply to {what}"))
}

pub async fn clients() -> Result<Vec<Client>> {
    Ok(serde_json::from_value(json("clients").await?)?)
}

pub async fn active_window() -> Result<Option<Client>> {
    let v = json("activewindow").await?;
    Ok(if v.get("address").is_some() { Some(serde_json::from_value(v)?) } else { None })
}

pub async fn workspaces() -> Result<Vec<Workspace>> {
    Ok(serde_json::from_value(json("workspaces").await?)?)
}

pub async fn active_workspace() -> Result<Workspace> {
    Ok(serde_json::from_value(json("activeworkspace").await?)?)
}

/// Name, resolution, position, scale, focus and active workspace of each monitor.
pub async fn monitors() -> Result<Vec<Value>> {
    let all = json("monitors").await?;
    Ok(all
        .as_array()
        .context("unexpected monitors reply")?
        .iter()
        .map(|m| {
            serde_json::json!({
                "name": m["name"], "description": m["description"], "width": m["width"],
                "height": m["height"], "x": m["x"], "y": m["y"], "scale": m["scale"],
                "focused": m["focused"], "active_workspace": m["activeWorkspace"],
            })
        })
        .collect())
}

pub fn valid_address(address: &str) -> bool {
    address.len() > 2 && address.len() <= 18 && address.starts_with("0x") && address[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// A workspace as Hyprland names it: a number, or `special:<name>`.
pub fn workspace_selector(input: &str) -> Result<String> {
    let input = input.trim();
    if let Ok(n) = input.parse::<i64>() {
        ensure!((1..=99).contains(&n), "workspace numbers are 1 to 99");
        return Ok(n.to_string());
    }
    if let Some(name) = input.strip_prefix("special:") {
        ensure!(
            !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "invalid special workspace name"
        );
        return Ok(input.to_string());
    }
    bail!("a workspace is a number or special:<name>")
}

/// Windows matching `query`: an exact address, else an exact class (any case), else a title
/// containing the words (any case). The caller decides what to do with none or several.
pub fn match_windows<'a>(clients: &'a [Client], query: &str) -> Vec<&'a Client> {
    let q = query.trim();
    if valid_address(q) {
        return clients.iter().filter(|c| c.address == q).collect();
    }
    let lower = q.to_lowercase();
    let by_class: Vec<_> = clients
        .iter()
        .filter(|c| c.class.to_lowercase() == lower || c.class.rsplit('.').next().is_some_and(|s| s.to_lowercase() == lower))
        .collect();
    if !by_class.is_empty() {
        return by_class;
    }
    clients.iter().filter(|c| c.title.to_lowercase().contains(&lower)).collect()
}

async fn dispatch(lua: String) -> Result<()> {
    crate::hyprctl(&["eval", &format!("hl.dispatch({lua})")]).await.map(|_| ())
}

pub async fn focus_window(address: &str) -> Result<()> {
    ensure!(valid_address(address), "invalid window address");
    dispatch(format!(r#"hl.dsp.focus({{ window = "address:{address}" }})"#)).await
}

pub async fn move_window(address: &str, workspace: &str) -> Result<()> {
    ensure!(valid_address(address), "invalid window address");
    let ws = workspace_selector(workspace)?;
    dispatch(format!(r#"hl.dsp.window.move({{ workspace = "{ws}", follow = false, window = "address:{address}" }})"#)).await
}

pub async fn close_window(address: &str) -> Result<()> {
    ensure!(valid_address(address), "invalid window address");
    dispatch(format!(r#"hl.dsp.window.close({{ window = "address:{address}" }})"#)).await
}

pub async fn focus_workspace(workspace: &str) -> Result<()> {
    let ws = workspace_selector(workspace)?;
    dispatch(format!(r#"hl.dsp.focus({{ workspace = "{ws}" }})"#)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Client> {
        serde_json::from_str(
            r#"[
            {"address":"0x56494d771890","class":"io.github.design_nexus.nixe.Glimpse","title":"Glimpse","workspace":{"id":3,"name":"3"},"monitor":0,"pid":10,"focusHistoryID":0,"floating":false,"extra":1},
            {"address":"0xabc","class":"Alacritty","title":"nvim notes.md","workspace":{"id":1,"name":"1"},"monitor":0,"pid":11,"focusHistoryID":1,"floating":false},
            {"address":"0xdef","class":"Alacritty","title":"htop","workspace":{"id":2,"name":"2"},"monitor":0,"pid":12,"focusHistoryID":2,"floating":true}
        ]"#,
        )
        .unwrap()
    }

    #[test]
    fn matching() {
        let c = fixture();
        assert_eq!(match_windows(&c, "0xabc").len(), 1);
        assert_eq!(match_windows(&c, "alacritty").len(), 2, "ambiguous class");
        assert_eq!(match_windows(&c, "glimpse")[0].address, "0x56494d771890", "last class segment");
        assert_eq!(match_windows(&c, "HTOP")[0].address, "0xdef", "title");
        assert!(match_windows(&c, "firefox").is_empty());
        assert!(match_windows(&c, "0x999").is_empty());
    }

    #[test]
    fn validation() {
        assert!(valid_address("0x56494d771890"));
        assert!(!valid_address("0x") && !valid_address("56494d") && !valid_address("0xzz") && !valid_address("0x1\" })"));
        assert_eq!(workspace_selector(" 3 ").unwrap(), "3");
        assert_eq!(workspace_selector("special:scratchpad").unwrap(), "special:scratchpad");
        for bad in ["0", "100", "e+1", "special:", "special:a\"b", "3\")"] {
            assert!(workspace_selector(bad).is_err(), "{bad}");
        }
    }
}
