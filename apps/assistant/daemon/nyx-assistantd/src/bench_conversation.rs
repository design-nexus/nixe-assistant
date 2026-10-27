//! Reproducible local conversation samples with token latency and memory.

use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

fn rss_mib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| line.strip_prefix("VmRSS:")?.split_whitespace().next()?.parse::<u64>().ok()).map(|kb| kb / 1024)
}

pub async fn one(model: &Path) -> Result<()> {
    let mut cfg = assistant_core::Config::load()?;
    cfg.llm.model = model.display().to_string();
    cfg.llm.ctx = 12_288;
    cfg.llm.n_predict = 512;
    cfg.llm.temperature = 0.7;
    let baseline_vram = super::vram_mib();
    let baseline_rss = rss_mib();
    let loading = Instant::now();
    let llm = assistant_inference::LlamaCppBackend::spawn(&cfg)?;
    while !llm.is_loaded() && loading.elapsed() < std::time::Duration::from_secs(120) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    anyhow::ensure!(llm.is_loaded(), "model did not load for conversation benchmark");
    let load_ms = loading.elapsed().as_millis();
    let cases: &[(&str, &str, Vec<(String, String)>, &str, f32)] = &[
        ("naturalness", "You are a concise, warm spoken desktop assistant. Use natural sentences and no markdown.", vec![], "I am late to a meeting and my laptop audio is not working. What should I do first?", 0.7),
        ("persona", "You are Cipher, a calm, precise spoken assistant. Give direct answers in two short sentences. Never use markdown or claim an action was completed.", vec![], "I need to organize a messy project directory. What is your approach?", 0.7),
        ("reasoning", "You are a local desktop assistant. Explain findings before proposing changes. Do not claim to have run commands.", vec![], "A build fails with 'No space left on device', but the home partition has 80 GB free. Give the two most useful read-only checks and explain why.", 0.0),
        ("multi_turn", "You are a local desktop assistant. Answer briefly and preserve project context.", vec![("user".into(), "Remember: Atlas uses Rust and its release branch is stable.".into()), ("assistant".into(), "I will keep that project context for this conversation.".into())], "For Atlas, which language and branch did I mention?", 0.0),
    ];
    let mut results = Vec::<Value>::new();
    for (id, system, history, prompt, temperature) in cases {
        let start = Instant::now();
        let mut tokens = llm.generate(system, history, prompt, cfg.llm.n_predict, *temperature, Arc::new(AtomicBool::new(false))).await?;
        let mut first = None;
        let mut text = String::new();
        let mut count = 0usize;
        while let Some(chunk) = tokens.recv().await {
            if first.is_none() { first = Some(start.elapsed()); }
            text.push_str(&chunk?);
            count += 1;
        }
        let elapsed = start.elapsed();
        let ttft = first.unwrap_or(elapsed);
        let generated = elapsed.saturating_sub(ttft).as_secs_f64();
        results.push(json!({"case": id, "system": system, "history": history, "prompt": prompt, "answer": text, "ttft_ms": ttft.as_millis(), "total_ms": elapsed.as_millis(), "chunks": count, "chunks_per_sec_after_first": if generated > 0.0 { Some(count as f64 / generated) } else { None }}));
    }
    let peak_vram = super::vram_mib();
    let peak_rss = rss_mib();
    llm.unload().await?;
    println!("{}", json!({"model": model, "configuration": {"context": cfg.llm.ctx, "n_predict": cfg.llm.n_predict, "n_gpu_layers": cfg.llm.n_gpu_layers, "flash_attn": cfg.llm.flash_attn}, "load_ms": load_ms, "baseline_vram_mib": baseline_vram, "loaded_vram_mib": peak_vram, "vram_added_mib": peak_vram.zip(baseline_vram).map(|(peak, base)| peak.saturating_sub(base)), "baseline_rss_mib": baseline_rss, "loaded_rss_mib": peak_rss, "cases": results}));
    Ok(())
}
