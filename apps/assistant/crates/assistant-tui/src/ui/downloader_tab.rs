use crate::{
    app::App,
    models::{format_bytes, is_curated_installed, split_model, CURATED_MODELS},
};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Percentage(50),
        Constraint::Min(3),
    ])
    .split(area);
    let dl = app.download_progress_rx.borrow();
    f.render_widget(
        Paragraph::new(format!(
            "{:?}: {} · {} / {} · {}/s {}",
            dl.state,
            dl.filename,
            format_bytes(dl.downloaded_bytes),
            format_bytes(dl.total_bytes),
            format_bytes(dl.speed_bytes_sec),
            dl.error.as_deref().unwrap_or("")
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Download · c Cancel · / Search HF · u Direct URL"),
        ),
        chunks[0],
    );
    let curated = CURATED_MODELS
        .iter()
        .map(|m| {
            ListItem::new(format!(
                "{} ({}) {}\n  {}",
                m.name,
                m.size_str,
                if is_curated_installed(m, &app.installed_models) {
                    "[installed]"
                } else {
                    ""
                },
                m.description
            ))
        })
        .collect::<Vec<_>>();
    super::list(
        f,
        app,
        List::new(curated)
            .highlight_style(app.theme.selected())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(if app.download_pane == 0 {
                        "Curated [focused] · j/k Select · Enter Download"
                    } else {
                        "Curated · ← Focus"
                    }),
            ),
        chunks[1],
        app.selected_curated_idx,
        1,
    );
    let (items, selected, title) = if app.download_pane == 2 {
        (
            app.hf_files
                .iter()
                .map(|file| {
                    ListItem::new(format!(
                        "{file}{}",
                        if split_model(file) {
                            " [unsupported split model]"
                        } else {
                            ""
                        }
                    ))
                })
                .collect::<Vec<_>>(),
            app.selected_file_idx,
            format!("{} · Enter Download · ← Repositories", app.hf_repo),
        )
    } else {
        (
            app.hf_results
                .iter()
                .map(|repo| {
                    ListItem::new(format!(
                        "{} · {} downloads · {} likes",
                        repo.id, repo.downloads, repo.likes
                    ))
                })
                .collect::<Vec<_>>(),
            app.selected_hf_idx,
            format!(
                "Hugging Face '{}' · → Focus · Enter List files",
                app.hf_query
            ),
        )
    };
    super::list(
        f,
        app,
        List::new(items)
            .highlight_style(app.theme.selected())
            .block(Block::default().borders(Borders::ALL).title(title)),
        chunks[2],
        selected,
        2,
    );
}
