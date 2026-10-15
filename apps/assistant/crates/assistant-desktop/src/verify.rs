//! Did the action do anything? Only evidence counts: the window's title, its set of
//! accessibility elements and their states, the set of windows, or (for typing) the text
//! now in the field or on screen. A changed pixel is never proof: a blinking caret or a
//! notification would pass for success.

use crate::{a11y, ocr, screen, Rect};
use assistant_omarchy::hypr;
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct Before {
    window: hypr::Client,
    windows: Vec<String>,
    tree: Option<u64>,
    field: Option<(String, Option<String>)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Change {
    /// The action provably did something.
    pub verified: bool,
    /// What showed it, or why it couldn't be confirmed.
    pub evidence: Vec<String>,
}

/// A digest of what the window's tree shows: roles, names, states and field text.
fn digest(tree: &a11y::Tree) -> u64 {
    let mut h = DefaultHasher::new();
    for e in &tree.elements {
        (&e.role, &e.name, &e.states, &e.text).hash(&mut h);
    }
    h.finish()
}

async fn window_now(address: &str) -> Option<hypr::Client> {
    hypr::clients().await.ok()?.into_iter().find(|c| c.address == address)
}

async fn addresses() -> Vec<String> {
    hypr::clients().await.map(|cs| cs.into_iter().map(|c| c.address).collect()).unwrap_or_default()
}

/// Record the window before acting. `field` is an element id from the latest tree whose
/// text should be compared (the field being typed into).
pub async fn before(window: &hypr::Client, field: Option<&str>) -> Before {
    let tree = a11y::tree(window).await.ok().map(|t| digest(&t));
    let field = match field {
        Some(id) => a11y::snapshot(id).await.ok().map(|(_, text)| (id.to_string(), text)),
        None => None,
    };
    Before { window: window.clone(), windows: addresses().await, tree, field }
}

fn normalize(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// After a press, click or key: did the window change in a way that can be named?
pub async fn after(before: Before, settle_ms: u64) -> Change {
    tokio::time::sleep(std::time::Duration::from_millis(settle_ms)).await;
    let mut evidence = Vec::new();
    let now = window_now(&before.window.address).await;
    match &now {
        None => evidence.push("the window closed".into()),
        Some(w) if w.title != before.window.title => evidence.push(format!("the title became “{}”", w.title)),
        _ => {}
    }
    let opened = addresses().await.into_iter().filter(|a| !before.windows.contains(a)).count();
    if opened > 0 {
        evidence.push(format!("{opened} new window(s) opened"));
    }
    if let (Some(w), Some(old)) = (&now, before.tree) {
        if let Ok(tree) = a11y::tree(w).await {
            if digest(&tree) != old {
                evidence.push("the window's contents changed".into());
            }
        }
    }
    if let Some((id, old)) = &before.field {
        if let Ok((_, text)) = a11y::snapshot(id).await {
            if &text != old {
                evidence.push("the field's text changed".into());
            }
        }
    }
    let verified = !evidence.is_empty();
    if !verified {
        evidence.push(if before.tree.is_some() {
            "nothing in the window changed".into()
        } else {
            "couldn't confirm: the app doesn't expose its contents".into()
        });
    }
    Change { verified, evidence }
}

/// After typing `typed`: is it now in the field, or readable on screen?
pub async fn after_typing(before: Before, typed: &str, settle_ms: u64) -> Change {
    tokio::time::sleep(std::time::Duration::from_millis(settle_ms)).await;
    // The start of the text is enough (a long text may scroll out of view).
    let want: String = normalize(typed).chars().take(24).collect();
    if let Some((id, old)) = &before.field {
        if let Ok((_, Some(text))) = a11y::snapshot(id).await {
            if normalize(&text).contains(want.trim()) && Some(&text) != old.as_ref() {
                return Change { verified: true, evidence: vec!["the text is in the field".into()] };
            }
        }
    }
    if let Some(w) = window_now(&before.window.address).await {
        let area = Rect { x: w.at[0] as i32, y: w.at[1] as i32, w: w.size[0] as i32, h: w.size[1] as i32 };
        if let Ok(shot) = screen::capture(area).await {
            if let Ok(png) = shot.png() {
                let seen = ocr::read(&png).await.unwrap_or_default().into_iter().map(|l| l.text).collect::<Vec<_>>().join(" ");
                if normalize(&seen).contains(want.trim()) {
                    return Change { verified: true, evidence: vec!["the text is visible in the window".into()] };
                }
            }
        }
    }
    Change { verified: false, evidence: vec!["the typed text isn't in the field or visible in the window".into()] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_sees_state_changes() {
        let e = |states: Vec<&'static str>| a11y::Element {
            id: "e1".into(),
            role: "toggle button".into(),
            name: "Wi-Fi".into(),
            states,
            bounds: None,
            actions: vec![],
            text: None,
        };
        let t = |states| a11y::Tree { app: "a".into(), window: "w".into(), elements: vec![e(states)], truncated: false };
        assert_ne!(digest(&t(vec![])), digest(&t(vec!["checked"])));
        assert_eq!(digest(&t(vec!["checked"])), digest(&t(vec!["checked"])));
        assert_eq!(normalize("  Hello\n World "), "hello world");
    }
}
