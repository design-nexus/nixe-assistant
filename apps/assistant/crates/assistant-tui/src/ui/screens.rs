//! The ten screens. Wide layouts show every pane; compact ones give the focused pane the
//! space, and F6, Navigate or the mouse reach the others.

use super::{heading, key, kv, list, on_off, pane, text, truncate};
use crate::app::{App, Tab};
use crate::models::{format_bytes, is_curated_installed, split_model, CURATED_MODELS};
use crate::theme::Theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, Paragraph, Wrap};
use ratatui::Frame;
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.active_tab {
        Tab::Dashboard => overview(frame, app, area),
        Tab::Models => audio(frame, app, area),
        Tab::Downloader => download(frame, app, area),
        Tab::Persona => persona(frame, app, area),
        Tab::Features => features(frame, app, area),
        Tab::Diagnostics => diagnostics(frame, app, area),
        Tab::Activity => activity(frame, app, area),
        Tab::Calibration => calibration(frame, app, area),
        Tab::Commands => commands(frame, app, area),
        Tab::Plugins => plugins(frame, app, area),
    }
}

fn muted(t: &Theme) -> Style {
    Style::default().fg(t.muted)
}
fn data(t: &Theme) -> Style {
    Style::default().fg(t.cyan)
}
fn bold(t: &Theme) -> Style {
    Style::default().fg(t.fg).add_modifier(Modifier::BOLD)
}
fn plain(t: &Theme) -> Style {
    Style::default().fg(t.fg)
}
fn compact(area: Rect) -> bool {
    area.width < 90 || area.height < 12
}
fn empty(t: &Theme, what: &str, k: &str, action: &str) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("{what}  "), muted(t))];
    if !k.is_empty() {
        spans.extend(key(t, k, action));
    }
    Line::from(spans)
}
fn file_name(path: &std::path::Path) -> String {
    path.file_name().and_then(|f| f.to_str()).unwrap_or("?").to_string()
}

// ---- 1 Overview ----------------------------------------------------------------------

fn overview(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = pane(frame, app, area, 0, "Overview");
    let t = app.theme.clone();
    let st = app.daemon_status.clone();
    let (state, color) = super::state(app, &t);

    let mut assistant = vec![heading(&t, "Assistant")];
    assistant.push(Line::from(vec![
        Span::styled(format!("{:<13}", app.config.assistant.name), bold(&t)),
        Span::styled(state, Style::default().fg(color)),
    ]));
    let mut row = key(&t, "s", if app.service_active { "stop" } else { "start" });
    if st.is_some() {
        row.extend(key(&t, "m", if st.as_ref().is_some_and(|s| s.muted) { "unmute" } else { "mute" }));
    }
    assistant.push(Line::from(row));
    let wake =
        Line::from(vec![Span::styled(format!("{:<13}", "Wake word"), muted(&t)), on_off(&t, app.config.wake_word.enabled)]);
    assistant.push(wake);
    assistant.push(kv(&t, "Talk", "hold Alt+Space", plain(&t)));
    let session = st.as_ref().map(|s| {
        let mode = s.session["mode"].as_str().unwrap_or("wake").replace('_', " ");
        match s.session["remaining_secs"].as_u64().filter(|r| *r > 0) {
            Some(r) => format!("{mode} · {r} s left"),
            None => mode,
        }
    });
    assistant.push(kv(&t, "Session", session.unwrap_or_else(|| "—".into()), plain(&t)));
    if st.as_ref().is_some_and(|s| s.tool_mode) {
        assistant.push(kv(&t, "Tool mode", "on", Style::default().fg(t.yellow)));
    }

    let model = app.model_path();
    let loaded = match &st {
        Some(s) if s.model_loaded => Span::styled("● loaded", Style::default().fg(t.green)),
        Some(_) => Span::styled("○ loads when you speak", muted(&t)),
        None => Span::styled("○ not running", muted(&t)),
    };
    let llm = vec![
        heading(&t, "Language model"),
        Line::from(Span::styled(file_name(&model), data(&t))),
        Line::from(vec![Span::styled(format!("{:<13}", "State"), muted(&t)), loaded]),
        kv(
            &t,
            "Graphics",
            if app.config.llm.n_gpu_layers < 0 {
                "all layers".to_string()
            } else {
                format!("{} layers", app.config.llm.n_gpu_layers)
            },
            plain(&t),
        ),
        kv(&t, "Context", format!("{} tokens", app.config.llm.ctx), plain(&t)),
        Line::from(vec![
            Span::styled(format!("{:<13}", "Speed-ups"), muted(&t)),
            Span::styled(
                [("flash attention", app.config.llm.flash_attn), ("prefix cache", app.config.llm.prefix_cache)]
                    .iter()
                    .filter(|(_, on)| *on)
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
                    .join(" · "),
                plain(&t),
            ),
        ]),
    ];

    let mut speech = vec![
        heading(&t, "Speech"),
        kv(&t, "Voice", format!("{} · {:.2}×", app.config.tts.voice, app.config.tts.speed), plain(&t)),
        kv(&t, "Voice model", format!("Kokoro {}", app.config.tts.variant), plain(&t)),
        kv(
            &t,
            "Detection",
            format!("{} · {:.2} {}", app.config.vad.backend, app.vad_threshold(), app.vad_sensitivity_label()),
            plain(&t),
        ),
        Line::from(vec![Span::styled(format!("{:<13}", "Echo cancel"), muted(&t)), on_off(&t, app.config.barge_in.aec)]),
    ];
    speech.push(Line::from(key(&t, "t", "voice preview")));

    let restart = st.as_ref().map(|s| s.restart_required.join(", ")).unwrap_or_default();
    let mut draft = vec![heading(&t, "Draft")];
    if app.is_dirty {
        draft.push(Line::from(Span::styled("▲ Changes not applied", Style::default().fg(t.yellow))));
        let mut row = key(&t, "Ctrl+S", "apply");
        row.extend(key(&t, "Ctrl+R", "apply and restart"));
        draft.push(Line::from(row));
    } else {
        draft.push(Line::from(Span::styled("✓ Saved", Style::default().fg(t.green))));
    }
    if !restart.is_empty() {
        draft.push(kv(&t, "Restart for", restart, Style::default().fg(t.yellow)));
    }
    draft.push(kv(
        &t,
        "HTTP",
        if app.config.http.enabled { format!("127.0.0.1:{}", app.config.http.port) } else { "off".into() },
        plain(&t),
    ));
    draft.push(kv(&t, "Socket", "assistant.sock (owner only)", plain(&t)));

    if inner.width >= 80 && inner.height >= 16 {
        let [top, bottom] = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).areas(inner);
        let [a, b] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).spacing(4).areas(top);
        let [c, d] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).spacing(4).areas(bottom);
        for (rect, lines) in [(a, assistant), (b, llm), (c, speech), (d, draft)] {
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), rect);
        }
        app.navigation.pane(inner, 0);
    } else {
        let mut lines = assistant;
        for part in [llm, speech, draft] {
            lines.push(Line::from(""));
            lines.extend(part);
        }
        text(frame, app, inner, 0, lines);
    }
}

