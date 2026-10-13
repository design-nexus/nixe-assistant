//! Tab 4: Persona & Wake word settings rendering with saved personas and alias editor.

use crate::app::{App, PersonaPane};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    if area.width >= 100 {
        render_wide(f, app, area);
    } else {
        render_narrow(f, app, area);
    }
}

fn render_wide(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(36), Constraint::Min(40)])
        .split(area);

    render_personas_list(f, app, chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Summary & Mode
            Constraint::Min(8),    // Aliases Editor
            Constraint::Length(6), // Prompt Preview
        ])
        .split(chunks[1]);

    render_persona_summary(f, app, right_chunks[0]);
    render_aliases_editor(f, app, right_chunks[1]);
    render_prompt_preview(f, app, right_chunks[2]);
}

fn render_narrow(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7), // Compact Personas List
            Constraint::Min(9),    // Aliases Editor
            Constraint::Length(6), // Prompt & Info
        ])
        .split(area);

    render_personas_list(f, app, chunks[0]);
    render_aliases_editor(f, app, chunks[1]);
    render_prompt_preview(f, app, chunks[2]);
}

fn render_personas_list(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let is_focused = app.persona_pane == PersonaPane::Personas;
    let border_style = if is_focused {
        t.focus_border()
    } else {
        t.base()
    };

    let mut lines = Vec::new();
    for (i, p) in app.personas.iter().enumerate() {
        let is_selected = i == app.selected_persona_idx;
        let is_active = p.id == app.config.assistant.persona_id;

        let active_badge = if is_active {
            Span::styled("[ACTIVE] ", t.success())
        } else {
            Span::styled("[      ] ", t.muted_text())
        };

        let cursor = if is_selected { "▸ " } else { "  " };

        let name_style = if is_selected {
            t.selected()
        } else if is_active {
            t.accent_text()
        } else {
            t.card()
        };

        let voice_tag = Span::styled(format!(" {} ", p.voice), t.muted_text());
        let count_tag = Span::styled(format!("({}a)", p.aliases.len()), t.muted_text());

        lines.push(Line::from(vec![
            Span::styled(
                cursor,
                if is_selected {
                    t.accent_text()
                } else {
                    t.muted_text()
                },
            ),
            active_badge,
            Span::styled(format!("{:<8} ", p.name), name_style),
            voice_tag,
            Span::raw(" "),
            count_tag,
        ]));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No personas found.",
            t.muted_text(),
        )));
    }

    let title = if area.width >= 35 {
        " Saved Personas [Enter:Use, n:New, d:Del] "
    } else {
        " Personas "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    super::list(
        f,
        app,
        List::new(lines.into_iter().map(ListItem::new).collect::<Vec<_>>()).block(block),
        area,
        app.selected_persona_idx,
        4,
    );
}

