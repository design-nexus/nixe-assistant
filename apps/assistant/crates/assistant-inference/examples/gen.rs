//! Check a model's spoken answers by hand: `gen MODEL.gguf` prints time to first token and
//! whether a <think> block leaks. Planner steps are compared with `nixe-assistant bench planner`.
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let model = std::env::args().nth(1).unwrap();
    let mut cfg = assistant_core::Config::load()?;
    cfg.llm.model = model.clone();
    if std::env::var("NO_PREFIX").is_ok() {
        cfg.llm.prefix_cache = false;
    }
    let llm = assistant_inference::LlamaCppBackend::spawn(&cfg)?;
    let persona = cfg.assistant.prompt.to_string();
    for q in ["What's the capital of France?", "Give me one tip for sleeping better."] {
        let t = std::time::Instant::now();
        let mut rx = llm.generate_with_grammar(&persona, &[], q, 1500, 0.0, Arc::new(AtomicBool::new(false)), None).await?;
        let (mut out, mut first) = (String::new(), 0);
        while let Some(tok) = rx.recv().await {
            if first == 0 {
                first = t.elapsed().as_millis();
            }
            out.push_str(&tok?);
        }
        println!("CHAT {q:?}: first token {first} ms, total {} ms, think_leak={}: {:?}", t.elapsed().as_millis(), out.contains("<think>"), out.chars().take(160).collect::<String>());
    }
    llm.unload().await?;
    Ok(())
}
