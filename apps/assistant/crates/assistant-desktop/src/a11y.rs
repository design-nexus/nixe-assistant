//! The accessibility tree (AT-SPI) of an app window: what is on it, and acting on it
//! directly (press a button, set a field's text, focus) without synthetic input.
//!
//! Element ids (`e1`, `e2`, …) come from the latest tree read and stay valid until the next
//! one; acting on an id re-checks that the element still exists. Extents are relative to
//! the window: on Wayland, AT-SPI can't give screen positions, so callers add the window's
//! position from Hyprland ([`crate::Rect::offset`]).

use crate::Rect;
use anyhow::{bail, Context, Result};
use assistant_omarchy::hypr;
use atspi::connection::AccessibilityConnection;
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::{CoordType, State};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Roles a person can operate; these always make it into the tree.
const INTERACTIVE: &[&str] = &[
    "push button", "toggle button", "check box", "radio button", "menu item", "check menu item", "radio menu item",
    "entry", "password text", "text", "combo box", "link", "page tab", "list item", "tree item", "switch", "spin button",
    "slider", "menu", "button", "toggle", "search box", "editbar", "table cell", "icon",
];
/// Roles that carry readable text worth showing even though they can't be operated.
const TEXTUAL: &[&str] = &["label", "heading", "static", "paragraph", "status bar", "alert", "dialog", "frame", "page tab list"];
/// Nodes looked at, and elements returned, at most: a web page can have tens of thousands.
const MAX_VISITED: usize = 2500;
const MAX_ELEMENTS: usize = 150;

#[derive(Debug, Clone, Serialize)]
pub struct Element {
    pub id: String,
    pub role: String,
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Rect>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tree {
    pub app: String,
    pub window: String,
    pub elements: Vec<Element>,
    /// The tree was bigger than what was read.
    pub truncated: bool,
}

/// Where each id of the latest tree lives on the accessibility bus.
static IDS: Mutex<Option<HashMap<String, (String, String)>>> = Mutex::new(None);

/// Turn on the session's accessibility switch (what a screen reader sets), so Firefox,
/// Chromium and Electron apps expose their trees. Apps started earlier need a restart.
/// Returns whether it was off.
pub async fn enable() -> Result<bool> {
    let was = atspi::connection::read_session_accessibility().await.unwrap_or(false);
    if !was {
        atspi::connection::set_session_accessibility(true).await.context("turning on accessibility")?;
        tracing::info!("session accessibility turned on for computer use");
    }
    Ok(!was)
}

async fn connect() -> Result<AccessibilityConnection> {
    AccessibilityConnection::new()
        .await
        .context("the accessibility bus isn't reachable (is at-spi-dbus-bus running?)")
}

/// The process id behind a bus name on the accessibility bus.
async fn pid_of(conn: &zbus::Connection, bus: &str) -> Option<u32> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = zbus::names::BusName::try_from(bus.to_string()).ok()?;
    dbus.get_connection_unix_process_id(name).await.ok()
}

fn state_names(set: atspi::StateSet) -> Vec<&'static str> {
    let keep = [
        (State::Focused, "focused"),
        (State::Checked, "checked"),
        (State::Pressed, "pressed"),
        (State::Selected, "selected"),
        (State::Expanded, "expanded"),
        (State::Editable, "editable"),
        (State::Active, "active"),
    ];
    let mut out: Vec<&'static str> = keep.iter().filter(|(s, _)| set.contains(*s)).map(|(_, n)| *n).collect();
    if !set.contains(State::Enabled) && !set.contains(State::Sensitive) {
        out.push("disabled");
    }
    out
}

/// The window's root accessible: the app whose process owns the window, then its frame
/// whose name matches the window title (or the active one).
async fn window_root<'a>(conn: &'a zbus::Connection, root: &AccessibleProxy<'a>, window: &hypr::Client) -> Result<(AccessibleProxy<'a>, String)> {
    for app in root.get_children().await? {
        let Some(bus) = app.name_as_str().map(str::to_string) else { continue };
        if pid_of(conn, &bus).await != Some(window.pid as u32) {
            continue;
        }
        let app = app.into_accessible_proxy(conn).await?;
        let app_name = app.name().await.unwrap_or_default();
        let mut fallback = None;
        for frame in app.get_children().await? {
            let frame = frame.into_accessible_proxy(conn).await?;
            let name = frame.name().await.unwrap_or_default();
            if !window.title.is_empty() && name == window.title {
                return Ok((frame, app_name));
            }
            let active = frame.get_state().await.is_ok_and(|s| s.contains(State::Active));
            if active || fallback.is_none() {
                fallback = Some(frame);
            }
        }
        if let Some(frame) = fallback {
            return Ok((frame, app_name));
        }
    }
    bail!(
        "“{}” ({}) isn't on the accessibility bus: it was started before accessibility was on, or doesn't support it; restart it, or use screen.marks",
        window.title,
        window.class
    )
}