fn render_persona_summary(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let p = app.selected_persona();
    let is_narrow = area.width < 80;

    let (name, voice, is_active) = match p {
        Some(p) => (
            p.name.as_str(),
            p.voice.as_str(),
            p.id == app.config.assistant.persona_id,
        ),
        None => ("None", "af_heart", false),
    };

    let status_span = if is_active {
        Span::styled("ACTIVE draft persona", t.success())
    } else {
        Span::styled("INACTIVE (Press Enter in list to activate)", t.muted_text())
    };

    let wake_span = if app.config.wake_word.enabled {
        Span::styled("ENABLED ('w' toggle)", t.success())
    } else {
        Span::styled("DISABLED ('w' toggle)", t.muted_text())
    };

    let lines = if is_narrow {
        vec![
            Line::from(vec![
                Span::styled("Name: ", t.muted_text()),
                Span::styled(name, t.accent_text()),
                Span::raw("  "),
                status_span,
            ]),
            Line::from(vec![
                Span::styled("Voice: ", t.muted_text()),
                Span::styled(format!("[ {voice} ]"), t.card()),
                Span::styled(" ('v' cycle)  ", t.muted_text()),
                Span::styled("Wake: ", t.muted_text()),
                wake_span,
            ]),
            Line::from(vec![
                Span::styled("PTT Bind: ", t.muted_text()),
                Span::styled("Alt + Space (hold to speak, release to send)", t.card()),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled("Selected Persona: ", t.muted_text()),
                Span::styled(name, t.accent_text()),
                Span::raw("   ·   "),
                status_span,
            ]),
            Line::from(vec![
                Span::styled("TTS Voice:        ", t.muted_text()),
                Span::styled(format!("[ {voice} ]"), t.card()),
                Span::styled(
                    "  (Press 'v' to cycle voice)   ·   Wake Detection: ",
                    t.muted_text(),
                ),
                wake_span,
            ]),
            Line::from(vec![
                Span::styled("PTT Hotkey:       ", t.muted_text()),
                Span::styled(
                    "Alt + Space (Hold to talk, release to submit query)",
                    t.card(),
                ),
            ]),
        ]
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.base())
        .title(" Persona Status & Voice ");

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_aliases_editor(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let is_focused = app.persona_pane == PersonaPane::Aliases;
    let border_style = if is_focused {
        t.focus_border()
    } else {
        t.base()
    };
    let p = app.selected_persona();

    let mut lines = Vec::new();

    if let Some(p) = p {
        let prefixes = p.prefixes.join(", ");
        lines.push(Line::from(vec![
            Span::styled("Wake Prefixes: ", t.muted_text()),
            Span::styled(format!("[ {prefixes} ]"), t.card()),
            Span::styled("  + [Alias phrase below]", t.muted_text()),
        ]));
        lines.push(Line::from(""));

        if p.aliases.is_empty() {
            lines.push(Line::from(Span::styled(
                "No aliases configured. Press 'a' to add one or 'g' to auto-generate.",
                t.muted_text(),
            )));
        } else {
            for (i, alias) in p.aliases.iter().enumerate() {
                let is_selected = is_focused && i == app.selected_alias_idx;
                let cursor = if is_selected { "▸ " } else { "  " };
                let num = format!("{:02}: ", i + 1);

                let alias_style = if is_selected {
                    t.selected()
                } else if i == 0 {
                    t.accent_text()
                } else {
                    t.card()
                };

                let tag = if i == 0 {
                    Span::styled(" [Exact]", t.muted_text())
                } else {
                    Span::styled(" [Phonetic]", t.muted_text())
                };

                lines.push(Line::from(vec![
                    Span::styled(
                        cursor,
                        if is_selected {
                            t.accent_text()
                        } else {
                            t.muted_text()
                        },
                    ),
                    Span::styled(num, t.muted_text()),
                    Span::styled(format!("{:<16}", alias), alias_style),
                    tag,
                ]));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No persona selected.",
            t.muted_text(),
        )));
    }

    let title = if area.width >= 60 {
        " Phonetic Wake Aliases [a: Add, d: Delete, e: Edit, g: Auto-Gen, h/l: Switch Pane] "
    } else {
        " Wake Aliases [a:Add, d:Del, e:Edit, g:Gen] "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    super::list(
        f,
        app,
        List::new(lines.into_iter().map(ListItem::new).collect::<Vec<_>>()).block(block),
        area,
        app.selected_alias_idx + 2,
        5,
    );
}

fn render_prompt_preview(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let p = app.selected_persona();

    let prompt = match p {
        Some(p) => {
            if p.prompt.is_empty() {
                app.config.system_prompt()
            } else {
                p.prompt.replace("{name}", &p.name)
            }
        }
        None => app.config.system_prompt(),
    };

    let lines = vec![Line::from(vec![
        Span::styled("Spoken Prompt: ", t.muted_text()),
        Span::styled(prompt, t.card()),
    ])];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.base())
        .title(" Persona System Prompt Preview ");

    f.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        area,
    );
}
