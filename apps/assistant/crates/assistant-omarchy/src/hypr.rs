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
    /// Top-left corner in layout (logical) coordinates.
    #[serde(default)]
    pub at: [i64; 2],
    #[serde(default)]
    pub size: [i64; 2],
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
/// containing the words (any case), else the name's own words ("VS Code" finds class `code`
/// and "… - Visual Studio Code"). The caller decides what to do with none or several.
pub fn match_windows<'a>(clients: &'a [Client], query: &str) -> Vec<&'a Client> {
    let found = match_exact(clients, query);
    if !found.is_empty() {
        return found;
    }
    // Words people add that name no window.
    const FILLER: &[&str] = &["the", "window", "app", "application", "titled", "called", "named", "open"];
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !FILLER.contains(w))
        .map(str::to_string)
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let score = |c: &Client| {
        let class = c.class.to_lowercase();
        let last = class.rsplit('.').next().unwrap_or(&class).to_string();
        let title = c.title.to_lowercase();
        words.iter().filter(|w| **w == class || **w == last || title.split(|ch: char| !ch.is_alphanumeric()).any(|t| t == w.as_str())).count()
    };
    let best = clients.iter().map(score).max().unwrap_or(0);
    if best == 0 {
        return Vec::new();
    }
    clients.iter().filter(|c| score(c) == best).collect()
}

fn match_exact<'a>(clients: &'a [Client], query: &str) -> Vec<&'a Client> {
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