/// Read the window's tree: operable elements first, readable text after, capped.
pub async fn tree(window: &hypr::Client) -> Result<Tree> {
    let a11y = connect().await?;
    let conn = a11y.connection();
    let root = a11y.root_accessible_on_registry().await?;
    let (frame, app) = window_root(conn, &root, window).await?;
    let mut queue = VecDeque::from([frame]);
    let mut visited = 0usize;
    let mut operable = Vec::new();
    let mut readable = Vec::new();
    let mut places = Vec::new();
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if visited > MAX_VISITED {
            break;
        }
        let state = node.get_state().await.unwrap_or_default();
        // Hidden subtrees (closed menus, other tabs) are skipped whole.
        if visited > 1 && !state.contains(State::Showing) && !state.contains(State::Visible) {
            continue;
        }
        let role = node.get_role_name().await.unwrap_or_default();
        let name = node.name().await.unwrap_or_default();
        let is_operable = INTERACTIVE.contains(&role.as_str());
        if is_operable || (TEXTUAL.contains(&role.as_str()) && !name.trim().is_empty()) {
            let proxies = node.proxies().await.ok();
            let bounds = match &proxies {
                Some(p) => match p.component().await {
                    Ok(c) => c.get_extents(CoordType::Window).await.ok().map(|(x, y, w, h)| Rect { x, y, w, h }),
                    Err(_) => None,
                },
                None => None,
            };
            let actions = match &proxies {
                Some(p) => match p.action().await {
                    Ok(a) => a.get_actions().await.map(|v| v.into_iter().map(|a| a.name).collect()).unwrap_or_default(),
                    Err(_) => Vec::new(),
                },
                None => Vec::new(),
            };
            // Short text content for fields (what a person sees in them).
            let text = if is_operable && name.trim().is_empty() {
                match &proxies {
                    Some(p) => match p.text().await {
                        Ok(t) => {
                            let n = t.character_count().await.unwrap_or(0).min(200);
                            t.get_text(0, n).await.ok().filter(|s| !s.is_empty())
                        }
                        Err(_) => None,
                    },
                    None => None,
                }
            } else {
                None
            };
            let element = Element { id: String::new(), role, name: name.trim().to_string(), states: state_names(state), bounds, actions, text };
            let place = (node.inner().destination().to_string(), node.inner().path().to_string());
            if is_operable {
                operable.push((element, place));
            } else {
                readable.push((element, place));
            }
        }
        if let Ok(children) = node.get_children().await {
            for child in children.into_iter().take(400) {
                if let Ok(c) = child.into_accessible_proxy(conn).await {
                    queue.push_back(c);
                }
            }
        }
    }
    let truncated = visited > MAX_VISITED || operable.len() + readable.len() > MAX_ELEMENTS;
    let mut elements = Vec::new();
    for (i, (mut e, place)) in operable.into_iter().chain(readable).take(MAX_ELEMENTS).enumerate() {
        e.id = format!("e{}", i + 1);
        places.push((e.id.clone(), place));
        elements.push(e);
    }
    *IDS.lock().unwrap() = Some(places.into_iter().collect());
    Ok(Tree { app, window: window.title.clone(), elements, truncated })
}

/// Whether a person can operate the element (a button, item, field…), judged by its role:
/// labels expose clipboard actions too, so actions alone don't say.
pub fn operable(e: &Element) -> bool {
    INTERACTIVE.contains(&e.role.as_str())
}

/// Find elements of the latest-style tree by name (case-insensitive substring) or role.
pub fn find<'a>(tree: &'a Tree, text: Option<&str>, role: Option<&str>) -> Vec<&'a Element> {
    let text = text.map(str::to_lowercase);
    tree.elements
        .iter()
        .filter(|e| text.as_ref().is_none_or(|t| e.name.to_lowercase().contains(t) || e.text.as_ref().is_some_and(|x| x.to_lowercase().contains(t))))
        .filter(|e| role.is_none_or(|r| e.role == r))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// The element's own click/press/activate/toggle action.
    Press,
    Focus,
    SetText,
}

/// Action names that mean "operate it", best first.
const PRESS_ACTIONS: &[&str] = &["click", "press", "activate", "toggle", "jump", "open", "select"];

