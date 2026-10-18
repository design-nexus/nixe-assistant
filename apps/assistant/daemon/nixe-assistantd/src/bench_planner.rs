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
    Case { group: "typed", ask: "In Music, play the album Blue.", done: &[],
        expect: &[Some(("music.play", &[("what", M::Is("album")), ("name", M::Is("blue"))]))] },
    Case { group: "typed", ask: "In Music, shuffle my Running playlist.", done: &[],
        expect: &[Some(("music.play", &[("what", M::Is("playlist")), ("name", M::Has("running"))]))] },
    Case { group: "typed", ask: "In Music, play Hey Jude next.", done: &[],
        expect: &[Some(("music.queue", &[("name", M::Has("hey jude"))]))] },
    Case { group: "typed", ask: "In Music, what's playing?", done: &[],
        expect: &[Some(("music.status", &[]))] },
    Case { group: "typed", ask: "In Contacts, what's Anna Berg's phone number?", done: &[],
        expect: &[Some(("contacts.find", &[("query", M::Has("anna"))]))] },
    Case { group: "typed", ask: "In Contacts, add Sam Lee with the email sam@example.com.", done: &[],
        expect: &[Some(("contacts.add", &[("first", M::Is("sam")), ("last", M::Is("lee"))]))] },
    Case { group: "typed", ask: "In Contacts, add the email anna@work.example to Anna Berg.",
        done: &[("contacts.find", r#"{"query":"Anna Berg"}"#, r#"{"isError":false,"structuredContent":{"contacts":[{"id":"c-17","name":"Anna Berg","emails":[]}]}}"#)],
        expect: &[Some(("contacts.update", &[("id", M::Is("c-17")), ("add_email", M::Is("anna@work.example"))]))] },
    // Archive
    Case { group: "typed", ask: "In Archive, what's inside ~/Downloads/photos.zip?", done: &[],
        expect: &[Some(("archive.list", &[("path", M::Has("photos.zip"))]))] },
    Case { group: "typed", ask: "In Archive, extract ~/Downloads/photos.zip.", done: &[],
        expect: &[Some(("archive.extract", &[("path", M::Has("photos.zip"))]))] },
    Case { group: "typed", ask: "In Archive, extract ~/Downloads/photos.zip.",
        done: &[("archive.extract", r#"{"path":"~/Downloads/photos.zip"}"#, r#"{"isError":false,"structuredContent":{"extracted":24,"to":"/home/me/Downloads/photos"}}"#)],
        expect: &[None] },
    Case { group: "typed", ask: "In Archive, pack ~/notes and ~/todo.txt into ~/backup.zip.", done: &[],
        expect: &[Some(("archive.create", &[("path", M::Has("backup.zip"))]))] },
    // Chat
    Case { group: "typed", ask: "In Chat, find the conversation about the trip to Lisbon.", done: &[],
        expect: &[Some(("chat.search", &[("query", M::Has("lisbon"))]))] },
    Case { group: "typed", ask: "In Chat, read me the conversation about Lisbon.",
        done: &[("chat.search", r#"{"query":"Lisbon"}"#, r#"{"isError":false,"structuredContent":{"conversations":[{"id":42,"title":"Lisbon trip","model":"gemma"}]}}"#)],
        expect: &[Some(("chat.read", &[("id", M::Is("42"))]))] },
    Case { group: "typed", ask: "In Chat, ask what the capital of Peru is.", done: &[],
        expect: &[Some(("chat.send", &[("text", M::Has("peru"))]))] },
    // Fonts
    Case { group: "typed", ask: "In Fonts, install JetBrains Mono.", done: &[],
        expect: &[Some(("fonts.install", &[("family", M::Has("jetbrains"))])), Some(("fonts.search", &[("query", M::Has("jetbrains"))]))] },
    Case { group: "typed", ask: "In Fonts, is Inter installed?", done: &[],
        expect: &[Some(("fonts.installed", &[]))] },
    // Photos
    Case { group: "typed", ask: "In Photos, show me my photos from 2023.", done: &[],
        expect: &[Some(("photos.search", &[("year", M::Is("2023"))]))] },
    Case { group: "typed", ask: "In Photos, which albums do I have?", done: &[],
        expect: &[Some(("photos.albums", &[]))] },
    // Glimpse
    Case { group: "typed", ask: "Use Glimpse to find my tax return file.", done: &[],
        expect: &[Some(("glimpse.search", &[("query", M::Has("tax"))]))] },
    // Media Player
    Case { group: "typed", ask: "In Media Player, play the next episode of Severance.", done: &[],
        expect: &[Some(("video.play", &[("what", M::Is("show")), ("name", M::Has("severance"))])), Some(("video.play", &[("what", M::Is("episode")), ("name", M::Has("severance"))]))] },
    Case { group: "typed", ask: "In Media Player, pause the video.", done: &[],
        expect: &[Some(("video.control", &[("action", M::Is("pause"))])), Some(("video.control", &[("action", M::Is("toggle"))]))] },
    // PDF
    Case { group: "typed", ask: "In PDF, open ~/papers/attention.pdf at page 3.", done: &[],
        expect: &[Some(("pdf.open", &[("path", M::Has("attention.pdf")), ("page", M::Is("3"))]))] },
    Case { group: "typed", ask: "In the PDF window, go to page 12.", done: &[],
        expect: &[Some(("pdf.go_to", &[("page", M::Is("12"))]))] },
    Case { group: "typed", ask: "In PDF, find the word transformer.", done: &[],
        expect: &[Some(("pdf.search", &[("text", M::Has("transformer"))]))] },
    // Settings (shows a page; changes nothing)
    Case { group: "typed", ask: "In Settings, open the display page.", done: &[],
        expect: &[Some(("settings.open", &[("page", M::Has("display"))])), Some(("settings.pages", &[]))] },
    // Store (looks and shows; installs nothing)
    Case { group: "typed", ask: "In Store, is GIMP installed?", done: &[],
        expect: &[Some(("store.search", &[("query", M::Has("gimp"))]))] },
    Case { group: "typed", ask: "In Store, install GIMP.", done: &[],
        expect: &[Some(("store.open", &[("query", M::Has("gimp"))])), Some(("store.search", &[("query", M::Has("gimp"))]))] },
    // Tasks
    Case { group: "typed", ask: "In Tasks, what's using the most memory?", done: &[],
        expect: &[Some(("tasks.top", &[("by", M::Is("memory"))]))] },
    Case { group: "typed", ask: "In Tasks, what's using the most memory?",
        done: &[("tasks.top", r#"{"by":"memory"}"#, r#"{"isError":false,"structuredContent":{"processes":[{"name":"firefox","memory_mib":2310}],"memory_used_mib":11800}}"#)],
        expect: &[None] },
    // Torrents
    Case { group: "typed", ask: "Pause all my torrents.", done: &[],
        expect: &[Some(("torrents.pause", &[("all", M::Is("true"))]))] },
    Case { group: "typed", ask: "Add this magnet link to Torrents: magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567", done: &[],
        expect: &[Some(("torrents.add", &[("link", M::Has("magnet:?xt=urn:btih:0123456789abcdef"))]))] },
];

/// `media.*` as the media ability defines them (offered with every typed case, for Spotify).
/// The Nixe apps' own tools come from the nixe-apps ability itself (`app_tools`).
const MEDIA: &[(&str, &str, &str)] = &[
    ("media.status", "Read current song and playback state", r#"{"type":"object","properties":{"player":{"type":"string"}}}"#),
    ("media.control", "Play, pause, toggle, next, previous, or stop a media player", r#"{"type":"object","properties":{"player":{"type":"string"},"action":{"type":"string","enum":["play","pause","toggle","next","previous","stop"]}},"required":["action"]}"#),
];

/// The Nixe apps' tools exactly as the nixe-apps ability in this checkout lists them (all 55),
/// read from its own table so the bench can't drift from what the daemon is offered.
fn app_tools() -> &'static [Value] {
    static TOOLS: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    TOOLS.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let script = "import json, sys\nsys.path[:0] = sys.argv[1:3]\nimport main\nprint(json.dumps(main.request_tools(main.TOOLS)))";
        let out = std::process::Command::new("python3")
            .args(["-I", "-c", script])
            .arg(root.join("abilities/nixe-apps"))
            .arg(root.join("plugin-sdk"))
            .output()
            .expect("python3 runs the nixe-apps ability's tool table");
        assert!(out.status.success(), "nixe-apps tool table: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).expect("nixe-apps tools are JSON")
    })
}

/// Which app tools a typed case offers: the ones the daemon offers for this request (the app it
/// names), or every one of them.
#[derive(Clone, Copy, PartialEq)]
enum Apps {
    None,
    Named,
    All,
}

/// The catalog an app request sees in the daemon: computer and window tools (computer use
/// asks first; window tools are allowed by default), and for typed cases `media.*` and the app
/// tools.
fn catalog(ask: &str, apps: Apps) -> Value {
    let entry = |(name, description, schema): &(&str, &str, &str), permission: &str| {
        json!({"name": name, "description": description, "inputSchema": serde_json::from_str::<Value>(schema).unwrap_or(json!({})), "permission": permission})
    };
    let mut tools: Vec<Value> = assistant_tools::system::TOOLS.iter().filter(|t| t.0.starts_with("windows.")).map(|t| entry(t, "allow")).collect();
    tools.extend(assistant_tools::computer::TOOLS.iter().map(|t| entry(t, "ask")));
    if apps != Apps::None {
        tools.extend(MEDIA.iter().map(|t| entry(t, "allow")));
        let named = crate::session::app_tool_prefixes(ask);
        tools.extend(
            app_tools()
                .iter()
                .filter(|t| apps == Apps::All || named.iter().any(|p| t["name"].as_str().unwrap_or("").starts_with(p)))
                .map(|t| {
                    let mut t = t.clone();
                    t["permission"] = json!("allow");
                    t
                }),
        );
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
            let v = match &args[*key] {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            let v = v.as_str();
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
    let all = format!("{} app tools + media", app_tools().len());
    println!("{computer} computer-use steps, {typed} with the app's own tools offered: the named app's (as the daemon does), then all {all}\n");
    println!("| Model | Computer use | App tools | All {all} | Detours | Mistakes | Median step | p90 step | Median, all tools | VRAM added | File |\n|---|---|---|---|---|---|---|---|---|---|---|");
    for (label, path) in models() {
        let Some(path) = path else {
            println!("| {label} | not installed | — | — | — | — | — | — | — | — | — |");
            continue;
        };
        let out = tokio::process::Command::new(std::env::current_exe()?).args(["bench", "planner", "--one"]).arg(&path).output().await?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let Some(r) = stdout.lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok()) else {
            println!("| {label} | failed: {} | — | — | — | — | — | — | — | — | {} |", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""), gb(&path));
            continue;
        };
        let mistakes = r["mistakes"].as_object().map(|m| m.iter().map(|(k, v)| format!("{v} {k}")).collect::<Vec<_>>().join(", ")).filter(|s| !s.is_empty()).unwrap_or("none".into());
        println!(
            "| {label} | {}/{computer} | {}/{typed} | {}/{typed} | {} | {mistakes} | {} ms | {} ms | {} ms | {} | {} |",
            r["computer"], r["typed"], r["typed_all"], r["detours"], r["median_ms"], r["p90_ms"], r["all_median_ms"],
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

/// Where a case's next call comes from: the assistant's own llama.cpp, or a running
/// llama-server (Prism's fork, for packings the bundled llama.cpp can't load).
enum Generator {
    Local(assistant_inference::LlamaCppBackend, i32),
    Server(reqwest::Client, String),
}

impl Generator {
    async fn next_call(&self, system: &str, user: &str) -> Result<String> {
        match self {
            Generator::Local(llm, n_predict) => {
                let mut rx = llm
                    .generate_with_grammar(system, &[], user, *n_predict, 0.0, Arc::new(AtomicBool::new(false)), Some(crate::planner::GRAMMAR.into()))
                    .await?;
                let mut out = String::new();
                while let Some(tok) = rx.recv().await {
                    out.push_str(&tok?);
                }
                Ok(out)
            }
            // The same prompt and grammar; the model's own chat template, with thinking off
            // (the bundled path closes an empty think block the same way).
            Generator::Server(client, url) => {
                let body = json!({
                    "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
                    "grammar": crate::planner::GRAMMAR,
                    "temperature": 0.0,
                    "max_tokens": 768,
                    "cache_prompt": true,
                    "chat_template_kwargs": {"enable_thinking": false},
                });
                let reply: Value = client.post(format!("{url}/v1/chat/completions")).json(&body).send().await?.error_for_status()?.json().await?;
                Ok(reply["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
            }
        }
    }
}

/// Every case through `generator`: the counts, step times and answers, as one JSON object.
async fn run_cases(generator: &Generator) -> Result<Value> {
    let (mut times, mut all_times, mut answers) = (Vec::new(), Vec::new(), Vec::new());
    let (mut computer, mut typed, mut typed_all, mut detours) = (0, 0, 0, 0);
    let mut mistakes = serde_json::Map::new();
    // Detours and mistakes count the daemon's own catalogs; the all-tools pass only its rights.
    let runs = CASES
        .iter()
        .map(|c| (c, if c.group == "typed" { Apps::Named } else { Apps::None }))
        .chain(CASES.iter().filter(|c| c.group == "typed").map(|c| (c, Apps::All)));
    for (case, apps) in runs {
        let system = crate::planner::system(&catalog(case.ask, apps), "", "", "", true);
        let results: Vec<Value> = case
            .done
            .iter()
            .map(|(tool, args, result)| json!({"tool": tool, "arguments": serde_json::from_str::<Value>(args).unwrap_or_default(), "result": serde_json::from_str::<Value>(result).unwrap_or_default()}))
            .collect();
        let t = Instant::now();
        let out = generator.next_call(&system, &crate::planner::step(case.ask, &[], &results)).await?;
        let ms = t.elapsed().as_millis() as u64;
        let verdict = score(&out, case.expect);
        if apps == Apps::All {
            all_times.push(ms);
            typed_all += usize::from(verdict == "right");
            answers.push(json!({"group": "all app tools", "ask": case.ask, "steps_done": case.done.len(), "answer": out, "verdict": verdict}));
            continue;
        }
        times.push(ms);
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
    let mut sorted = times.clone();
    sorted.sort();
    let pick = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    all_times.sort();
    let all_median = all_times.get(all_times.len() / 2).copied().unwrap_or(0);
    Ok(json!({"computer": computer, "typed": typed, "typed_all": typed_all, "all_median_ms": all_median, "detours": detours, "mistakes": mistakes, "median_ms": pick(0.5), "p90_ms": pick(0.9), "answers": answers}))
}

/// One model over every case; prints one JSON line.
pub async fn planner_one(model: &Path) -> Result<()> {
    super::child_logging();
    let mut cfg = assistant_core::Config::load()?;
    cfg.llm.model = model.display().to_string();
    // The same context for every model (planner prompts are about 3k tokens, about 9k with
    // every app tool offered); a larger model's KV cache at the configured size may not fit
    // next to it.
    cfg.llm.ctx = 12288;
    let before = super::vram_mib();
    let generator = Generator::Local(assistant_inference::LlamaCppBackend::spawn(&cfg)?, cfg.llm.n_predict);
    let mut r = run_cases(&generator).await?;
    let peak = super::vram_mib();
    if let Generator::Local(llm, _) = generator {
        llm.unload().await?;
    }
    r["vram_added_mib"] = json!(match (before, peak) {
        (Some(b), Some(p)) => Some(p.saturating_sub(b)),
        _ => None,
    });
    println!("{r}");
    Ok(())
}

/// Every case against a running llama-server at `url` (`bench planner --server URL --label
/// NAME`): one row of the same table, and its answers in planner-<label>.json. Graphics
/// memory is the card's total in use, which is the server when the assistant is stopped.
pub async fn planner_server(url: &str, label: &str) -> Result<()> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build()?;
    let url = url.trim_end_matches('/').to_string();
    for _ in 0..120 {
        if client.get(format!("{url}/health")).send().await.is_ok_and(|r| r.status().is_success()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let (computer, typed) = (CASES.iter().filter(|c| c.group == "computer").count(), CASES.iter().filter(|c| c.group == "typed").count());
    let r = run_cases(&Generator::Server(client, url)).await?;
    let mistakes = r["mistakes"].as_object().map(|m| m.iter().map(|(k, v)| format!("{v} {k}")).collect::<Vec<_>>().join(", ")).filter(|s| !s.is_empty()).unwrap_or("none".into());
    println!("| Model | Computer use | App tools | All app tools | Detours | Mistakes | Median step | p90 step | Median, all tools | VRAM in use |\n|---|---|---|---|---|---|---|---|---|---|");
    println!(
        "| {label} | {}/{computer} | {}/{typed} | {}/{typed} | {} | {mistakes} | {} ms | {} ms | {} ms | {} |",
        r["computer"], r["typed"], r["typed_all"], r["detours"], r["median_ms"], r["p90_ms"], r["all_median_ms"],
        super::vram_mib().map(|v| format!("{v} MiB")).unwrap_or("not measured".into())
    );
    let safe: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    let log = super::bench_dir().join(format!("planner-{safe}.json"));
    let _ = tokio::fs::create_dir_all(super::bench_dir()).await;
    tokio::fs::write(&log, serde_json::to_vec_pretty(&r["answers"])?).await?;
    println!("\nAnswers: {}", log.display());
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
        let ask = "In Notepad, type hello";
        assert!(!names_in(&catalog(ask, Apps::None)).iter().any(|n| n.starts_with("media.") || n.starts_with("notepad.")));
        assert!(names_in(&catalog(ask, Apps::Named)).iter().any(|n| n == "media.control"));
        assert!(names_in(&catalog(ask, Apps::None)).iter().any(|n| n == "input.key"));
    }

    #[test]
    fn named_offers_one_app_and_all_offers_every_one() {
        let named = names_in(&catalog("In PDF, go to page 4", Apps::Named));
        assert!(named.iter().any(|n| n == "pdf.go_to") && !named.iter().any(|n| n.starts_with("write.")));
        let all = names_in(&catalog("In PDF, go to page 4", Apps::All));
        assert_eq!(all.iter().filter(|n| !n.starts_with("media.") && n.contains('.') && app_tools().iter().any(|t| t["name"] == n.as_str())).count(), 55);
    }

    #[test]
    fn numbers_and_flags_score_as_text() {
        let page: &[Alt] = &[Some(("pdf.go_to", &[("page", M::Is("12"))]))];
        assert_eq!(score(r#"{"tool":"pdf.go_to","arguments":{"page":12}}"#, page), "right");
        let all: &[Alt] = &[Some(("torrents.pause", &[("all", M::Is("true"))]))];
        assert_eq!(score(r#"{"tool":"torrents.pause","arguments":{"all":true}}"#, all), "right");
    }

    #[test]
    fn every_expected_tool_is_offered() {
        for case in CASES {
            let offered = names_in(&catalog(case.ask, if case.group == "typed" { Apps::Named } else { Apps::None }));
            for (tool, _) in case.expect.iter().flatten() {
                assert!(offered.contains(&tool.to_string()), "{tool} isn't in the catalog for {:?}", case.ask);
            }
        }
    }

    fn names_in(v: &Value) -> Vec<String> {
        v.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
    }
}