// ---- 2 Audio -------------------------------------------------------------------------

fn audio(frame: &mut Frame, app: &mut App, area: Rect) {
    let focus = app.focused_pane();
    if compact(area) {
        match focus {
            0 => models_pane(frame, app, area),
            1 => voice_pane(frame, app, area),
            _ => detection_pane(frame, app, area),
        }
        return;
    }
    let [left, right] = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).spacing(1).areas(area);
    let [top, bottom] = Layout::vertical([Constraint::Length(8), Constraint::Min(7)]).areas(right);
    models_pane(frame, app, left);
    voice_pane(frame, app, top);
    detection_pane(frame, app, bottom);
}

fn models_pane(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = pane(frame, app, area, 0, &format!("Models {}", app.installed_models.len()));
    let t = &app.theme;
    let active = app.model_path();
    let wide = inner.width >= 60;
    let items = app
        .installed_models
        .iter()
        .map(|path| {
            let mut spans = vec![Span::styled(file_name(path), plain(t))];
            if wide {
                let dir = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("");
                spans.push(Span::styled(format!("  {dir}"), muted(t)));
            }
            if *path == active {
                spans.push(Span::styled("  ● in use", Style::default().fg(t.green)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let e = empty(t, "No GGUF models in ~/.local/share/nixe-assistant/models or ~/.lmstudio/models.", "3", "Download");
    list(frame, app, inner, 0, items, Some(app.selected_model_idx), e);
}

fn voice_pane(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = pane(frame, app, area, 1, "Voice");
    let t = &app.theme;
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{:<13}", "Voice"), muted(t)),
            Span::styled(app.config.tts.voice.clone(), data(t)),
            Span::raw("  "),
        ])
        .patch_style(Style::default()),
        kv(t, "Speed", format!("{:.2}×", app.config.tts.speed), data(t)),
        kv(t, "Model", format!("Kokoro {}", app.config.tts.variant), data(t)),
    ];
    let mut row = key(t, "v", "voice");
    row.extend(key(t, "+/-", "speed"));
    row.extend(key(t, "x", "model"));
    row.extend(key(t, "t", "preview"));
    lines.push(Line::from(""));
    lines.push(Line::from(row));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
    app.navigation.pane(inner, 1);
}

fn detection_pane(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = pane(frame, app, area, 2, "Speech detection");
    let t = &app.theme;
    let threshold = app.vad_threshold();
    let silero = app.config.vad.backend == "silero";
    let level = if silero { threshold } else { (threshold / 0.05).clamp(0.0, 1.0) };
    let width = inner.width.saturating_sub(14).clamp(6, 30) as usize;
    let filled = (level * width as f32).round() as usize;
    let mut lines = vec![
        kv(t, "Detector", if silero { "Silero (neural)" } else { "Energy (loudness)" }, data(t)),
        kv(t, "Threshold", format!("{threshold:.3} · {}", app.vad_sensitivity_label()), data(t)),
        Line::from(vec![
            Span::styled(format!("{:<13}", "Level"), muted(t)),
            Span::styled("█".repeat(filled.min(width)), Style::default().fg(t.cyan).bg(t.bg_raised)),
            Span::styled(" ".repeat(width - filled.min(width)), Style::default().bg(t.bg_raised)),
        ]),
        Line::from(Span::styled("Lower hears soft speech; higher ignores background noise.", muted(t))),
    ];
    let mut row = key(t, "←/→", "sensitivity");
    row.extend(key(t, "b", "detector"));
    lines.push(Line::from(row));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
    app.navigation.pane(inner, 2);
}

// ---- 3 Download ----------------------------------------------------------------------

fn download(frame: &mut Frame, app: &mut App, area: Rect) {
    let focus = app.focused_pane();
    if compact(area) {
        match focus {
            0 => recommended_pane(frame, app, area),
            1 => results_pane(frame, app, area),
            _ => files_pane(frame, app, area),
        }
        return;
    }
    let [left, right] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).spacing(1).areas(area);
    let [top, bottom] = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(right);
    recommended_pane(frame, app, left);
    results_pane(frame, app, top);
    files_pane(frame, app, bottom);
}

fn recommended_pane(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = pane(frame, app, area, 0, "Recommended");
    let t = app.theme.clone();
    let [rows, about] = Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).areas(inner);
    let items = CURATED_MODELS
        .iter()
        .map(|m| {
            let mut spans =
                vec![Span::styled(m.name.to_string(), plain(&t)), Span::styled(format!("  {}", m.size_str), muted(&t))];
            if is_curated_installed(m, &app.installed_models) {
                spans.push(Span::styled("  ● installed", Style::default().fg(t.green)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    list(frame, app, rows, 0, items, Some(app.selected_curated_idx), Line::from(""));
    if let Some(m) = CURATED_MODELS.get(app.selected_curated_idx) {
        frame.render_widget(
            Paragraph::new(vec![Line::from(Span::styled(m.description.to_string(), muted(&t)))]).wrap(Wrap { trim: true }),
            about,
        );
    }
}

fn results_pane(frame: &mut Frame, app: &mut App, area: Rect) {
    let title =
        if app.hf_query.is_empty() { "Search results".to_string() } else { format!("Search results · {}", app.hf_query) };
    let inner = pane(frame, app, area, 1, &truncate(&title, area.width.saturating_sub(6) as usize));
    let t = &app.theme;
    let items = app
        .hf_results
        .iter()
        .map(|r| {
            ListItem::new(Line::from(vec![
                Span::styled(r.id.clone(), plain(t)),
                Span::styled(format!("  {} downloads · {} likes", r.downloads, r.likes), muted(t)),
            ]))
        })
        .collect();
    let e = if app.operation_busy && app.hf_results.is_empty() {
        Line::from(Span::styled("Searching…", muted(t)))
    } else {
        empty(t, "No search yet.", "/", "search Hugging Face")
    };
    list(frame, app, inner, 1, items, Some(app.selected_hf_idx), e);
}

fn files_pane(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = if app.hf_repo.is_empty() { "Files".to_string() } else { format!("Files · {}", app.hf_repo) };
    let inner = pane(frame, app, area, 2, &truncate(&title, area.width.saturating_sub(6) as usize));
    let t = &app.theme;
    let items = app
        .hf_files
        .iter()
        .map(|f| {
            let mut spans = vec![Span::styled(f.clone(), plain(t))];
            if split_model(f) {
                spans.push(Span::styled("  split, not supported", Style::default().fg(t.yellow)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let dl = app.download_progress_rx.borrow().clone();
    let e = match dl.state {
        crate::models::DownloadState::Idle => {
            empty(t, "Pick a search result to list its GGUF files.", "u", "download from a URL")
        }
        state => Line::from(Span::styled(
            format!(
                "Last download: {state:?} · {} {}",
                dl.filename,
                if dl.total_bytes > 0 { format_bytes(dl.total_bytes) } else { String::new() }
            ) + &dl.error.map(|e| format!(" · {e}")).unwrap_or_default(),
            muted(t),
        )),
    };
    list(frame, app, inner, 2, items, Some(app.selected_file_idx), e);
}

// ---- 4 Persona -----------------------------------------------------------------------

fn persona(frame: &mut Frame, app: &mut App, area: Rect) {
    let focus = app.focused_pane();
    if compact(area) {
        match focus {
            0 => personas_pane(frame, app, area),
            1 => aliases_pane(frame, app, area),
            _ => persona_details(frame, app, area),
        }
        return;
    }
    let [left, right] = Layout::horizontal([Constraint::Length(38), Constraint::Min(30)]).spacing(1).areas(area);
    let [top, bottom] = Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(right);
    personas_pane(frame, app, left);
    aliases_pane(frame, app, top);
    persona_details(frame, app, bottom);
}

fn personas_pane(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = pane(frame, app, area, 0, "Personas");
    let t = &app.theme;
    let items = app
        .personas
        .iter()
        .map(|p| {
            let mut spans = vec![Span::styled(p.name.clone(), plain(t)), Span::styled(format!("  {}", p.voice), muted(t))];
            if p.id == app.config.assistant.persona_id {
                spans.push(Span::styled("  ● active", Style::default().fg(t.green)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let e = empty(t, "No personas.", "n", "new persona");
    list(frame, app, inner, 0, items, Some(app.selected_persona_idx), e);
}

fn aliases_pane(frame: &mut Frame, app: &mut App, area: Rect) {
    let count = app.selected_persona().map_or(0, |p| p.aliases.len());
    let inner = pane(frame, app, area, 1, &format!("Wake aliases {count}"));
    let t = &app.theme;
    let items = app
        .selected_persona()
        .map(|p| {
            p.aliases
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{:02}  ", i + 1), muted(t)),
                        Span::styled(a.clone(), plain(t)),
                        Span::styled(if i == 0 { "  exact" } else { "  sounds like" }, muted(t)),
                    ]))
                })
                .collect()
        })
        .unwrap_or_default();
    let e = empty(t, "No wake aliases.", "a", "add one");
    let selected = (app.focused_pane() == 1).then_some(app.selected_alias_idx);
    list(frame, app, inner, 1, items, selected, e);
}

fn persona_details(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = pane(frame, app, area, 2, "Details");
    let t = app.theme.clone();
    let Some(p) = app.selected_persona().cloned() else {
        frame.render_widget(Paragraph::new(empty(&t, "No persona selected.", "n", "new persona")), inner);
        return;
    };
    let active = p.id == app.config.assistant.persona_id;
    let prompt = if p.prompt.is_empty() { app.config.system_prompt() } else { p.prompt.replace("{name}", &p.name) };
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{:<13}", p.name), bold(&t)),
        if active {
            Span::styled("● active", Style::default().fg(t.green))
        } else {
            Span::styled("○ not active  ", muted(&t))
        },
    ])];
    if !active {
        lines.push(Line::from(key(&t, "Enter", "use this persona")));
    }
    lines.push(kv(&t, "Voice", p.voice.clone(), data(&t)));
    lines.push(Line::from(vec![
        Span::styled(format!("{:<13}", "Wake word"), muted(&t)),
        on_off(&t, app.config.wake_word.enabled),
    ]));
    lines.push(kv(&t, "Prefixes", p.prefixes.join(", "), plain(&t)));
    lines.push(kv(&t, "Talk", "hold Alt+Space", plain(&t)));
    lines.push(Line::from(""));
    lines.push(heading(&t, "Prompt"));
    lines.push(Line::from(Span::styled(prompt, plain(&t))));
    text(frame, app, inner, 2, lines);
}

// ---- 5 Features ----------------------------------------------------------------------

/// Name, value (on/off or a word), and what it does. Order matches `toggle_feature`.
fn feature_rows(app: &App) -> Vec<(&'static str, Result<bool, String>, String)> {
    let c = &app.config;
    let city = if c.location.city.is_empty() { "automatic".to_string() } else { c.location.city.clone() };
    vec![
        ("Wake word", Ok(c.wake_word.enabled), format!("Listen for “Hey {}” and the follow-up.", c.assistant.name)),
        ("Push-to-talk", Err(c.assistant.mode.clone()), "Hold Alt+Space to talk, release to send.".into()),
        ("Interrupting", Ok(c.barge_in.enabled), "Speak over a reply to stop or redirect it.".into()),
        ("Echo cancellation", Ok(c.barge_in.aec), "PipeWire's WebRTC filter, so the speakers don't trigger the mic.".into()),
        (
            "Speech detection",
            Err(format!("{:.2} {}", app.vad_threshold(), app.vad_sensitivity_label())),
            "How loud speech must be. ←/→ adjust.".into(),
        ),
        ("Coding agent handoff", Ok(c.agent.enabled), "Send coding requests (“ask claude…”) to a terminal agent.".into()),
        {
            let id = c.agent.default_agent.as_str();
            let known = assistant_tools::agent::agent(id);
            let choices = assistant_tools::agent::installed_agents().iter().map(|a| a.name).collect::<Vec<_>>().join(", ");
            let name = known.map_or(id.to_string(), |a| a.name.to_string());
            let missing = known.is_none_or(|a| !assistant_tools::agent::on_path(a.program));
            (
                "Default coding agent",
                Err(if missing { format!("{name} (not installed)") } else { name }),
                if choices.is_empty() {
                    "The agent that takes coding requests. None is installed; omarchy default agent installs one.".into()
                } else {
                    format!("The agent that takes coding requests. Enter cycles through the installed ones: {choices}.")
                },
            )
        },
        ("Flash attention", Ok(c.llm.flash_attn), "Faster attention on supported graphics cards.".into()),
        ("Prefix cache", Ok(c.llm.prefix_cache), "Reuse the system prompt and earlier turns between requests.".into()),
        ("HTTP API", Ok(c.http.enabled), format!("REST and event stream on 127.0.0.1:{}.", c.http.port)),
        ("Overlay", Ok(c.ui.enabled), "The on-screen card with the waveform, words and approvals.".into()),
        ("Clock", Ok(c.clock.enabled), "Answers “what time is it?” without the model.".into()),
        ("Location", Ok(c.location.enabled), format!("Approximate city for weather and local search: {city}. (l) edit city")),
    ]
}

fn features(frame: &mut Frame, app: &mut App, area: Rect) {
    let rows = feature_rows(app);
    let focus = app.focused_pane();
    let (list_area, details_area) = if compact(area) {
        if focus == 0 {
            (Some(area), None)
        } else {
            (None, Some(area))
        }
    } else {
        let [l, r] = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).spacing(1).areas(area);
        (Some(l), Some(r))
    };
    let t = app.theme.clone();
    if let Some(a) = list_area {
        let inner = pane(frame, app, a, 0, "Features");
        let w = inner.width.saturating_sub(4) as usize;
        let items = rows
            .iter()
            .map(|(name, value, _)| {
                let (word, style) = match value {
                    Ok(true) => ("● on".to_string(), Style::default().fg(t.green)),
                    Ok(false) => ("○ off".to_string(), muted(&t)),
                    Err(v) => (v.clone(), data(&t)),
                };
                let pad = w.saturating_sub(name.width() + word.width()).max(2);
                ListItem::new(Line::from(vec![
                    Span::styled(name.to_string(), plain(&t)),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(word, style),
                ]))
            })
            .collect();
        list(frame, app, inner, 0, items, Some(app.selected_feature_idx), Line::from(""));
    }
    if let Some(a) = details_area {
        let inner = pane(frame, app, a, 1, "Details");
        if let Some((name, value, about)) = rows.get(app.selected_feature_idx) {
            let mut lines = vec![heading(&t, name)];
            lines.push(match value {
                Ok(on) => Line::from(on_off(&t, *on)),
                Err(v) => Line::from(Span::styled(v.clone(), data(&t))),
            });
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(about.clone(), plain(&t))));
            lines.push(Line::from(""));
            lines.push(Line::from(key(&t, "Enter", if value.is_ok() { "toggle" } else { "change" })));
            lines.push(Line::from(Span::styled("Changes stay in the draft until (Ctrl+S) apply.", muted(&t))));
            text(frame, app, inner, 1, lines);
        }
    }
}

// ---- 6 Diagnostics, 7 Activity, 8 Calibration ----------------------------------------

/// Pretty JSON as lines, keys in the accent.
fn json_lines(t: &Theme, value: &Value) -> Vec<Line<'static>> {
    serde_json::to_string_pretty(value)
        .unwrap_or_default()
        .lines()
        .map(|l| match l.split_once("\": ") {
            Some((k, v)) => Line::from(vec![
                Span::styled(format!("{k}\":"), Style::default().fg(t.accent)),
                Span::styled(format!(" {v}"), plain(t)),
            ]),
            None => Line::from(Span::styled(l.to_string(), plain(t))),
        })
        .collect()
}

fn diagnostics(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = pane(frame, app, area, 0, "Report");
    let t = app.theme.clone();
    let lines = if app.diagnostics.is_null() {
        let mut v = vec![Line::from(Span::styled("No report yet. It checks the models, audio, plugins and browser.", muted(&t)))];
        v.push(Line::from(if app.service_active {
            key(&t, "r", "run diagnostics")
        } else {
            key(&t, "1", "start the assistant on Overview")
        }));
        v
    } else {
        json_lines(&t, &app.diagnostics)
    };
    text(frame, app, inner, 0, lines);
}

fn turn_parts(turn: &Value) -> (String, String, Option<u64>) {
    let events = turn["events"].as_array().cloned().unwrap_or_default();
    let said = events.iter().find(|e| e["type"] == "transcript").and_then(|e| e["text"].as_str()).unwrap_or("").to_string();
    let reply =
        events.iter().rev().find(|e| e["type"] == "assistant_text").and_then(|e| e["text"].as_str()).unwrap_or("").to_string();
    let total = events.iter().find(|e| e["type"] == "timing").and_then(|e| e["total_ms"].as_u64());
    (said, reply, total)
}

fn clock(ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms as i64).single().map(|d| d.format("%H:%M:%S").to_string()).unwrap_or_default()
}

fn activity(frame: &mut Frame, app: &mut App, area: Rect) {
    let turns: Vec<Value> = app.activity["turns"].as_array().cloned().unwrap_or_default().into_iter().rev().collect();
    let focus = app.focused_pane();
    let (list_area, details_area) = if compact(area) {
        if focus == 0 {
            (Some(area), None)
        } else {
            (None, Some(area))
        }
    } else {
        let [l, r] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).spacing(1).areas(area);
        (Some(l), Some(r))
    };
    let t = app.theme.clone();
    let tool_mode = app.daemon_status.as_ref().is_some_and(|s| s.tool_mode);
    if let Some(a) = list_area {
        let inner = pane(frame, app, a, 0, &format!("Turns {}", turns.len()));
        let items = turns
            .iter()
            .map(|turn| {
                let (said, reply, _) = turn_parts(turn);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{}  ", clock(turn["at_ms"].as_u64().unwrap_or(0))), muted(&t)),
                    Span::styled(if said.is_empty() { "(no words)".into() } else { said }, plain(&t)),
                    Span::styled(if reply.is_empty() { String::new() } else { format!(" → {reply}") }, muted(&t)),
                ]))
            })
            .collect();
        let e = if app.daemon_status.is_none() {
            empty(&t, "The assistant isn't running.", "1", "Overview")
        } else {
            Line::from(Span::styled("No turns yet. Hold Alt+Space and talk.", muted(&t)))
        };
        list(frame, app, inner, 0, items, Some(app.selected_turn), e);
    }
    if let Some(a) = details_area {
        let inner = pane(frame, app, a, 1, "Details");
        let mut lines = Vec::new();
        if let Some(turn) = turns.get(app.selected_turn) {
            let (said, reply, total) = turn_parts(turn);
            lines.push(heading(&t, &format!("Turn {} · {}", turn["turn_id"], clock(turn["at_ms"].as_u64().unwrap_or(0)))));
            let dash = |s: String| if s.is_empty() { "—".to_string() } else { s };
            lines.push(kv(&t, "You", dash(said), plain(&t)));
            lines.push(kv(&t, "Reply", dash(reply), plain(&t)));
            if let Some(ms) = total {
                lines.push(kv(&t, "First audio", format!("{ms} ms"), data(&t)));
            }
            for e in turn["events"].as_array().into_iter().flatten() {
                match e["type"].as_str() {
                    Some("error") => {
                        lines.push(kv(&t, "Error", e["message"].as_str().unwrap_or("").to_string(), Style::default().fg(t.red)))
                    }
                    Some("timing") => {
                        let stages = e["stages"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|s| s["ms"].as_u64().unwrap_or(0) > 0)
                            .map(|s| format!("{} {} ms", s["name"].as_str().unwrap_or(""), s["ms"]))
                            .collect::<Vec<_>>()
                            .join(" · ");
                        lines.push(kv(&t, "Stages", stages, muted(&t)));
                    }
                    Some("task") => {
                        lines.push(Line::from(""));
                        lines.push(heading(&t, "Task"));
                        lines.extend(json_lines(&t, e));
                    }
                    _ => {}
                }
            }
        } else {
            lines.push(Line::from(Span::styled("Select a turn to see what was heard, said and done.", muted(&t))));
        }
        lines.push(Line::from(""));
        let mut row = key(&t, "c", "clear");
        row.extend(key(&t, "Ctrl+T", if tool_mode { "tool mode off" } else { "tool mode on" }));
        lines.push(Line::from(row));
        text(frame, app, inner, 1, lines);
    }
}

