//! Print a window's accessibility tree: `cargo run -p assistant-desktop --example tree -- CLASS|TITLE`.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let query = std::env::args().nth(1).unwrap_or_default();
    let clients = assistant_omarchy::hypr::clients().await?;
    let window = assistant_omarchy::hypr::match_windows(&clients, &query).into_iter().next().cloned().ok_or_else(|| anyhow::anyhow!("no window matches {query}"))?;
    let t0 = std::time::Instant::now();
    let tree = assistant_desktop::a11y::tree(&window).await?;
    eprintln!("{} elements in {} ms (truncated: {})", tree.elements.len(), t0.elapsed().as_millis(), tree.truncated);
    println!("{}", serde_json::to_string_pretty(&tree)?);
    Ok(())
}
