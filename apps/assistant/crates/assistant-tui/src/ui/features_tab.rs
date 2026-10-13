//! Tab 4: Extensible feature flags and system integrations rendering.

use crate::app::App;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

struct FeatureItem {
    name: &'static str,
    status: String,
    description: String,
}

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let is_narrow = area.width < 95;
    let name = &app.config.assistant.name;

    let items = vec![
        FeatureItem {
            name: "Hands-Free Wake Word",
            status: if app.config.wake_word.enabled {
                "ENABLED".into()
            } else {
                "DISABLED".into()
            },
            description: format!(
                "Listen for voice trigger phrase ('Hey {name}') and follow-up speech"
            ),
        },
        FeatureItem {
            name: "Push-to-Talk (PTT)",
            status: app.config.assistant.mode.clone(),
            description: "Hold Alt+Space hotkey to capture voice, release to submit".into(),
        },
        FeatureItem {
            name: "Barge-In Interruption",
            status: if app.config.barge_in.enabled {
                "ENABLED".into()
            } else {
                "DISABLED".into()
            },
            description: "Allow speaking over TTS playback to immediately cancel and listen".into(),
        },
        FeatureItem {
            name: "PipeWire WebRTC AEC",
            status: if app.config.barge_in.aec {
                "ENABLED".into()
            } else {
                "DISABLED".into()
            },
            description: "Hardware echo cancellation filter preventing speaker echo feedback"
                .into(),
        },
        FeatureItem {
            name: "VAD Sensitivity Threshold",
            status: format!(
                "{:.2} ({})",
                app.vad_threshold(),
                app.vad_sensitivity_label()
            ),
            description: "Speech detector threshold. [Space] presets · [←]/[→] adjust".into(),
        },
        FeatureItem {
            name: "Coding Agent Handoff",
            status: if app.config.agent.enabled {
                "ENABLED".into()
            } else {
                "DISABLED".into()
            },
            description: "Route coding requests ('ask claude', 'code...') to dedicated terminal"
                .into(),
        },
        FeatureItem {
            name: "Default Coding Agent",
            status: app.config.agent.default_agent.to_uppercase(),
            description: "Default CLI agent spawned: agy (Antigravity), claude, codex, or cursor"
                .into(),
        },
        FeatureItem {
            name: "CUDA Flash Attention",
            status: if app.config.llm.flash_attn {
                "ENABLED".into()
            } else {
                "DISABLED".into()
            },
            description: "Accelerate transformer attention matrix computation on supported GPUs"
                .into(),
        },
        FeatureItem {
            name: "Prefix KV-Cache Reuse",
            status: if app.config.llm.prefix_cache {
                "ENABLED".into()
            } else {
                "DISABLED".into()
            },
            description: "Cache system prompt and previous conversation turns across utterances"
                .into(),
        },
        FeatureItem {
            name: "Local HTTP API Server",
            status: if app.config.http.enabled {
                "ENABLED".into()
            } else {
                "DISABLED".into()
            },
            description: "Localhost HTTP REST & SSE streaming server on 127.0.0.1:8765".into(),
        },
        FeatureItem {
            name: "Desktop Quickshell HUD",
            status: if app.config.ui.enabled {
                "ENABLED".into()
            } else {
                "DISABLED".into()
            },
            description: "On-screen floating microphone waveform and transcription HUD overlay"
                .into(),
        },
        FeatureItem {
            name: "Desktop Clock Voice Tool",
            status: if app.config.clock.enabled {
                "ENABLED".into()
            } else {
                "DISABLED".into()
            },
            description: "Instant deterministic fast-path response for 'what time is it?'".into(),
        },
        FeatureItem {
            name: "Location",
            status: if app.config.location.enabled {
                "ENABLED"
            } else {
                "DISABLED"
            }
            .into(),
            description: format!(
                "On-demand approximate IP city · l Edit city override · {}",
                if app.config.location.city.is_empty() {
                    "Automatic"
                } else {
                    &app.config.location.city
                }
            ),
        },
    ];

    let list_items: Vec<ListItem> = items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            let is_sel = i == app.selected_feature_idx;
            let cursor = if is_sel { "> " } else { "  " };
            let is_on = item.status == "ENABLED" || item.status == "push-to-talk";
            let status_style = if is_on { t.success() } else { t.muted_text() };
            let row_style = if is_sel { t.selected() } else { t.base() };

            if is_narrow {
                let lines = vec![
                    Line::from(vec![
                        Span::styled(cursor, row_style),
                        Span::styled(format!("{:<24}", item.name), row_style),
                        Span::styled(format!("[ {:^10} ]", item.status), status_style),
                    ]),
                    Line::from(vec![
                        Span::raw("    "),
                        Span::styled(item.description, t.muted_text()),
                    ]),
                ];
                ListItem::new(lines)
            } else {
                let lines = vec![Line::from(vec![
                    Span::styled(cursor, row_style),
                    Span::styled(format!("{:<26}", item.name), row_style),
                    Span::styled(format!("[ {:^11} ]", item.status), status_style),
                    Span::styled(format!("  — {}", item.description), t.muted_text()),
                ])];
                ListItem::new(lines)
            }
        })
        .collect();

    let title = if area.width >= 90 {
        " Feature Flags & Integrations — [j/k] Navigate · [Enter/Space] Toggle / Cycle · [←/→] Adjust "
    } else {
        " Features — [Enter] Toggle · [←/→] Adjust "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.base())
        .title(title);

    super::list(
        f,
        app,
        List::new(list_items).block(block),
        area,
        app.selected_feature_idx,
        3,
    );
}