fn calibration(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = pane(frame, app, area, 0, "Calibration");
    let t = app.theme.clone();
    let mut lines = vec![
        Line::from(Span::styled(
            "Stay quiet for five seconds, then speak normally for five seconds. The assistant measures the room and your voice.",
            plain(&t),
        )),
        Line::from(""),
    ];
    let mut row = key(&t, "Enter", "start");
    if !app.calibration["suggestions"].is_null() {
        row.extend(key(&t, "a", "copy suggestions to the draft"));
    }
    if app.operation_busy {
        row.extend(key(&t, "Ctrl+X", "cancel"));
    }
    lines.push(Line::from(row));
    if !app.calibration.is_null() {
        lines.push(Line::from(""));
        lines.push(heading(&t, "Result"));
        lines.extend(json_lines(&t, &app.calibration));
    } else if !app.service_active {
        lines.push(Line::from(Span::styled("The assistant isn't running; start it on Overview first.", muted(&t))));
    }
    text(frame, app, inner, 0, lines);
}

// ---- 9 Commands ----------------------------------------------------------------------

fn route_row(t: &Theme, v: &Value) -> ListItem<'static> {
    let phrases =
        v["phrases"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    let target = v["tool"]
        .as_str()
        .map(String::from)
        .or_else(|| v["exec"].as_array().and_then(|a| a.first()).and_then(Value::as_str).map(String::from))
        .unwrap_or_default();
    ListItem::new(Line::from(vec![Span::styled(phrases, plain(t)), Span::styled(format!("  {target}"), muted(t))]))
}

