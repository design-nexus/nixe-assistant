//! Compare a model on the assistant's two jobs: `gen MODEL.gguf`.
//! 1) a short spoken answer (time to first token, any <think> leak); 2) planner steps.
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const GRAMMAR: &str = r#"root ::= ws (call | finish) ws
call ::= "{" ws "\"tool\"" ws ":" ws string ws "," ws "\"arguments\"" ws ":" ws object ws "}"
finish ::= "{" ws "\"finish\"" ws ":" ws "true" ws "}"
value ::= object | array | string | number | "true" | "false" | "null"
object ::= "{" ws (string ws ":" ws value (ws "," ws string ws ":" ws value)*)? ws "}"
array ::= "[" ws (value (ws "," ws value)*)? ws "]"
string ::= "\"" ([^"\\\x00-\x1F] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F]{4}))* "\""
number ::= "-"? ("0" | [1-9] [0-9]*) ("." [0-9]+)? ([eE] [+-]? [0-9]+)?
ws ::= [ \t\n\r]*
"#;

async fn run(llm: &assistant_inference::LlamaCppBackend, system: &str, user: &str, grammar: bool) -> anyhow::Result<(String, u128, u128)> {
    let t = std::time::Instant::now();
    let mut rx = llm.generate_with_grammar(system, &[], user, if grammar { 200 } else { 1500 }, 0.0, Arc::new(AtomicBool::new(false)), grammar.then(|| GRAMMAR.to_string())).await?;
    let (mut out, mut first) = (String::new(), 0);
    while let Some(tok) = rx.recv().await {
        if first == 0 {
            first = t.elapsed().as_millis();
        }
        out.push_str(&tok?);
    }
    Ok((out, first, t.elapsed().as_millis()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let model = std::env::args().nth(1).unwrap();
    let mut cfg = assistant_core::Config::load()?;
    cfg.llm.model = model.clone();
    if std::env::var("NO_PREFIX").is_ok() {
        cfg.llm.prefix_cache = false;
    }
    let llm = assistant_inference::LlamaCppBackend::spawn(&cfg)?;
    let persona = format!("{}", cfg.assistant.prompt);
    for q in ["What's the capital of France?", "Give me one tip for sleeping better."] {
        let (out, first, all) = run(&llm, &persona, q, false).await?;
        println!("CHAT {q:?}: first token {first} ms, total {all} ms, think_leak={}: {:?}", out.contains("<think>"), out.chars().take(160).collect::<String>());
    }
    let tools = r#"[{"name":"ui.act","description":"Operate an element of an app window by name: press, focus, type or set_text.","args":"window, name, action, text"},{"name":"input.type","description":"Type text into an app window's text field.","args":"window, text, field"},{"name":"input.key","description":"Press a key or combination in an app window.","args":"window, combo"},{"name":"windows.focus","description":"Focus a window.","args":"window"},{"name":"ui.tree","description":"Read an app window's accessibility tree.","args":"window"}]"#;
    let system = format!("Plan one tool call or finish. Reply with JSON only: {{\"tool\":\"registered.name\",\"arguments\":{{}}}} or {{\"finish\":true}}. Only choose tools in this catalog: {tools}. For app windows (always give the window's name in window): to type, use input.type; to press a button or menu item, use ui.act with its name; press keys with input.key. Do every step the request asks before finishing. Once the task is fulfilled, finish; never repeat a completed action.");
    let think = false;
    let cases = [
        ("In the Notepad window, open a new tab.", "[]", "[]"),
        ("In the Notepad window, open a new tab.", r#"[{"tool":"windows.focus","arguments":{"window":"Notepad"}}]"#, r#"[{"tool":"windows.focus","result":{"verified":true}}]"#),
        ("In VS Code, go to the end of the file, type Line two. and save it.", "[]", "[]"),
        ("In VS Code, go to the end of the file, type Line two. and save it.", r#"[{"tool":"input.key","arguments":{"window":"VS Code","combo":"ctrl+end"}}]"#, r#"[{"tool":"input.key","result":{"verified":true}}]"#),
        ("In VS Code, go to the end of the file, type Line two. and save it.", r#"[{"tool":"input.key","arguments":{"window":"VS Code","combo":"ctrl+end"}},{"tool":"input.type","arguments":{"window":"VS Code","text":"\nLine two."}}]"#, r#"[{"tool":"input.type","result":{"verified":true}}]"#),
    ];
    for (ask, done, results) in cases {
        let user = format!("{ask}\nAlready dispatched actions (do not repeat completed actions): {done}\nPrevious results: {results}\nNext call or finish:");
        let (out, _, all) = run(&llm, &system, &user, !think).await?;
        // With thinking, the plan is the JSON after the reasoning.
        let out = match out.rfind("</think>") { Some(i) => out[i + 8..].trim().to_string(), None => out };
        println!("PLAN after {} step(s) ({all} ms): {out}", done.matches("\"tool\"").count());
    }
    llm.unload().await?;
    Ok(())
}
