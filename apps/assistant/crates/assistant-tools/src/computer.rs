//! Computer-use tools: read an app window (accessibility tree, numbered marks, a look with
//! the vision model) and act on it (accessibility actions first, then pointer and keyboard),
//! checking each action afterwards.
//!
//! They run in the daemon, not the plugin worker: element and mark numbers from one call are
//! used by the next, and `screen.look` needs the daemon's model. A target whose label says it
//! sends, buys or deletes ("Send", "Delete", …) is refused until the call is repeated with
//! `confirmed: true`, which the policy always asks about with a key press.

use anyhow::{bail, ensure, Context, Result};
use assistant_core::config::Config;
use assistant_desktop::{a11y, guard, input, marks, ocr, screen, verify, Rect};
use assistant_omarchy::hypr;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub const TOOLS: &[(&str, &str, &str)] = &[
    (
        "ui.tree",
        "Read an app window's accessibility tree: numbered elements (e1…) with role, name, state. Defaults to the focused window.",
        r#"{"type":"object","properties":{"window":{"type":"string","maxLength":256}},"additionalProperties":false}"#,
    ),
    (
        "ui.find",
        "Find elements in a window's accessibility tree by name text and/or role.",
        r#"{"type":"object","properties":{"text":{"type":"string","maxLength":200},"role":{"type":"string","maxLength":40},"window":{"type":"string","maxLength":256}},"additionalProperties":false}"#,
    ),
    (
        "ui.act",
        "Operate an element of an app window, by id from ui.tree or by its name: press it, focus it, type into it (keyboard), or set its text. Repeat with confirmed=true only after the person approves a send/buy/delete.",
        r#"{"type":"object","properties":{"element":{"type":"string","pattern":"^e[0-9]{1,4}$"},"name":{"type":"string","maxLength":200},"window":{"type":"string","maxLength":256},"action":{"enum":["press","focus","type","set_text"]},"text":{"type":"string","maxLength":4000},"confirmed":{"type":"boolean"}},"required":["action"],"additionalProperties":false}"#,
    ),
    (
        "screen.marks",
        "List what can be pointed at in a window as numbered marks (m1…) with labels, from the accessibility tree and on-screen text.",
        r#"{"type":"object","properties":{"window":{"type":"string","maxLength":256},"ocr":{"type":"boolean"}},"additionalProperties":false}"#,
    ),
    (
        "screen.look",
        "Ask the vision model about a window's screenshot with numbered marks drawn on it; it can answer with a mark to act on.",
        r#"{"type":"object","properties":{"question":{"type":"string","minLength":1,"maxLength":500},"window":{"type":"string","maxLength":256}},"required":["question"],"additionalProperties":false}"#,
    ),
    (
        "input.click",
        "Click a mark from screen.marks/screen.look (or x,y screen coordinates). Repeat with confirmed=true only after the person approves a send/buy/delete.",
        r#"{"type":"object","properties":{"mark":{"type":"string","pattern":"^m[0-9]{1,4}$"},"x":{"type":"integer","minimum":0,"maximum":20000},"y":{"type":"integer","minimum":0,"maximum":20000},"button":{"enum":["left","right"]},"double":{"type":"boolean"},"confirmed":{"type":"boolean"}},"additionalProperties":false}"#,
    ),
    (
        "input.type",
        "Type text into an app window's text field (the named field, the focused one, or its main text area).",
        r#"{"type":"object","properties":{"text":{"type":"string","minLength":1,"maxLength":2000},"window":{"type":"string","maxLength":256},"field":{"type":"string","maxLength":200}},"required":["text"],"additionalProperties":false}"#,
    ),
    (
        "input.key",
        "Press a key or combination (enter, tab, escape, end, ctrl+s, ctrl+end…) in an app window.",
        r#"{"type":"object","properties":{"combo":{"type":"string","minLength":1,"maxLength":40},"window":{"type":"string","maxLength":256}},"required":["combo"],"additionalProperties":false}"#,
    ),
    (
        "input.scroll",
        "Scroll at a mark (or x,y); positive steps scroll down.",
        r#"{"type":"object","properties":{"mark":{"type":"string","pattern":"^m[0-9]{1,4}$"},"x":{"type":"integer","minimum":0,"maximum":20000},"y":{"type":"integer","minimum":0,"maximum":20000},"steps":{"type":"integer","minimum":-30,"maximum":30}},"required":["steps"],"additionalProperties":false}"#,
    ),
];