fn commands(frame: &mut Frame, app: &mut App, area: Rect) {
    let focus = app.focused_pane();
    let t = app.theme.clone();
    let lists = |frame: &mut Frame, app: &mut App, a: Rect, which: usize| {
        let commands = which == 1;
        let entries = app.route_list(commands);
        let inner = pane(frame, app, a, which, &format!("{} {}", if commands { "Commands" } else { "Apps" }, entries.len()));
        let items = entries.iter().map(|v| route_row(&t, v)).collect();
        let selected = (app.commands_pane == commands).then_some(app.selected_route);
        let e = empty(&t, if commands { "No commands." } else { "No apps." }, "n", "new entry");
        list(frame, app, inner, which, items, selected, e);
    };
    if compact(area) {
        match focus {
            0 | 1 => lists(frame, app, area, focus),
            _ => route_details(frame, app, area),
        }
        return;
    }
    let [left, right] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).spacing(1).areas(area);
    let [top, bottom] = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(left);
    lists(frame, app, top, 0);
    lists(frame, app, bottom, 1);
    route_details(frame, app, right);
}

fn route_details(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = pane(frame, app, area, 2, "Details");
    let t = app.theme.clone();
    let entries = app.route_entries();
    let mut lines = Vec::new();
    if let Some(selected) = entries.get(app.selected_route) {
        let tool = selected["tool"].as_str().unwrap_or("app.launch");
        let cfg = &app.config;
        let rules = cfg.permissions.rules();
        let policy = assistant_policy::decide(
            tool,
            &assistant_policy::Context {
                available: true,
                trusted: true,
                tool_rule: cfg.mcp.permissions.get(tool).and_then(|s| assistant_policy::Decision::parse(s)),
                rules: &rules,
                mode: cfg.agent.mode,
                resource_approved: false,
                runs_code: false,
            },
        )
        .decision();
        let (word, color) = match policy {
            assistant_policy::Decision::Allow => ("runs without asking", t.green),
            assistant_policy::Decision::Deny => ("refused", t.red),
            _ => ("asks first", t.yellow),
        };
        lines.push(kv(&t, "Permission", word, Style::default().fg(color)));
        lines.push(Line::from(""));
        lines.extend(json_lines(&t, selected));
    } else {
        lines.push(Line::from(Span::styled("Nothing selected.", muted(&t))));
    }
    lines.push(Line::from(""));
    lines.push(heading(&t, "Routing preview"));
    if app.route_preview.is_empty() {
        lines.push(Line::from(key(&t, "p", "route a phrase (nothing runs)")));
    } else {
        lines.push(Line::from(Span::styled(app.route_preview.clone(), plain(&t))));
    }
    text(frame, app, inner, 2, lines);
}

