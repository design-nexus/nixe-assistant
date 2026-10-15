//! Which element a name picks in a window: `pick WINDOW NAME`.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (win, name) = (a.next().unwrap_or_default(), a.next().unwrap_or_default());
    let clients = assistant_omarchy::hypr::clients().await?;
    let w = assistant_omarchy::hypr::match_windows(&clients, &win).into_iter().next().cloned().unwrap();
    let tree = assistant_desktop::a11y::tree(&w).await?;
    for e in assistant_desktop::a11y::find(&tree, Some(&name), None) {
        println!("found {} {:?} {:?} actions={:?}", e.id, e.role, e.name, e.actions);
    }
    println!("choose -> {:?}", assistant_tools::computer::choose(&tree, &name).map(|e| e.id));
    Ok(())
}