/// Press (`down`) or release a key with modifiers in one window, through Hyprland itself.
/// `mods` are Hyprland's names ("CTRL SHIFT"), `key` an xkb keysym name ("s", "End").
pub async fn send_key_state(address: &str, mods: &str, key: &str, down: bool) -> Result<()> {
    ensure!(valid_address(address), "invalid window address");
    ensure!(mods.chars().all(|c| c.is_ascii_uppercase() || c == ' '), "invalid modifiers");
    ensure!(!key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "invalid key");
    let state = if down { "down" } else { "up" };
    dispatch(format!(r#"hl.dsp.send_key_state({{ mods = "{mods}", key = "{key}", state = "{state}", window = "address:{address}" }})"#)).await
}

/// Put the cursor at a layout (logical) position.
pub async fn move_cursor(x: i32, y: i32) -> Result<()> {
    dispatch(format!("hl.dsp.cursor.move({{ x = {x}, y = {y} }})")).await
}

pub async fn focus_workspace(workspace: &str) -> Result<()> {
    let ws = workspace_selector(workspace)?;
    dispatch(format!(r#"hl.dsp.focus({{ workspace = "{ws}" }})"#)).await
}

/// A desktop phrase's action: a closed set, each value checked, run through the same
/// dispatchers as the window tools. Phrase files name these (`["hypr", "close_window"]`);
/// raw `hyprctl eval` from a file isn't run.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    CloseWindow,
    /// A number, `special:<name>`, `e+1` / `e-1` (next / previous) or `previous` (the last one).
    FocusWorkspace(String),
    /// The focused window to a workspace number or `special:<name>`; `follow` goes along.
    MoveToWorkspace(String, bool),
    ToggleSpecial(String),
    FloatToggle,
    ToggleSplit,
    FocusDirection(char),
    CycleNext,
    CyclePrevious,
}

fn special_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `workspace_selector`, plus the relative ones focusing allows.
fn focus_selector(input: &str) -> Result<String> {
    match input.trim() {
        s @ ("e+1" | "e-1" | "previous") => Ok(s.to_string()),
        s => workspace_selector(s),
    }
}

impl Action {
    /// From a phrase's argv after `hypr`.
    pub fn parse(args: &[&str]) -> Result<Self> {
        let one = || args.get(1).copied().context("the action needs a value");
        Ok(match args.first().copied().unwrap_or("") {
            "close_window" => Self::CloseWindow,
            "focus_workspace" => Self::FocusWorkspace(focus_selector(one()?)?),
            "move_to_workspace" => Self::MoveToWorkspace(
                workspace_selector(one()?)?,
                match args.get(2).copied() {
                    None | Some("stay") => false,
                    Some("follow") => true,
                    Some(_) => bail!("after the workspace: follow or stay"),
                },
            ),
            "toggle_special" => {
                let name = one()?;
                ensure!(special_name_ok(name), "invalid special workspace name");
                Self::ToggleSpecial(name.into())
            }
            "float_toggle" => Self::FloatToggle,
            "toggle_split" => Self::ToggleSplit,
            "focus_direction" => match one()? {
                d @ ("l" | "r" | "u" | "d") => Self::FocusDirection(d.chars().next().unwrap()),
                _ => bail!("direction is l, r, u or d"),
            },
            "cycle_next" => Self::CycleNext,
            "cycle_previous" => Self::CyclePrevious,
            other => bail!("unknown desktop action {other:?}"),
        })
    }

    /// The dispatcher, built only from checked values.
    fn lua(&self) -> String {
        match self {
            Self::CloseWindow => "hl.dsp.window.close()".into(),
            Self::FocusWorkspace(ws) => format!(r#"hl.dsp.focus({{ workspace = "{ws}" }})"#),
            Self::MoveToWorkspace(ws, follow) => format!(r#"hl.dsp.window.move({{ workspace = "{ws}", follow = {follow} }})"#),
            Self::ToggleSpecial(name) => format!(r#"hl.dsp.workspace.toggle_special("{name}")"#),
            Self::FloatToggle => r#"hl.dsp.window.float({ action = "toggle" })"#.into(),
            Self::ToggleSplit => r#"hl.dsp.layout("togglesplit")"#.into(),
            Self::FocusDirection(d) => format!(r#"hl.dsp.focus({{ direction = "{d}" }})"#),
            Self::CycleNext => "hl.dsp.window.cycle_next()".into(),
            Self::CyclePrevious => "hl.dsp.window.cycle_next({ next = false })".into(),
        }
    }
}

/// Re-read Hyprland until `check` holds, for up to 1.5 s.
async fn settled(check: impl Fn(Option<&Client>, Option<&Workspace>, &[Client]) -> bool) -> bool {
    for _ in 0..15 {
        let (w, ws, cs) = (active_window().await.ok().flatten(), active_workspace().await.ok(), clients().await.unwrap_or_default());
        if check(w.as_ref(), ws.as_ref(), &cs) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

/// Run a phrase's action and check it took: the window gone or moved, the workspace or focus
/// changed, floating flipped. The reply says when it couldn't be confirmed.
pub async fn run_action(args: &[&str]) -> Result<String> {
    let action = Action::parse(args)?;
    let window = active_window().await.ok().flatten();
    let workspace = active_workspace().await.ok();
    if matches!(action, Action::CloseWindow | Action::MoveToWorkspace(..) | Action::FloatToggle | Action::ToggleSplit) {
        ensure!(window.is_some(), "no window is focused");
    }
    dispatch(action.lua()).await?;
    let before_ws = workspace.as_ref().map(|w| w.name.clone()).unwrap_or_default();
    let before_win = window.as_ref().map(|w| w.address.clone()).unwrap_or_default();
    let verified = match &action {
        Action::CloseWindow => settled(|_, _, cs: &[Client]| cs.iter().all(|c| c.address != before_win)).await,
        Action::FocusWorkspace(ws) if ws.starts_with("special:") || ws.parse::<i64>().is_ok() => {
            let target = ws.clone();
            settled(move |_, w: Option<&Workspace>, _| w.is_some_and(|w| w.name == target || w.id.to_string() == target)).await
        }
        Action::FocusWorkspace(_) => settled(|_, w: Option<&Workspace>, _| w.is_some_and(|w| w.name != before_ws)).await,
        Action::MoveToWorkspace(ws, _) => {
            let target = ws.clone();
            settled(move |_, _, cs: &[Client]| cs.iter().any(|c| c.address == before_win && (c.workspace.name == target || c.workspace.id.to_string() == target))).await
        }
        Action::FloatToggle => {
            let was = window.as_ref().is_some_and(|w| w.floating);
            settled(move |_, _, cs: &[Client]| cs.iter().any(|c| c.address == before_win && c.floating != was)).await
        }
        Action::FocusDirection(_) | Action::CycleNext | Action::CyclePrevious => settled(|w: Option<&Client>, _, _| w.is_some_and(|w| w.address != before_win)).await,
        // Nothing to read back: the special workspace's visibility and the split aren't reported.
        Action::ToggleSpecial(_) | Action::ToggleSplit => true,
    };
    Ok(if verified { "done".into() } else { "sent, but the change couldn't be confirmed".into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrase_actions_are_closed_and_checked() {
        assert_eq!(Action::parse(&["close_window"]).unwrap().lua(), "hl.dsp.window.close()");
        assert_eq!(Action::parse(&["focus_workspace", "e+1"]).unwrap().lua(), r#"hl.dsp.focus({ workspace = "e+1" })"#);
        assert_eq!(Action::parse(&["focus_workspace", "3"]).unwrap(), Action::FocusWorkspace("3".into()));
        assert_eq!(Action::parse(&["move_to_workspace", "special:scratchpad"]).unwrap().lua(), r#"hl.dsp.window.move({ workspace = "special:scratchpad", follow = false })"#);
        assert_eq!(Action::parse(&["move_to_workspace", "4", "follow"]).unwrap().lua(), r#"hl.dsp.window.move({ workspace = "4", follow = true })"#);
        assert!(Action::parse(&["move_to_workspace", "4", "true"]).is_err());
        assert_eq!(Action::parse(&["focus_direction", "l"]).unwrap().lua(), r#"hl.dsp.focus({ direction = "l" })"#);
        assert_eq!(Action::parse(&["toggle_special", "scratchpad"]).unwrap().lua(), r#"hl.dsp.workspace.toggle_special("scratchpad")"#);
        for bad in [
            &["focus_workspace", r#"1" }) hl.dsp.exec_cmd("rm"#][..],
            &["move_to_workspace", "e+1"],
            &["toggle_special", "a\"b"],
            &["focus_direction", "left"],
            &["exec", "foot"],
            &["focus_workspace"],
            &[],
        ] {
            assert!(Action::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn matches_by_name_words() {
        let c = |class: &str, title: &str| -> Client {
            serde_json::from_value(serde_json::json!({"address":"0x1","class":class,"title":title,"workspace":{"id":1,"name":"1"}})).unwrap()
        };
        let all = vec![
            c("code", "bench.md - Visual Studio Code"),
            c("io.github.design_nexus.nixe.Notepad", "notes.txt — Notepad"),
            c("firefox", "Bench search — Mozilla Firefox"),
        ];
        let first = |q: &str| match_windows(&all, q).first().map(|w| w.class.clone());
        assert_eq!(first("VS Code").as_deref(), Some("code"));
        assert_eq!(first("Visual Studio Code").as_deref(), Some("code"));
        assert_eq!(first("the Notepad window").as_deref(), Some("io.github.design_nexus.nixe.Notepad"));
        assert_eq!(first("notepad").as_deref(), Some("io.github.design_nexus.nixe.Notepad"));
        assert_eq!(first("Firefox window titled Bench search").as_deref(), Some("firefox"));
        assert!(match_windows(&all, "Spotify").is_empty());
    }

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