// ---- 0 Plugins -----------------------------------------------------------------------

fn plugins(frame: &mut Frame, app: &mut App, area: Rect) {
    let installed = app
        .daemon_status
        .as_ref()
        .map(|s| s.plugins.clone())
        .unwrap_or_else(|| assistant_tools::plugins::PluginManager::new().inspect());
    let entries = installed.as_array().cloned().unwrap_or_default();
    let focus = app.focused_pane();
    let (list_area, details_area) = if compact(area) {
        if focus == 0 {
            (Some(area), None)
        } else {
            (None, Some(area))
        }
    } else {
        let [l, r] = Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).spacing(1).areas(area);
        (Some(l), Some(r))
    };
    let t = app.theme.clone();
    let health =
        |id: &str| entries.iter().find(|p| p["id"] == id).and_then(|p| p["health"].as_str()).unwrap_or("offline").to_string();
    let health_style = |h: &str| match h {
        "running" => Style::default().fg(t.green),
        "failed" => Style::default().fg(t.red),
        _ => muted(&t),
    };
    if let Some(a) = list_area {
        let inner = pane(frame, app, a, 0, &format!("Plugins {}", app.plugin_draft.len()));
        let items = app
            .plugin_draft
            .iter()
            .map(|(id, s)| {
                let h = health(id);
                ListItem::new(Line::from(vec![
                    Span::styled(if s.enabled { "☑ " } else { "☐ " }, Style::default().fg(t.accent)),
                    Span::styled(id.clone(), plain(&t)),
                    Span::styled(format!("  ● {h}"), health_style(&h)),
                ]))
            })
            .collect();
        let e = Line::from(Span::styled("No plugins installed. Run: nixe-assistant plugins install …", muted(&t)));
        list(frame, app, inner, 0, items, Some(app.selected_plugin), e);
    }
    if let Some(a) = details_area {
        let inner = pane(frame, app, a, 1, "Details");
        let mut lines = Vec::new();
        if let Some((id, draft)) = app.plugin_draft.iter().nth(app.selected_plugin).map(|(i, d)| (i.clone(), d.clone())) {
            let status = entries.iter().find(|p| p["id"] == id.as_str()).cloned().unwrap_or_default();
            let m = &status["manifest"];
            let h = health(&id);
            lines.push(Line::from(vec![
                Span::styled(format!("{id}  "), bold(&t)),
                Span::styled(format!("version {}  ", m["version"].as_str().unwrap_or("?")), muted(&t)),
                Span::styled(format!("● {h}"), health_style(&h)),
            ]));
            lines.push(kv(&t, "In the draft", if draft.enabled { "enabled" } else { "disabled" }, plain(&t)));
            lines.push(kv(&t, "Saved", status["saved"]["enabled"].to_string(), plain(&t)));
            lines.push(kv(&t, "Running", status["running_enabled"].to_string(), plain(&t)));
            if let Some(err) = status["error"].as_str() {
                lines.push(kv(&t, "Last error", err.to_string(), Style::default().fg(t.red)));
            }
            lines.push(kv(&t, "Needs", m["dependencies"].to_string(), plain(&t)));
            lines.push(kv(&t, "Optional", m["optional_dependencies"].to_string(), plain(&t)));
            lines.push(Line::from(""));
            lines.push(heading(&t, "Tools and permissions"));
            let names = m["tools"].as_array().cloned().unwrap_or_default();
            if let Some(tools) = app.daemon_status.as_ref().and_then(|s| s.mcp_tools.as_array()) {
                for tool in tools.iter().filter(|tool| names.contains(&tool["name"])) {
                    let perm = tool["permission"].as_str().unwrap_or("?");
                    let style = match perm {
                        "allow" => Style::default().fg(t.green),
                        "deny" => Style::default().fg(t.red),
                        _ => Style::default().fg(t.yellow),
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {:<28}", tool["name"].as_str().unwrap_or("")), plain(&t)),
                        Span::styled(perm.to_string(), style),
                    ]));
                }
            }
            lines.push(Line::from(""));
            lines.push(heading(&t, "Settings"));
            lines.extend(json_lines(&t, &draft.settings));
            lines.push(kv(&t, "Schema", m["settings_schema"].to_string(), muted(&t)));
            lines.push(Line::from(""));
            lines.push(kv(&t, "Starts", format!("{} {}", m["executable"].as_str().unwrap_or(""), m["args"]), muted(&t)));
            lines.push(kv(&t, "Source", status["saved"]["source"].to_string(), muted(&t)));
            lines.push(kv(&t, "Data", status["data_directory"].as_str().unwrap_or("").to_string(), muted(&t)));
            lines.push(Line::from(""));
            let mut row = key(&t, "Space", "toggle");
            row.extend(key(&t, "e", "settings"));
            row.extend(key(&t, "r", "retry"));
            lines.push(Line::from(row));
        } else {
            lines.push(Line::from(Span::styled("No plugin selected.", muted(&t))));
        }
        text(frame, app, inner, 1, lines);
    }
}