/// Answers a question about a PNG (the daemon's vision model).
pub type Looker = Arc<dyn Fn(Vec<u8>, String) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> + Send + Sync>;

/// What the last read saw, so the next call can use its numbers.
#[derive(Default)]
struct Seen {
    window: Option<hypr::Client>,
    tree: Option<a11y::Tree>,
    marks: Vec<marks::Mark>,
}

static SEEN: Mutex<Option<Seen>> = Mutex::new(None);

fn with_seen<T>(f: impl FnOnce(&mut Seen) -> T) -> T {
    let mut guard = SEEN.lock().unwrap();
    f(guard.get_or_insert_with(Seen::default))
}

fn area(w: &hypr::Client) -> Rect {
    Rect { x: w.at[0] as i32, y: w.at[1] as i32, w: w.size[0] as i32, h: w.size[1] as i32 }
}

/// The window a call is about: the one named, else the one last read. Looking may fall back
/// to the focused window; acting never does (it could be a different app than meant).
async fn target(name: Option<&str>, cfg: &Config, acting: bool) -> Result<hypr::Client> {
    let clients = hypr::clients().await?;
    let w = match name.filter(|n| !n.trim().is_empty()) {
        Some(n) => {
            let found = hypr::match_windows(&clients, n);
            // A denied kind of window is refused as such, however many match.
            if let Some(denied) = found.iter().find_map(|w| guard::check(w, &cfg.computer.deny_classes).err()) {
                return Err(denied);
            }
            ensure!(found.len() <= 1, "“{n}” matches {} windows; name one more exactly", found.len());
            found.into_iter().next().cloned().with_context(|| format!("no window matches “{n}”"))?
        }
        None => {
            let last = with_seen(|s| s.window.as_ref().map(|w| w.address.clone()));
            match last.and_then(|a| clients.iter().find(|c| c.address == a).cloned()) {
                Some(w) => w,
                None if acting => bail!("say which window to use (give window); the last one read isn't open"),
                None => hypr::active_window().await?.context("no focused window")?,
            }
        }
    };
    guard::check(&w, &cfg.computer.deny_classes)?;
    Ok(w)
}

/// Whether a label names something that sends, buys, deletes or submits.
pub fn consequential(label: &str, words: &[String]) -> Option<String> {
    let lower = label.to_lowercase();
    let tokens: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).collect();
    words.iter().find(|w| tokens.contains(&w.to_lowercase().as_str())).cloned()
}

fn needs_confirmation(label: &str, word: &str) -> Value {
    json!({"needs_confirmation": true, "label": label, "reason": format!("“{label}” looks like it would {word}; ask the person, then repeat with confirmed=true")})
}

async fn read_marks(w: &hypr::Client, want_ocr: Option<bool>) -> Result<(Vec<marks::Mark>, Option<screen::Shot>)> {
    let tree = a11y::tree(w).await.ok();
    let elements = tree.as_ref().map(|t| t.elements.clone()).unwrap_or_default();
    let operable = elements.iter().filter(|e| e.bounds.is_some_and(|b| !b.is_empty())).count();
    // OCR is slow on text-heavy windows; it runs when the tree has little, or on request.
    let use_ocr = want_ocr.unwrap_or(operable < 5);
    let shot = screen::capture(area(w)).await.ok();
    let lines = match (&shot, use_ocr) {
        (Some(s), true) => ocr::read(&s.png()?).await.unwrap_or_default(),
        _ => Vec::new(),
    };
    let found = marks::build(&elements, (w.at[0], w.at[1]), shot.as_ref(), &lines);
    with_seen(|s| {
        s.window = Some(w.clone());
        s.tree = tree;
        s.marks = found.clone();
    });
    Ok((found, shot))
}

fn mark(id: &str) -> Result<marks::Mark> {
    with_seen(|s| s.marks.iter().find(|m| m.id == id).cloned()).with_context(|| format!("no mark {id}; list them again (screen.marks)"))
}

