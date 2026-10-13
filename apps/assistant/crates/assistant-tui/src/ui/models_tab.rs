//! Tab 1: Models & Audio settings rendering.

use crate::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(52), // Installed Models List
            Constraint::Percentage(48), // TTS & VAD Cards
        ])
        .split(area);

    render_models_list(f, app, chunks[0]);

    if area.width >= 90 {
        let bottom_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(46), // TTS
                Constraint::Percentage(54), // VAD
            ])
            .split(chunks[1]);
        render_tts_card(f, app, bottom_cols[0]);
        render_vad_card(f, app, bottom_cols[1]);
    } else {
        let bottom_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(chunks[1]);
        render_tts_card(f, app, bottom_rows[0]);
        render_vad_card(f, app, bottom_rows[1]);
    }
}

fn render_models_list(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let active_path = app.model_path();
    let is_narrow = area.width < 90;

    let items: Vec<ListItem> = if app.installed_models.is_empty() {
        vec![ListItem::new("No local GGUF models found in ~/.local/share/omarchy-assistant/models/ or ~/.lmstudio/models/")]
    } else {
        app.installed_models
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let is_selected = i == app.selected_model_idx;
                let is_active = *path == active_path;

                let filename = path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("unknown.gguf");

                let marker = if is_active { "[ACTIVE] " } else { "         " };
                let cursor = if is_selected { "> " } else { "  " };

                let style = if is_selected {
                    t.selected()
                } else if is_active {
                    t.accent_text()
                } else {
                    t.base()
                };

                let mut spans = vec![
                    Span::styled(cursor, style),
                    Span::styled(
                        marker,
                        if is_active {
                            t.success()
                        } else {
                            t.muted_text()
                        },
                    ),
                    Span::styled(filename, style),
                ];

                if !is_narrow {
                    let parent_str = path
                        .parent()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    if !parent_str.is_empty() {
                        spans.push(Span::styled(format!("  ({parent_str})"), t.muted_text()));
                    }
                }

                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let title = if is_narrow {
        format!(
            " Models ({}) — [Enter] Set Active ",
            app.installed_models.len()
        )
    } else {
        format!(
            " Installed GGUF Models ({}) — [j/k] Select · [Enter] Set Active ",
            app.installed_models.len()
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.base())
        .title(title);

    super::list(
        f,
        app,
        List::new(items).block(block),
        area,
        app.selected_model_idx,
        0,
    );
}

fn render_tts_card(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let voice = &app.config.tts.voice;
    let variant = &app.config.tts.variant;
    let speed = app.config.tts.speed;
    let is_narrow = area.width < 50;

    let lines = if is_narrow {
        vec![
            Line::from(vec![
                Span::styled("Voice: ", t.muted_text()),
                Span::styled(format!("[ {voice} ]"), t.selected()),
                Span::styled(" ([v])", t.muted_text()),
            ]),
            Line::from(vec![
                Span::styled("Model: ", t.muted_text()),
                Span::styled(format!("[ {variant} ]"), t.card()),
                Span::styled(" ([x])", t.muted_text()),
            ]),
            Line::from(vec![
                Span::styled("Speed: ", t.muted_text()),
                Span::styled(format!("[ {:.2}x ]", speed), t.card()),
                Span::styled(" ([+/-])", t.muted_text()),
            ]),
            Line::from(vec![
                Span::styled("Preview: ", t.muted_text()),
                Span::styled("[t]", t.accent_text()),
                Span::raw(" Test audio"),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled("Voice (Kokoro): ", t.muted_text()),
                Span::styled(format!("[ {voice} ]"), t.selected()),
                Span::styled("  ([v] cycle 11 voices)", t.muted_text()),
            ]),
            Line::from(vec![
                Span::styled("Model Variant:  ", t.muted_text()),
                Span::styled(format!("[ {variant} ]"), t.card()),
                Span::styled("  ([x] toggle INT8 / FP32)", t.muted_text()),
            ]),
            Line::from(vec![
                Span::styled("Speech Speed:   ", t.muted_text()),
                Span::styled(format!("[ {:.2}x ]", speed), t.card()),
                Span::styled("  ([+] / [-] adjust)", t.muted_text()),
            ]),
            Line::from(vec![
                Span::styled("Voice Preview:  ", t.muted_text()),
                Span::styled("[t]", t.accent_text()),
                Span::raw(" Play selected voice and speed"),
            ]),
        ]
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.base())
        .title(" Speech Synthesis (TTS) · Draft · Ctrl+S Apply ");

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_vad_card(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let backend = &app.config.vad.backend;
    let threshold = app.vad_threshold();
    let label = app.vad_sensitivity_label();
    let is_narrow = area.width < 50;
    let bar_len = if is_narrow { 10 } else { 16 };
    let bar = vad_meter(threshold, bar_len);
    let pct = (threshold * 100.0).round() as usize;

    let backend_display = if backend == "silero" {
        "Silero Neural VAD"
    } else {
        "Energy RMS Detector"
    };

    let lines = if is_narrow {
        vec![
            Line::from(vec![
                Span::styled("Engine: ", t.muted_text()),
                Span::styled(backend_display, t.card()),
                Span::styled(" ([b])", t.muted_text()),
            ]),
            Line::from(vec![
                Span::styled("Level:  ", t.muted_text()),
                Span::styled(label, t.accent_text()),
            ]),
            Line::from(vec![
                Span::styled("Meter:  ", t.muted_text()),
                Span::styled(bar, t.selected()),
                Span::styled(format!(" {pct}%"), t.card()),
            ]),
            Line::from(vec![
                Span::styled("Keys:   ", t.muted_text()),
                Span::styled("[←]/[[", t.accent_text()),
                Span::raw(" +Sens "),
                Span::styled("[→]/]]", t.accent_text()),
                Span::raw(" -Sens "),
                Span::styled("[Spc]", t.accent_text()),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled("Engine:           ", t.muted_text()),
                Span::styled(format!("[ {backend_display} ]"), t.card()),
                Span::styled("  ([b] toggle RMS)", t.muted_text()),
            ]),
            Line::from(vec![
                Span::styled("Sensitivity:      ", t.muted_text()),
                Span::styled(label, t.accent_text()),
                Span::styled(
                    format!("  (Threshold: {threshold:.2} / {pct}%)"),
                    t.muted_text(),
                ),
            ]),
            Line::from(vec![
                Span::styled("Meter:            ", t.muted_text()),
                Span::styled(bar, t.selected()),
                Span::styled(format!("  {pct}% trigger"), t.card()),
            ]),
            Line::from(vec![
                Span::styled("Sensitivity Keys: ", t.muted_text()),
                Span::styled("[←] / [ [ ]", t.accent_text()),
                Span::raw(" More Sensitive  ·  "),
                Span::styled("[→] / [ ] ]", t.accent_text()),
                Span::raw(" Less Sensitive  ·  "),
                Span::styled("[Space]", t.accent_text()),
                Span::raw(" Preset"),
            ]),
            Line::from(vec![
                Span::styled("Description:      ", t.muted_text()),
                Span::styled(
                    "Lower threshold detects soft whispers; higher filters ambient noise",
                    t.muted_text(),
                ),
            ]),
        ]
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.accent_text())
        .title(" Voice Activity Detection (VAD) Sensitivity · Draft · Ctrl+S Apply ");

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn vad_meter(threshold: f32, max_blocks: usize) -> String {
    let blocks = max_blocks.clamp(8, 24);
    let filled = ((threshold.clamp(0.0, 1.0)) * blocks as f32).round() as usize;
    let filled = filled.min(blocks);
    let empty = blocks.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}