/// Act on an element of the latest tree. Returns a short description of what was done.
pub async fn act(id: &str, act: Act, text: Option<&str>) -> Result<String> {
    let (bus, path) = IDS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(id).cloned())
        .with_context(|| format!("no element {id}; read the tree again (ui.tree)"))?;
    let a11y = connect().await?;
    let node = AccessibleProxy::builder(a11y.connection())
        .destination(bus)?
        .path(path)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    let name = node.name().await.context("the element is gone; read the tree again")?;
    let proxies = node.proxies().await?;
    match act {
        Act::Press => {
            let action = proxies.action().await.context("it has no action to press")?;
            let names: Vec<String> = action.get_actions().await?.into_iter().map(|a| a.name.to_lowercase()).collect();
            let index = PRESS_ACTIONS
                .iter()
                .find_map(|want| names.iter().position(|n| n == want))
                .or(if names.is_empty() { None } else { Some(0) })
                .context("it has no action to press")?;
            anyhow::ensure!(action.do_action(index as i32).await?, "the app refused the action");
            Ok(format!("{} “{name}”", names[index]))
        }
        Act::Focus => {
            let c = proxies.component().await.context("it can't take focus")?;
            anyhow::ensure!(c.grab_focus().await?, "the app refused focus");
            Ok(format!("focused “{name}”"))
        }
        Act::SetText => {
            let text = text.context("set_text needs text")?;
            let e = proxies.editable_text().await.context("it isn't an editable text field")?;
            anyhow::ensure!(e.set_text_contents(text).await?, "the app refused the text");
            Ok(format!("set the text of “{name}”"))
        }
    }
}

/// An element's current states and text, to check what an action changed.
pub async fn snapshot(id: &str) -> Result<(Vec<&'static str>, Option<String>)> {
    let (bus, path) = IDS.lock().unwrap().as_ref().and_then(|m| m.get(id).cloned()).context("no such element")?;
    let a11y = connect().await?;
    let node = AccessibleProxy::builder(a11y.connection())
        .destination(bus)?
        .path(path)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    let states = state_names(node.get_state().await?);
    let text = match node.proxies().await?.text().await {
        Ok(t) => {
            let n = t.character_count().await.unwrap_or(0).min(400);
            t.get_text(0, n).await.ok()
        }
        Err(_) => None,
    };
    Ok((states, text))
}

/// Roles text can be typed into.
const EDITABLE: &[&str] = &["text", "entry", "password text", "search box", "editbar", "document text", "terminal", "paragraph"];

/// The field to type into: one named like `field`, else the focused editable element, else
/// the largest editable one (a document's body). Returns its id.
pub fn editable<'a>(tree: &'a Tree, field: Option<&str>) -> Option<&'a Element> {
    let editable: Vec<&Element> = tree
        .elements
        .iter()
        .filter(|e| EDITABLE.contains(&e.role.as_str()) || e.states.contains(&"editable"))
        .filter(|e| !e.states.contains(&"disabled"))
        .collect();
    // A browser's address bar is a text field too, but a page's search box is what "search"
    // means unless the address bar is asked for by name.
    let address_bar = |e: &&Element| {
        let n = e.name.to_lowercase();
        n.contains("enter address") || n.contains("search or enter") || n.contains("address and search bar") || n.contains("search with")
    };
    let wants_address = field.is_some_and(|f| {
        let f = f.to_lowercase();
        f.contains("address") || f.contains("url") || f.contains("location")
    });
    let editable: Vec<&Element> = editable.into_iter().filter(|e| wants_address || !address_bar(e)).collect();
    if let Some(f) = field.map(str::to_lowercase).filter(|f| !f.is_empty()) {
        if let Some(e) = editable.iter().find(|e| e.name.to_lowercase().contains(&f)) {
            return Some(e);
        }
    }
    editable
        .iter()
        .find(|e| e.states.contains(&"focused"))
        .or_else(|| editable.iter().max_by_key(|e| e.bounds.map(|b| b.w as i64 * b.h as i64).unwrap_or(0)))
        .copied()
}

/// The window-relative box of an element of the latest tree, if it has one.
pub fn bounds_of(tree: &Tree, id: &str) -> Option<Rect> {
    tree.elements.iter().find(|e| e.id == id).and_then(|e| e.bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(id: &str, role: &str, name: &str, states: Vec<&'static str>, w: i32) -> Element {
        Element { id: id.into(), role: role.into(), name: name.into(), states, bounds: Some(Rect { x: 0, y: 0, w, h: 20 }), actions: vec![], text: None }
    }

    #[test]
    fn typing_avoids_the_address_bar() {
        let tree = Tree {
            app: "Firefox".into(),
            window: "Bench search".into(),
            elements: vec![
                el("e1", "entry", "Search with Google or enter address", vec!["focused"], 900),
                el("e2", "entry", "Search box", vec![], 200),
                el("e3", "push button", "Search", vec![], 60),
            ],
            truncated: false,
        };
        assert_eq!(editable(&tree, Some("search")).unwrap().id, "e2");
        assert_eq!(editable(&tree, None).unwrap().id, "e2");
        assert_eq!(editable(&tree, Some("address bar")).unwrap().id, "e1");
    }
}