async fn focus(w: &hypr::Client) -> Result<()> {
    if hypr::active_window().await?.is_none_or(|a| a.address != w.address) {
        hypr::focus_window(&w.address).await?;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    Ok(())
}

/// The text under a screen point (a small OCR crop), to check a raw-coordinate click.
async fn label_at(x: i32, y: i32) -> String {
    let around = Rect { x: x - 90, y: y - 20, w: 180, h: 40 };
    match screen::capture(around).await {
        Ok(shot) => match shot.png() {
            Ok(png) => ocr::read(&png).await.unwrap_or_default().into_iter().map(|l| l.text).collect::<Vec<_>>().join(" "),
            Err(_) => String::new(),
        },
        Err(_) => String::new(),
    }
}

/// The mark a vision answer points at: a tag ("m12" or a bare "12"), or a box in the
/// model's 0–1000 coordinates (GLM's grounding format), taken as the smallest mark that
/// contains the box's centre. `marks` are image-pixel boxes of an image `size` wide and high.
pub fn answer_mark(answer: &str, marks: &[(String, Rect)], size: (u32, u32)) -> Option<String> {
    let inner = answer.replace("<|begin_of_box|>", " ").replace("<|end_of_box|>", " ");
    let numbers: Vec<f64> = inner
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter_map(|t| t.trim_matches('.').parse::<f64>().ok())
        .collect();
    if numbers.len() >= 4 && mark_in(&inner).is_none() {
        let (cx, cy) = ((numbers[0] + numbers[2]) / 2.0 / 1000.0 * size.0 as f64, (numbers[1] + numbers[3]) / 2.0 / 1000.0 * size.1 as f64);
        return marks
            .iter()
            .filter(|(_, r)| (r.x as f64) <= cx && cx <= (r.x + r.w) as f64 && (r.y as f64) <= cy && cy <= (r.y + r.h) as f64)
            .min_by_key(|(_, r)| r.w * r.h)
            .map(|(id, _)| id.clone());
    }
    mark_in(&inner).or_else(|| match numbers.as_slice() {
        [n] if n.fract() == 0.0 && *n >= 1.0 => Some(format!("m{}", *n as u32)),
        _ => None,
    })
}

/// The element a name refers to: something that can be operated beats a label with the same
/// words, and an exact name beats one that only contains them.
pub fn choose(tree: &a11y::Tree, name: &str) -> Option<a11y::Element> {
    let found = a11y::find(tree, Some(name), None);
    let operable = a11y::operable;
    let exact = |e: &a11y::Element| e.name.eq_ignore_ascii_case(name);
    found
        .iter()
        .copied()
        .find(|e| operable(e) && exact(e))
        .or_else(|| found.iter().copied().find(|e| operable(e)))
        .or_else(|| found.iter().copied().find(|e| exact(e)))
        .or_else(|| found.first().copied())
        .cloned()
}

/// The first mark id ("m12") in a vision answer.
pub fn mark_in(answer: &str) -> Option<String> {
    let lower = answer.to_lowercase();
    let bytes = lower.as_bytes();
    (0..bytes.len()).find_map(|i| {
        if bytes[i] != b'm' || (i > 0 && bytes[i - 1].is_ascii_alphanumeric()) {
            return None;
        }
        let digits: String = lower[i + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
        (!digits.is_empty() && digits.len() <= 4).then(|| format!("m{digits}"))
    })
}

pub async fn call(name: &str, args: &Value, cfg: &Config, looker: Option<&Looker>) -> Option<Result<Value>> {
    if !TOOLS.iter().any(|(t, ..)| *t == name) {
        return None;
    }
    static ENABLED: std::sync::Once = std::sync::Once::new();
    if !ENABLED.is_completed() {
        if let Err(err) = a11y::enable().await {
            tracing::warn!("{err:#}");
        }
        ENABLED.call_once(|| {});
    }
    let s = |k: &str| args[k].as_str();
    let confirmed = args["confirmed"].as_bool().unwrap_or(false);
    let words = &cfg.computer.confirm_words;
    Some(match name {
        "ui.tree" => async {
            let w = target(s("window"), cfg, false).await?;
            let tree = a11y::tree(&w).await?;
            with_seen(|seen| {
                seen.window = Some(w.clone());
                seen.tree = Some(tree.clone());
            });
            let elements: Vec<Value> = tree
                .elements
                .iter()
                .map(|e| json!({"id": e.id, "role": e.role, "name": e.name, "states": e.states, "text": e.text}))
                .collect();
            Ok(json!({"app": tree.app, "window": tree.window, "elements": elements, "truncated": tree.truncated}))
        }
        .await,
        "ui.find" => async {
            let w = target(s("window"), cfg, false).await?;
            let tree = a11y::tree(&w).await?;
            let found: Vec<Value> = a11y::find(&tree, s("text"), s("role"))
                .into_iter()
                .map(|e| json!({"id": e.id, "role": e.role, "name": e.name, "states": e.states}))
                .collect();
            with_seen(|seen| {
                seen.window = Some(w.clone());
                seen.tree = Some(tree.clone());
            });
            Ok(json!({"elements": found}))
        }
        .await,
        "ui.act" => async {
            // By name (reading the window's tree now), or by id from the last read.
            let (w, element) = match (s("name").filter(|n| !n.trim().is_empty()), s("element")) {
                (Some(name), _) => {
                    let w = target(s("window"), cfg, true).await?;
                    let tree = a11y::tree(&w).await?;
                    let pick = choose(&tree, name).with_context(|| format!("no element named like “{name}” in “{}”", w.title))?;
                    with_seen(|seen| {
                        seen.window = Some(w.clone());
                        seen.tree = Some(tree.clone());
                    });
                    (w, pick)
                }
                (None, Some(id)) => {
                    let (w, element) = with_seen(|seen| (seen.window.clone(), seen.tree.as_ref().and_then(|t| t.elements.iter().find(|e| e.id == id).cloned())));
                    (w.context("read the window first (ui.tree), or give name and window")?, element.with_context(|| format!("no element {id}; read the tree again"))?)
                }
                (None, None) => bail!("give the element's name (with window) or its id from ui.tree"),
            };
            let id = element.id.as_str();
            guard::check(&w, &cfg.computer.deny_classes)?;
            if s("action") == Some("type") {
                let text = s("text").context("type needs text")?;
                focus(&w).await?;
                let before = verify::before(&w, Some(id)).await;
                a11y::act(id, a11y::Act::Focus, None).await?;
                input::type_text(text).await?;
                let change = verify::after_typing(before, text, 300).await;
                return Ok(json!({"element": id, "name": element.name, "method": "keyboard", "verified": change.verified, "evidence": change.evidence}));
            }
            let action = match s("action") {
                Some("press") => a11y::Act::Press,
                Some("focus") => a11y::Act::Focus,
                Some("set_text") => a11y::Act::SetText,
                _ => bail!("action must be press, focus, type or set_text"),
            };
            if action == a11y::Act::Press && !confirmed {
                if let Some(word) = consequential(&element.name, words) {
                    return Ok(needs_confirmation(&element.name, &word));
                }
            }
            let before = verify::before(&w, Some(id)).await;
            let did = a11y::act(id, action, s("text")).await;
            let mut change = match action {
                a11y::Act::SetText => verify::after_typing(before, s("text").unwrap_or(""), 300).await,
                _ => verify::after(before, 400).await,
            };
            let mut method = "accessibility";
            // Recovery: an element that ignored its action gets a real click on its box.
            if action == a11y::Act::Press && (did.is_err() || !change.verified) {
                if let Some(b) = element.bounds.filter(|b| !b.is_empty()) {
                    let (x, y) = b.offset(w.at[0], w.at[1]).center();
                    focus(&w).await?;
                    let before = verify::before(&w, Some(id)).await;
                    input::click(x, y, input::Button::Left, false).await?;
                    change = verify::after(before, 400).await;
                    method = "click";
                } else {
                    did.as_ref().map_err(|e| anyhow::anyhow!("{e:#}"))?;
                }
            } else {
                did.as_ref().map_err(|e| anyhow::anyhow!("{e:#}"))?;
            }
            Ok(json!({"element": id, "name": element.name, "method": method, "verified": change.verified, "evidence": change.evidence}))
        }
        .await,
        "screen.marks" => async {
            let w = target(s("window"), cfg, false).await?;
            let (found, _) = read_marks(&w, args["ocr"].as_bool()).await?;
            let list: Vec<Value> = found.iter().map(|m| json!({"id": m.id, "label": m.label, "role": m.role, "source": m.source})).collect();
            Ok(json!({"window": w.title, "marks": list}))
        }
        .await,
        "screen.look" => async {
            let looker = looker.context("vision isn't available")?;
            let w = target(s("window"), cfg, false).await?;
            let (found, shot) = read_marks(&w, None).await?;
            let shot = shot.context("couldn't capture the window")?;
            let drawn = image::DynamicImage::ImageRgba8(marks::draw(&shot, &found));
            let full_w = shot.image.width() as f64;
            let small = screen::Shot { image: drawn, area: shot.area }.downscaled(cfg.vision.max_width);
            let k = small.width() as f64 / full_w;
            let boxes: Vec<(String, Rect)> = found
                .iter()
                .map(|m| {
                    let r = shot.to_image(m.bounds);
                    (m.id.clone(), Rect { x: (r.x as f64 * k) as i32, y: (r.y as f64 * k) as i32, w: (r.w as f64 * k) as i32, h: (r.h as f64 * k) as i32 })
                })
                .collect();
            let mut png = std::io::Cursor::new(Vec::new());
            small.write_to(&mut png, image::ImageFormat::Png)?;
            let question = format!(
                "{}\nNumbered tags mark things that can be clicked. If the answer is a place to act, end with its tag number; say none if no tag fits.",
                s("question").unwrap_or("")
            );
            let t = std::time::Instant::now();
            let answer = looker(png.into_inner(), question).await?;
            let picked = answer_mark(&answer, &boxes, (small.width(), small.height())).filter(|m| found.iter().any(|f| &f.id == m));
            let label = picked.as_ref().and_then(|m| found.iter().find(|f| &f.id == m)).map(|f| f.label.clone());
            Ok(json!({"answer": answer, "mark": picked, "label": label, "look_ms": t.elapsed().as_millis() as u64}))
        }
        .await,
        "input.click" | "input.scroll" => async {
            let w = with_seen(|seen| seen.window.clone());
            let w = match w {
                Some(w) => w,
                None => target(None, cfg, true).await?,
            };
            guard::check(&w, &cfg.computer.deny_classes)?;
            let (x, y, label) = match s("mark") {
                Some(id) => {
                    let m = mark(id)?;
                    let (x, y) = m.bounds.center();
                    (x, y, m.label)
                }
                None => {
                    let (x, y) = (args["x"].as_i64().context("give a mark or x and y")? as i32, args["y"].as_i64().context("give a mark or x and y")? as i32);
                    let b = area(&w);
                    ensure!(x >= b.x && y >= b.y && x < b.x + b.w && y < b.y + b.h, "that point is outside “{}”", w.title);
                    (x, y, if name == "input.click" { label_at(x, y).await } else { String::new() })
                }
            };
            if name == "input.click" && !confirmed {
                if let Some(word) = consequential(&label, words) {
                    return Ok(needs_confirmation(&label, &word));
                }
            }
            focus(&w).await?;
            let before = verify::before(&w, None).await;
            if name == "input.click" {
                let button = if s("button") == Some("right") { input::Button::Right } else { input::Button::Left };
                input::click(x, y, button, args["double"].as_bool().unwrap_or(false)).await?;
            } else {
                input::scroll(x, y, args["steps"].as_i64().unwrap_or(3) as i32).await?;
            }
            let change = verify::after(before, 400).await;
            Ok(json!({"at": [x, y], "label": label, "verified": change.verified, "evidence": change.evidence}))
        }
        .await,
        "input.type" | "input.key" => async {
            let w = target(s("window"), cfg, true).await?;
            focus(&w).await?;
            // Typing lands in the field meant for it: the window's keyboard focus may sit on a
            // tab or button.
            let mut field = None;
            let mut field_id = None;
            let mut buttons: Vec<String> = Vec::new();
            if name == "input.type" {
                if let Ok(tree) = a11y::tree(&w).await {
                    match a11y::editable(&tree, s("field")) {
                        Some(e) => {
                            field = Some(e.name.clone());
                            let id = e.id.clone();
                            with_seen(|seen| seen.tree = Some(tree.clone()));
                            // Focusing a field that has focus already can move its cursor
                            // (VS Code puts it back at the start), undoing ctrl+end.
                            if !e.states.contains(&"focused") {
                                let _ = a11y::act(&id, a11y::Act::Focus, None).await;
                            }
                            field_id = Some(id);
                        }
                        // No field in the tree: the app may draw its editor itself (Write's
                        // document). Typing goes to the keyboard focus; OCR then confirms or
                        // not, and a failure names the window's buttons.
                        None if !tree.elements.is_empty() => {
                            buttons = tree
                                .elements
                                .iter()
                                .filter(|e| e.role.contains("button") && !e.name.is_empty())
                                .map(|e| e.name.clone())
                                .take(12)
                                .collect();
                        }
                        None => {}
                    }
                }
            }
            with_seen(|seen| seen.window = Some(w.clone()));
            let before = verify::before(&w, field_id.as_deref()).await;
            let change = if name == "input.type" {
                let text = s("text").unwrap_or("");
                input::type_text(text).await?;
                let mut change = verify::after_typing(before, text, 300).await;
                if !change.verified && field_id.is_none() && !buttons.is_empty() {
                    change.evidence.push(format!("the window shows no text field; its buttons: {}", buttons.join("; ")));
                }
                change
            } else {
                input::key(s("combo").unwrap_or(""), Some(&w.address)).await?;
                verify::after(before, 400).await
            };
            Ok(json!({"window": w.title, "field": field, "verified": change.verified, "evidence": change.evidence}))
        }
        .await,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_parse() {
        for (name, _, schema) in TOOLS {
            serde_json::from_str::<Value>(schema).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn spots_consequential_targets() {
        let words: Vec<String> = assistant_core::config::ComputerCfg::default().confirm_words;
        assert_eq!(consequential("Send", &words).as_deref(), Some("send"));
        assert_eq!(consequential("Delete file", &words).as_deref(), Some("delete"));
        assert_eq!(consequential("Place order", &words).as_deref(), Some("order"));
        assert_eq!(consequential("Buy now", &words).as_deref(), Some("buy"));
        assert!(consequential("Sender settings", &words).is_none(), "whole words only");
        assert!(consequential("Save", &words).is_none());
        assert!(consequential("New tab (Ctrl+T)", &words).is_none());
    }

    #[test]
    fn names_pick_the_button_not_its_label() {
        let el = |id: &str, role: &str, name: &str| a11y::Element { id: id.into(), role: role.into(), name: name.into(), states: vec![], bounds: None, actions: vec![], text: None };
        let tree = a11y::Tree {
            app: "nixe-office".into(),
            window: "Write".into(),
            elements: vec![el("e7", "button", "Blank document Start from an empty page."), {
                // GTK labels expose clipboard actions; that doesn't make them buttons.
                let mut label = el("e20", "label", "Blank document");
                label.actions = vec!["clipboard.copy".into(), "link.open".into()];
                label
            }],
            truncated: false,
        };
        assert_eq!(choose(&tree, "Blank document").unwrap().id, "e7");
    }

    #[test]
    fn reads_tags_and_boxes() {
        let marks = vec![("m1".to_string(), Rect { x: 0, y: 0, w: 100, h: 50 }), ("m2".to_string(), Rect { x: 600, y: 0, w: 40, h: 40 })];
        assert_eq!(answer_mark("<|begin_of_box|>2<|end_of_box|>", &marks, (1000, 500)).as_deref(), Some("m2"));
        assert_eq!(answer_mark("m1", &marks, (1000, 500)).as_deref(), Some("m1"));
        // A box around (620, 20) in pixels of a 1000×500 image is [610,20,630,60] in 0–1000.
        assert_eq!(answer_mark("<|begin_of_box|>[610,20,630,60]<|end_of_box|>", &marks, (1000, 500)).as_deref(), Some("m2"));
        assert_eq!(answer_mark("[900,900,950,950]", &marks, (1000, 500)), None, "a box over no mark");
        assert_eq!(answer_mark("none", &marks, (1000, 500)), None);
    }

    #[test]
    fn reads_marks_from_answers() {
        assert_eq!(mark_in("The Save button is m12.").as_deref(), Some("m12"));
        assert_eq!(mark_in("M3").as_deref(), Some("m3"));
        assert_eq!(mark_in("none"), None);
        assert_eq!(mark_in("item m"), None);
        assert_eq!(mark_in("rhythm 5"), None, "an m inside a word isn't a mark");
    }
}
