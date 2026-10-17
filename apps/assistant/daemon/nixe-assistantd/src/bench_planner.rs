//! `bench planner`: the tool planner's next call on fixed computer-use steps, scored
//! automatically, for each model. The prompt and grammar are the daemon's own
//! (`planner.rs`); only the model changes. Raw model output is scored: the daemon's own
//! corrections (refusing an early finish once, filling in the window, skipping repeats)
//! aren't applied, so a model's mistakes show.

use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

/// How an argument must look.
enum M {
    /// Equal, ignoring case and spaces (key combos: "Ctrl + S" is ctrl+s).
    Is(&'static str),
    /// Contains the text, ignoring case.
    Has(&'static str),
}

/// One acceptable next call: a tool and what its arguments must hold. `None` is finish.
type Alt = Option<(&'static str, &'static [(&'static str, M)])>;

struct Case {
    /// "computer" (only computer tools offered) or "typed" (the app's own tools offered too).
    group: &'static str,
    ask: &'static str,
    /// Steps already done: tool, arguments, result (JSON).
    done: &'static [(&'static str, &'static str, &'static str)],
    expect: &'static [Alt],
}

const OK: &str = r#"{"isError":false,"structuredContent":{"verified":true}}"#;

const CASES: &[Case] = &[
    // Notepad
    Case { group: "computer", ask: "In the Notepad window, open a new tab.", done: &[],
        expect: &[Some(("ui.act", &[("name", M::Has("new tab"))])), Some(("input.key", &[("combo", M::Is("ctrl+t"))]))] },
    Case { group: "computer", ask: "In the Notepad window, open a new tab.",
        done: &[("ui.act", r#"{"window":"Notepad","name":"New tab","action":"press"}"#, OK)], expect: &[None] },
    Case { group: "computer", ask: "In Notepad, type the words The bench typed this. and then save the file with ctrl+s.", done: &[],
        expect: &[Some(("input.type", &[("text", M::Has("bench typed this"))])), Some(("ui.act", &[("text", M::Has("bench typed this"))]))] },
    Case { group: "computer", ask: "In Notepad, type the words The bench typed this. and then save the file with ctrl+s.",
        done: &[("input.type", r#"{"window":"Notepad","text":"The bench typed this."}"#, OK)],
        expect: &[Some(("input.key", &[("combo", M::Is("ctrl+s"))]))] },
    Case { group: "computer", ask: "In Notepad, type the words The bench typed this. and then save the file with ctrl+s.",
        done: &[("input.type", r#"{"window":"Notepad","text":"The bench typed this."}"#, OK), ("input.key", r#"{"window":"Notepad","combo":"ctrl+s"}"#, OK)],
        expect: &[None] },
    // Stopping after focusing the window is the early finish small planners make.
    Case { group: "computer", ask: "In Notepad, type hello and save it.",
        done: &[("windows.focus", r#"{"window":"Notepad"}"#, OK)],
        expect: &[Some(("input.type", &[("text", M::Has("hello"))])), Some(("ui.act", &[("text", M::Has("hello"))]))] },
    // Write
    Case { group: "computer", ask: "In the Write window, type the sentence: Benchmark sentence for Write.", done: &[],
        expect: &[Some(("input.type", &[("text", M::Has("benchmark sentence for write"))])), Some(("ui.act", &[("text", M::Has("benchmark sentence for write"))]))] },
    Case { group: "computer", ask: "In the Write window, type the sentence: Benchmark sentence for Write.",
        done: &[("input.type", r#"{"window":"Write","text":"Benchmark sentence for Write."}"#, OK)], expect: &[None] },
    // Firefox
    Case { group: "computer", ask: "In the Firefox window titled Bench search, type nixe into the search box and press the Search button.", done: &[],
        expect: &[Some(("input.type", &[("text", M::Is("nixe"))])), Some(("ui.act", &[("text", M::Is("nixe"))]))] },
    Case { group: "computer", ask: "In the Firefox window titled Bench search, type nixe into the search box and press the Search button.",
        done: &[("input.type", r#"{"window":"Bench search","field":"Search box","text":"nixe"}"#, OK)],
        expect: &[Some(("ui.act", &[("name", M::Has("search")), ("action", M::Is("press"))])), Some(("input.key", &[("combo", M::Is("enter"))]))] },
    Case { group: "computer", ask: "In the Firefox window titled Bench search, search the page for nixe.", done: &[],
        expect: &[Some(("input.type", &[("text", M::Is("nixe"))])), Some(("ui.act", &[("text", M::Is("nixe"))]))] },
    // VS Code
    Case { group: "computer", ask: "In VS Code, go to the end of the file, type a new line saying Line two. and save it with ctrl+s.", done: &[],
        expect: &[Some(("input.key", &[("combo", M::Is("ctrl+end"))]))] },
    Case { group: "computer", ask: "In VS Code, go to the end of the file, type a new line saying Line two. and save it with ctrl+s.",
        done: &[("input.key", r#"{"window":"VS Code","combo":"ctrl+end"}"#, OK)],
        expect: &[Some(("input.key", &[("combo", M::Is("enter"))])), Some(("input.type", &[("text", M::Has("line two"))]))] },
    Case { group: "computer", ask: "In VS Code, go to the end of the file, type a new line saying Line two. and save it with ctrl+s.",
        done: &[("input.key", r#"{"window":"VS Code","combo":"ctrl+end"}"#, OK), ("input.key", r#"{"window":"VS Code","combo":"enter"}"#, OK)],
        expect: &[Some(("input.type", &[("text", M::Has("line two"))]))] },
    Case { group: "computer", ask: "In VS Code, go to the end of the file, type a new line saying Line two. and save it with ctrl+s.",
        done: &[("input.key", r#"{"window":"VS Code","combo":"ctrl+end"}"#, OK), ("input.key", r#"{"window":"VS Code","combo":"enter"}"#, OK), ("input.type", r#"{"window":"VS Code","text":"Line two."}"#, OK)],
        expect: &[Some(("input.key", &[("combo", M::Is("ctrl+s"))]))] },
    Case { group: "computer", ask: "In VS Code, go to the end of the file, type a new line saying Line two. and save it with ctrl+s.",
        done: &[("input.key", r#"{"window":"VS Code","combo":"ctrl+end"}"#, OK), ("input.key", r#"{"window":"VS Code","combo":"enter"}"#, OK), ("input.type", r#"{"window":"VS Code","text":"Line two."}"#, OK), ("input.key", r#"{"window":"VS Code","combo":"ctrl+s"}"#, OK)],
        expect: &[None] },
    // Spotify (computer tools only: in the daemon, app requests see only these)
    Case { group: "computer", ask: "In the Spotify window, press the play/pause button.", done: &[],
        expect: &[Some(("ui.act", &[("name", M::Has("pla"))])), Some(("ui.act", &[("name", M::Has("pause"))])), Some(("input.key", &[("combo", M::Is("space"))]))] },
    Case { group: "computer", ask: "In Spotify, skip to the next track.", done: &[],
        expect: &[Some(("ui.act", &[("name", M::Has("next"))])), Some(("input.key", &[("combo", M::Is("ctrl+right"))]))] },
    Case { group: "computer", ask: "In Spotify, search for Daft Punk.", done: &[],
        expect: &[Some(("input.type", &[("text", M::Has("daft punk"))])), Some(("input.key", &[("combo", M::Is("ctrl+k"))])), Some(("input.key", &[("combo", M::Is("ctrl+l"))])), Some(("ui.act", &[("name", M::Has("search"))]))] },
    // Safety and fallbacks
    Case { group: "computer", ask: "In the Mail window, delete the newest message.",
        done: &[("ui.act", r#"{"window":"Mail","name":"Delete","action":"press"}"#, r#"{"isError":false,"structuredContent":{"needs_confirmation":true,"target":"Delete","text":"This deletes; ask the person first."}}"#)],
        expect: &[None] },
    Case { group: "computer", ask: "In the Game window, press the Start button.",
        done: &[("ui.tree", r#"{"window":"Game"}"#, r#"{"isError":true,"content":[{"type":"text","text":"this window has no accessibility tree"}]}"#)],
        expect: &[Some(("screen.marks", &[]))] },
    // The app's own tools next to the computer tools (as the daemon offers them for Nixe apps): the
    // typed tool is the right call.
    Case { group: "typed", ask: "In the Spotify window, press the play/pause button.", done: &[],
        expect: &[Some(("media.control", &[("action", M::Is("toggle"))])), Some(("media.control", &[("action", M::Is("play"))])), Some(("media.control", &[("action", M::Is("pause"))]))] },
    Case { group: "typed", ask: "In Spotify, skip to the next track.", done: &[],
        expect: &[Some(("media.control", &[("action", M::Is("next"))]))] },
    Case { group: "typed", ask: "In the Notepad window, open a new tab.", done: &[],
        expect: &[Some(("notepad.new_tab", &[]))] },
    Case { group: "typed", ask: "In Notepad, type hello and save it.", done: &[],
        expect: &[Some(("notepad.insert_text", &[("text", M::Has("hello"))]))] },
    Case { group: "typed", ask: "In the Write window, type the sentence: Benchmark sentence for Write.", done: &[],
        expect: &[Some(("write.insert_text", &[("text", M::Has("benchmark sentence for write"))]))] },
    Case { group: "typed", ask: "In Notepad, write call mum and save it as ~/todo.txt.", done: &[],
        expect: &[Some(("notepad.new_tab", &[]))] },
    Case { group: "typed", ask: "In Notepad, write call mum and save it as ~/todo.txt.",
        done: &[("notepad.new_tab", "{}", r#"{"isError":false,"structuredContent":{"tab":{"name":"Untitled"}}}"#), ("notepad.insert_text", r#"{"text":"call mum"}"#, r#"{"isError":false,"structuredContent":{"inserted":8}}"#)],
        expect: &[Some(("notepad.save", &[("path", M::Has("todo.txt"))]))] },
    Case { group: "typed", ask: "In Notepad, open ~/notes/ideas.txt.", done: &[],
        expect: &[Some(("notepad.open", &[("path", M::Has("ideas.txt"))]))] },
    Case { group: "typed", ask: "In Write, read me what the document says.", done: &[],
        expect: &[Some(("write.read", &[]))] },
    Case { group: "typed", ask: "In Write, add a heading saying Results.", done: &[],
        expect: &[Some(("write.insert_text", &[("text", M::Has("results")), ("style", M::Is("heading1"))])), Some(("write.insert_text", &[("text", M::Has("results")), ("style", M::Is("heading2"))]))] },
    Case { group: "typed", ask: "In Write, save the document as ~/report.odt.", done: &[],
        expect: &[Some(("write.save", &[("path", M::Has("report.odt"))]))] },
];

/// App tools for the typed group: `media.*` as the media ability defines them, and Notepad's
/// and Write's own tools as the nixe-apps ability defines them.
const TYPED: &[(&str, &str, &str)] = &[
    ("media.status", "Read current song and playback state", r#"{"type":"object","properties":{"player":{"type":"string"}}}"#),
    ("media.control", "Play, pause, toggle, next, previous, or stop a media player", r#"{"type":"object","properties":{"player":{"type":"string"},"action":{"type":"string","enum":["play","pause","toggle","next","previous","stop"]}},"required":["action"]}"#),
    ("notepad.new_tab", "Open a new, empty Notepad tab.", r#"{"type":"object","properties":{}}"#),
    ("notepad.open", "Open a text file (inside the home folder) in a Notepad tab.", r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}"#),
    ("notepad.read", "Read the current Notepad tab: name, file and text.", r#"{"type":"object","properties":{}}"#),
    ("notepad.tabs", "List Notepad's tabs: name, file, unsaved changes.", r#"{"type":"object","properties":{}}"#),
    ("notepad.insert_text", "Insert text into the current Notepad tab, at the cursor or at the end.", r#"{"type":"object","properties":{"text":{"type":"string"},"at":{"enum":["cursor","end"]}},"required":["text"]}"#),
    ("notepad.save", "Save the current Notepad tab; give a path (inside the home folder) for an untitled tab or to save under a new name.", r#"{"type":"object","properties":{"path":{"type":"string"},"overwrite":{"type":"boolean"}}}"#),
    ("write.new_document", "Start a new, empty Write document.", r#"{"type":"object","properties":{}}"#),
    ("write.open", "Open a document (inside the home folder) in Write.", r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}"#),
    ("write.read", "Read the open Write document: title, file and text (as Markdown).", r#"{"type":"object","properties":{}}"#),
    ("write.insert_text", "Insert text into the Write document at the cursor or the end; each line is a paragraph in the given style.", r#"{"type":"object","properties":{"text":{"type":"string"},"style":{"enum":["body","title","heading1","heading2","heading3","quote"]},"at":{"enum":["cursor","end"]}},"required":["text"]}"#),
    ("write.save", "Save the Write document; give a path (inside the home folder; .nwrite, .odt, .docx, .md...) for a new document or to save under a new name.", r#"{"type":"object","properties":{"path":{"type":"string"},"overwrite":{"type":"boolean"}}}"#),
    ("write.export_pdf", "Export the Write document as a PDF (a path inside the home folder ending in .pdf).", r#"{"type":"object","properties":{"path":{"type":"string"},"overwrite":{"type":"boolean"}},"required":["path"]}"#),
];

/// The catalog an app request sees in the daemon: computer and window tools (computer use
/// asks first; window tools are allowed by default). The typed group adds the app tools.
fn catalog(typed: bool) -> Value {
    let entry = |(name, description, schema): &(&str, &str, &str), permission: &str| {
        json!({"name": name, "description": description, "inputSchema": serde_json::from_str::<Value>(schema).unwrap_or(json!({})), "permission": permission})
    };
    let mut tools: Vec<Value> = assistant_tools::system::TOOLS.iter().filter(|t| t.0.starts_with("windows.")).map(|t| entry(t, "allow")).collect();
    tools.extend(assistant_tools::computer::TOOLS.iter().map(|t| entry(t, "ask")));
    if typed {
        tools.extend(TYPED.iter().map(|t| entry(t, "allow")));
    }
    Value::Array(tools)
}

fn norm(s: &str) -> String {
    s.to_lowercase().replace(' ', "").replace("control", "ctrl").replace("return", "enter")
}

/// right; a detour (a harmless look-first step: it costs a step, not the task); or what went
/// wrong: malformed, early finish, no finish, wrong tool, wrong argument.
fn score(out: &str, expect: &[Alt]) -> &'static str {
    let Ok(plan) = serde_json::from_str::<Value>(out.trim()) else {
        return "malformed";
    };
    let finished = plan["finish"] == true;
    let wants_finish = expect.iter().any(Option::is_none);
    if finished {
        return if wants_finish { "right" } else { "early finish" };
    }
    if wants_finish {
        return "no finish";
    }
    let tool = plan["tool"].as_str().unwrap_or("");
    let args = &plan["arguments"];
    let mut tool_seen = false;
    for (name, needs) in expect.iter().flatten() {
        if *name != tool {
            continue;
        }
        tool_seen = true;
        let fits = needs.iter().all(|(key, m)| {
            let v = args[*key].as_str().unwrap_or("");
            match m {
                M::Is(want) => norm(v) == norm(want),
                M::Has(want) => v.to_lowercase().contains(&want.to_lowercase()),
            }
        });
        if fits {
            return "right";
        }
    }
    let looks = ["ui.find", "ui.tree", "screen.marks", "windows.focus", "windows.list", "windows.active"].contains(&tool)
        || (tool == "ui.act" && args["action"] == "focus");
    if tool_seen {
        "wrong argument"
    } else if looks {
        "detour"
    } else {
        "wrong tool"
    }
}

/// The models compared: the suitable curated ones, where installed. (Ternary Bonsai 2 9B waits for
/// a bundled llama.cpp that applies its `prism.hadamard` transform; without it, it loads and
/// talks nonsense.)
pub(crate) fn models() -> Vec<(String, Option<std::path::PathBuf>)> {
    let installed = assistant_core::Config::scan_local_models();
    let find = |file: &str| installed.iter().find(|p| p.file_name().is_some_and(|n| n == file)).cloned();
    assistant_models::recommended(None).into_iter().map(|m| (m.name.to_string(), find(m.filename))).collect()
}

pub(crate) fn gb(path: &Path) -> String {
    std::fs::metadata(path).map(|m| format!("{:.1} GB", m.len() as f64 / 1e9)).unwrap_or("—".into())
}

/// Each model runs in its own process: llama.cpp's backend starts once per process.
pub async fn planner() -> Result<()> {
    let (computer, typed) = (CASES.iter().filter(|c| c.group == "computer").count(), CASES.iter().filter(|c| c.group == "typed").count());
    println!("{computer} computer-use steps, {typed} with the app's own tools offered\n");
    println!("| Model | Computer use | App tools | Detours | Mistakes | Median step | p90 step | VRAM added | File |\n|---|---|---|---|---|---|---|---|---|");
    for (label, path) in models() {
        let Some(path) = path else {
            println!("| {label} | not installed | — | — | — | — | — | — | — |");
            continue;
        };
        let out = tokio::process::Command::new(std::env::current_exe()?).args(["bench", "planner", "--one"]).arg(&path).output().await?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let Some(r) = stdout.lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok()) else {
            println!("| {label} | failed: {} | — | — | — | — | — | — | {} |", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""), gb(&path));
            continue;
        };
        let mistakes = r["mistakes"].as_object().map(|m| m.iter().map(|(k, v)| format!("{v} {k}")).collect::<Vec<_>>().join(", ")).filter(|s| !s.is_empty()).unwrap_or("none".into());
        println!(
            "| {label} | {}/{computer} | {}/{typed} | {} | {mistakes} | {} ms | {} ms | {} | {} |",
            r["computer"], r["typed"], r["detours"], r["median_ms"], r["p90_ms"],
            r["vram_added_mib"].as_u64().map(|v| format!("{v} MiB")).unwrap_or("not measured".into()),
            gb(&path)
        );
        let log = super::bench_dir().join(format!("planner-{}.json", path.file_stem().unwrap_or_default().to_string_lossy()));
        let _ = tokio::fs::create_dir_all(super::bench_dir()).await;
        tokio::fs::write(&log, serde_json::to_vec_pretty(&r["answers"])?).await?;
    }
    println!("\nEach model's answers: {}/planner-*.json", super::bench_dir().display());
    Ok(())
}

/// One model over every case; prints one JSON line.
pub async fn planner_one(model: &Path) -> Result<()> {
    super::child_logging();
    let mut cfg = assistant_core::Config::load()?;
    cfg.llm.model = model.display().to_string();
    // The same context for every model (planner prompts are about 3k tokens); a larger
    // model's KV cache at the configured size may not fit next to it.
    cfg.llm.ctx = 8192;
    let before = super::vram_mib();
    let llm = assistant_inference::LlamaCppBackend::spawn(&cfg)?;
    let (mut peak, mut times, mut answers) = (before, Vec::new(), Vec::new());
    let (mut computer, mut typed, mut detours) = (0, 0, 0);
    let mut mistakes = serde_json::Map::new();
    for case in CASES {
        let system = crate::planner::system(&catalog(case.group == "typed"), "", "", "", true);
        let results: Vec<Value> = case
            .done
            .iter()
            .map(|(tool, args, result)| json!({"tool": tool, "arguments": serde_json::from_str::<Value>(args).unwrap_or_default(), "result": serde_json::from_str::<Value>(result).unwrap_or_default()}))
            .collect();
        let t = Instant::now();
        let mut rx = llm
            .generate_with_grammar(&system, &[], &crate::planner::step(case.ask, &results), cfg.llm.n_predict, 0.0, Arc::new(AtomicBool::new(false)), Some(crate::planner::GRAMMAR.into()))
            .await?;
        let mut out = String::new();
        while let Some(tok) = rx.recv().await {
            out.push_str(&tok?);
        }
        times.push(t.elapsed().as_millis() as u64);
        peak = peak.max(super::vram_mib());
        let verdict = score(&out, case.expect);
        if verdict == "right" {
            if case.group == "typed" {
                typed += 1;
            } else {
                computer += 1;
            }
        } else if verdict == "detour" {
            detours += 1;
        } else {
            let n = mistakes.get(verdict).and_then(Value::as_u64).unwrap_or(0);
            mistakes.insert(verdict.into(), json!(n + 1));
        }
        answers.push(json!({"group": case.group, "ask": case.ask, "steps_done": case.done.len(), "answer": out, "verdict": verdict}));
    }
    llm.unload().await?;
    let mut sorted = times.clone();
    sorted.sort();
    let pick = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    let added = match (before, peak) {
        (Some(b), Some(p)) => Some(p.saturating_sub(b)),
        _ => None,
    };
    println!("{}", json!({"computer": computer, "typed": typed, "detours": detours, "mistakes": mistakes, "median_ms": pick(0.5), "p90_ms": pick(0.9), "vram_added_mib": added, "answers": answers}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_SAVE: &[Alt] = &[Some(("input.key", &[("combo", M::Is("ctrl+s"))]))];
    const TYPE: &[Alt] = &[Some(("input.type", &[("text", M::Has("line two"))]))];

    #[test]
    fn scores_calls_and_finishes() {
        assert_eq!(score(r#"{"tool":"input.key","arguments":{"combo":"Ctrl + S"}}"#, KEY_SAVE), "right");
        assert_eq!(score(r#"{"tool":"input.key","arguments":{"combo":"control+s"}}"#, KEY_SAVE), "right");
        assert_eq!(score(r#"{"tool":"input.key","arguments":{"combo":"ctrl+end"}}"#, KEY_SAVE), "wrong argument");
        assert_eq!(score(r#"{"tool":"ui.act","arguments":{"name":"Save"}}"#, KEY_SAVE), "wrong tool");
        assert_eq!(score(r#"{"tool":"ui.find","arguments":{"text":"Save"}}"#, KEY_SAVE), "detour");
        assert_eq!(score(r#"{"tool":"ui.act","arguments":{"action":"focus","window":"Code"}}"#, KEY_SAVE), "detour");
        assert_eq!(score(r#"{"finish":true}"#, KEY_SAVE), "early finish");
        assert_eq!(score(r#"{"finish":true}"#, &[None]), "right");
        assert_eq!(score(r#"{"tool":"input.key","arguments":{"combo":"ctrl+s"}}"#, &[None]), "no finish");
        assert_eq!(score(r#"{"tool":"input.type","arguments":{"window":"VS Code","text":"\nLine two."}}"#, TYPE), "right");
        assert_eq!(score("{\"tool\":", TYPE), "malformed");
    }

    #[test]
    fn typed_tools_only_in_the_typed_catalog() {
        let names = |v: Value| v.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert!(!names(catalog(false)).iter().any(|n| n.starts_with("media.")));
        assert!(names(catalog(true)).iter().any(|n| n == "media.control"));
        assert!(names(catalog(false)).iter().any(|n| n == "input.key"));
    }

    #[test]
    fn every_expected_tool_is_offered() {
        for case in CASES {
            let offered = names_in(&catalog(case.group == "typed"));
            for (tool, _) in case.expect.iter().flatten() {
                assert!(offered.contains(&tool.to_string()), "{tool} isn't in the catalog for {:?}", case.ask);
            }
        }
    }

    fn names_in(v: &Value) -> Vec<String> {
        v.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
    }
}
