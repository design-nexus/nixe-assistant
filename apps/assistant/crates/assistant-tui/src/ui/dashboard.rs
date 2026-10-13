//! Tab 0: Dashboard rendering.

use crate::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let is_narrow = area.width < 90;

    let chunks = if is_narrow {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4), // Status Banner
                Constraint::Length(6), // LLM Telemetry
                Constraint::Length(6), // Speech & Audio Telemetry
                Constraint::Min(5),    // Quick Actions
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4), // Status Banner
                Constraint::Length(7), // Loaded Models & Telemetry (2 cols)
                Constraint::Min(5),    // Quick Actions
            ])
            .split(area)
    };

    render_status_banner(f, app, chunks[0]);

    if is_narrow {
        render_llm_card(f, app, chunks[1]);
        render_audio_card(f, app, chunks[2]);
        render_quick_actions(f, app, chunks[3]);
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1]);

        render_llm_card(f, app, cols[0]);
        render_audio_card(f, app, cols[1]);
        render_quick_actions(f, app, chunks[2]);
    }
}

fn render_status_banner(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let (status_text, status_style) = if let Some(st) = &app.daemon_status {
        if st.muted {
            ("● MUTED (MIC PAUSED)", t.warning())
        } else {
            match st.state.as_str() {
                "listening" => ("● LISTENING (MIC ARMED)", t.accent_text()),
                "speaking" => ("● SPEAKING (AUDIO PLAYING)", t.success()),
                "thinking" => ("● THINKING (LLM INFER)", t.warning()),
                _ => ("● ONLINE · IDLE", t.success()),
            }
        }
    } else if app.service_active {
        ("● STARTING UP...", t.warning())
    } else {
        ("○ DISABLED (MODELS UNLOADED)", t.error())
    };

    let ptt_mode = &app.config.assistant.mode;
    let wake_mode = if app.config.wake_word.enabled {
        "Enabled"
    } else {
        "Disabled"
    };

    let lines = if area.width >= 90 {
        vec![
            Line::from(vec![
                Span::styled("Status: ", t.muted_text()),
                Span::styled(status_text, status_style),
                Span::styled("   Persona: ", t.muted_text()),
                Span::styled(&app.config.assistant.name, t.accent_text()),
                Span::styled("   Mode: ", t.muted_text()),
                Span::styled(ptt_mode, t.card()),
            ]),
            Line::from(vec![
                Span::styled("Wake Word: ", t.muted_text()),
                Span::styled(
                    wake_mode,
                    if app.config.wake_word.enabled {
                        t.success()
                    } else {
                        t.muted_text()
                    },
                ),
                Span::styled(
                    format!("   HTTP: 127.0.0.1:{}", app.config.http.port),
                    t.card(),
                ),
                Span::styled("   Socket: assistant.sock", t.muted_text()),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled("Daemon: ", t.muted_text()),
                Span::styled(status_text, status_style),
                Span::styled("   Persona: ", t.muted_text()),
                Span::styled(&app.config.assistant.name, t.accent_text()),
            ]),
            Line::from(vec![
                Span::styled("Wake: ", t.muted_text()),
                Span::styled(
                    wake_mode,
                    if app.config.wake_word.enabled {
                        t.success()
                    } else {
                        t.muted_text()
                    },
                ),
                Span::styled(" · Mode: ", t.muted_text()),
                Span::styled(ptt_mode, t.card()),
                Span::styled(format!(" · Port: {}", app.config.http.port), t.muted_text()),
            ]),
        ]
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.base())
        .title(" System Status ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_llm_card(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let active_model = app.model_path();
    let model_raw = active_model
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("auto");

    let model_name = if model_raw.len() > 28 && area.width < 50 {
        super::truncate(model_raw, 26)
    } else {
        model_raw.to_string()
    };

    let flash_str = if app.config.llm.flash_attn {
        "Flash attention"
    } else {
        "Off"
    };
    let prefix_str = if app.config.llm.prefix_cache {
        "KV Cache"
    } else {
        "Off"
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Model: ", t.muted_text()),
            Span::styled(model_name, t.accent_text()),
        ]),
        Line::from(vec![
            Span::styled("GPU layers: ", t.muted_text()),
            Span::styled(
                if app.config.llm.n_gpu_layers < 0 {
                    "automatic".into()
                } else {
                    app.config.llm.n_gpu_layers.to_string()
                },
                t.card(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Speed: ", t.muted_text()),
            Span::styled(format!("{flash_str} · {prefix_str}"), t.success()),
            Span::styled("   Ctx: ", t.muted_text()),
            Span::styled(format!("{}t", app.config.llm.ctx), t.card()),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.base())
        .title(" LLM & GPU Inference ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_audio_card(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let voice = &app.config.tts.voice;
    let variant = &app.config.tts.variant;
    let vad_backend = &app.config.vad.backend;
    let threshold = app.vad_threshold();
    let vad_lbl = app.vad_sensitivity_label();
    let aec_str = if app.config.barge_in.aec {
        "WebRTC AEC"
    } else {
        "RMS"
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("TTS: ", t.muted_text()),
            Span::styled(format!("Kokoro-82M ({variant})"), t.accent_text()),
            Span::styled(" · Voice: ", t.muted_text()),
            Span::styled(
                format!("{voice} ({:.2}x)", app.config.tts.speed),
                t.success(),
            ),
        ]),
        Line::from(vec![
            Span::styled("VAD: ", t.muted_text()),
            Span::styled(format!("{vad_backend} {threshold:.2}"), t.card()),
            Span::styled(format!(" ({vad_lbl})"), t.accent_text()),
        ]),
        Line::from(vec![
            Span::styled("AEC: ", t.muted_text()),
            Span::styled(aec_str, t.success()),
            Span::styled("   VAD Keys: ", t.muted_text()),
            Span::styled("[←]/[→]", t.card()),
            Span::styled(" adjust", t.muted_text()),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.base())
        .title(" Speech, Audio & VAD ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_quick_actions(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let dirty_str = if app.is_dirty {
        " [Ctrl+S: UNSAVED CHANGES]"
    } else {
        ""
    };
    let dirty_style = if app.is_dirty {
        t.warning()
    } else {
        t.muted_text()
    };

    let service_lbl = if app.service_active {
        "Stop Daemon (Free RAM)"
    } else {
        "Start Daemon (Load Models)"
    };

    let lines = if area.width >= 80 {
        vec![
            Line::from(vec![
                Span::styled("Quick Control Keys:", t.accent_text()),
                Span::styled(dirty_str, dirty_style),
            ]),
            Line::from(vec![
                Span::styled("  [m] ", t.accent_text()),
                Span::raw("Mute / Pause Mic Capture   "),
                Span::styled("  [←]/[→] ", t.accent_text()),
                Span::raw("Adjust VAD Sensitivity"),
            ]),
            Line::from(vec![
                Span::styled("  [t] ", t.accent_text()),
                Span::raw("Voice Preview        "),
                Span::styled("  [s] ", t.accent_text()),
                Span::raw(service_lbl),
            ]),
            Line::from(vec![
                Span::styled("  [1-5] ", t.accent_text()),
                Span::raw("Switch View Tabs          "),
                Span::styled("  [Ctrl+S] ", t.accent_text()),
                Span::raw("Save & Hot-Reload"),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled("Quick Keys:", t.accent_text()),
                Span::styled(dirty_str, dirty_style),
            ]),
            Line::from(vec![
                Span::styled("  [m] ", t.accent_text()),
                Span::raw("Mute   "),
                Span::styled("[←/→] ", t.accent_text()),
                Span::raw("VAD Sens   "),
                Span::styled("[s] ", t.accent_text()),
                Span::raw("Start/Stop"),
            ]),
            Line::from(vec![
                Span::styled("  [t] ", t.accent_text()),
                Span::raw("Voice  "),
                Span::styled("[Ctrl+S] ", t.accent_text()),
                Span::raw("Save Config"),
            ]),
        ]
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.base())
        .title(" Controls ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}
