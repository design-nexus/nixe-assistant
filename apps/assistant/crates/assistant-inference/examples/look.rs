//! Ask the model about a PNG: `look IMAGE.png "question"` (uses config.toml's model).
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (path, question) = (args.next().unwrap_or_default(), args.next().unwrap_or("What is on this screen?".into()));
    let cfg = assistant_core::Config::load()?;
    let llm = assistant_inference::LlamaCppBackend::spawn(&cfg)?;
    println!("projector: {:?}", llm.vision_model());
    let png = std::fs::read(&path)?;
    for n in 0..2 {
        let t = std::time::Instant::now();
        let answer = llm.look(png.clone(), &question, 120).await?;
        println!("look {} in {} ms: {answer}", n + 1, t.elapsed().as_millis());
    }
    llm.unload().await?;
    Ok(())
}
