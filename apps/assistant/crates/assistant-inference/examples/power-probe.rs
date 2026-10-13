//! Exercise resident model lifetime without microphone capture or speech playback.
use anyhow::{Context, Result};
use assistant_core::Config;
use assistant_inference::LlamaCppBackend;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;
async fn generate(backend: &LlamaCppBackend) -> Result<()> {
    let mut tokens = backend
        .generate_with_grammar(
            "Reply briefly.",
            &[],
            "Reply ready.",
            8,
            0.0,
            Arc::new(AtomicBool::new(false)),
            Some("root ::= \"ready\"".into()),
        )
        .await?;
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(token) = tokens.recv().await {
            text.push_str(&token?);
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("generation timed out")??;
    anyhow::ensure!(text.trim() == "ready", "unexpected probe output: {text:?}");
    anyhow::ensure!(backend.is_loaded(), "model did not reload");
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let backend = LlamaCppBackend::spawn(&Config::load()?)?;
    for _ in 0..300 {
        if backend.is_loaded() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::ensure!(backend.is_loaded(), "startup load failed");
    backend.set_battery(false).await?;
    anyhow::ensure!(backend.is_loaded(), "unchanged profile unloaded the model");
    backend.unload().await?;
    anyhow::ensure!(!backend.is_loaded(), "model remained resident");
    backend.set_battery(true).await?;
    anyhow::ensure!(backend.is_battery(), "battery mode failed");
    generate(&backend).await?;
    backend.unload().await?;
    anyhow::ensure!(!backend.is_loaded(), "CPU model remained resident");
    backend.set_battery(false).await?;
    anyhow::ensure!(!backend.is_battery(), "normal mode failed");
    generate(&backend).await?;
    backend.unload().await?;
    println!("Verified initial load, unload, CPU lazy reload, CPU unload, configured-device reload, and final unload.");
    Ok(())
}
