//! Aurora layout: identity header, screen navigation, the screen's panes, footer, overlays.

mod screens;

use crate::app::{App, EditTarget, Tab};
use crate::input::{APPROVAL_PANE, SCREENS};
use crate::navigation::Intent;
use crate::theme::Theme;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const MIN: (u16, u16) = (40, 14);

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.navigation.begin(frame.area());
    let area = frame.area();
    let theme = app.theme.clone();
    frame.render_widget(Block::default().style(theme.base()), area);
    if area.width < MIN.0 || area.height < MIN.1 {
        frame.render_widget(
            Paragraph::new(format!("assistant · needs {}×{}\nResize the terminal; Ctrl+C quits", MIN.0, MIN.1))
                .style(theme.base())
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let chrome = chrome(&theme);
    let shell = chrome.shell(frame, area);
    let footer_h = if shell.height >= 26 { 4 } else { 3 };
    let [title, tabs, body, footer] = Layout::vertical([
        Constraint::Length(crate::aurora::title_height(shell.height)),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(footer_h),
    ])
    .areas(shell);
    let (state, color) = state(app, &theme);
    chrome.title(frame, title, "assistant", "Voice assistant settings", &state, Style::default().fg(color));
    app.navigation.bar(frame, tabs, SCREENS, app.screen(), app.pane_names(), app.focused_pane(), app.editing(), app.skin());
    let body = crate::aurora::body(body);
    screens::draw(frame, app, body);
    let footer = chrome.footer(frame, footer);
    draw_footer(frame, app, &theme, footer);

    // Overlays, bottom to top.
    if app.approval().is_some() && !app.editing() {
        draw_approval(frame, app, &theme, shell);
    }
    if app.editing() {
        draw_edit(frame, app, &theme, shell);
    }
    if app.quit_confirm {
        let rect = centered(shell, 58, 7);
        dialog(frame, &theme, rect, " Quit ", &["The draft has changes that aren't applied.", "Discard them and quit?"]);
        app.navigation.dialog_buttons(frame, rect, app.skin());
    }
    if app.help {
        draw_help(frame, app, &theme, shell);
    }
    let actions = app.command_actions();
    app.commands.draw(frame, &actions, theme.bg, theme.fg, theme.accent, theme.bg_raised);
    let entries = app.navigation_entries();
    app.navigation.draw(frame, &entries, app.skin());
}

fn chrome(theme: &Theme) -> crate::aurora::Chrome {
    crate::aurora::Chrome {
        base: theme.base(),
        muted: Style::default().fg(theme.muted),
        accent: Style::default().fg(theme.accent),
        data: Style::default().fg(theme.cyan),
        border: Style::default().fg(theme.border),
        deep: theme.status(),
    }
}

/// The daemon's state as a symbol and a word.
pub fn state(app: &App, t: &Theme) -> (String, ratatui::style::Color) {
    match &app.daemon_status {
        Some(s) if s.muted => ("● muted".into(), t.yellow),
        Some(s) => match s.state.as_str() {
            "listening" => ("● listening".into(), t.accent),
            "thinking" | "transcribing" | "executing" => (format!("● {}", s.state), t.yellow),
            "speaking" => ("● speaking".into(), t.cyan),
            "error" => ("● error".into(), t.red),
            _ => ("● online".into(), t.green),
        },
        None if app.service_active => ("● starting".into(), t.yellow),
        None => ("○ stopped".into(), t.red),
    }
}

fn footer_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    // A dialog's own keys while it's open.
    if app.editing() {
        return vec![("Enter", "save"), ("Esc", "cancel"), ("Ctrl+U", "clear")];
    }
    if app.quit_confirm {
        return vec![("Enter", "discard and quit"), ("Esc", "cancel")];
    }
    if app.help {
        return vec![("Esc", "close"), ("PgUp/PgDn", "scroll")];
    }
    if let Some((_, p)) = app.approval() {
        let mut v = vec![("y", "allow once")];
        if p["task_scope_allowed"] == true {
            v.push(("t", "allow for task"));
        }
        v.extend([("n", "deny"), ("Esc", "later"), ("PgUp/PgDn", "scroll")]);
        return v;
    }
    let pane = app.focused_pane();
    let mut v: Vec<(&str, &str)> = match (app.active_tab, pane) {
        (Tab::Dashboard, _) => {
            vec![(("s"), if app.service_active { "stop" } else { "start" }), ("m", "mute"), ("t", "voice preview")]
        }
        (Tab::Models, 0) => vec![("Enter", "use model"), ("↑/↓", "select")],
        (Tab::Models, 1) => vec![("v", "voice"), ("+/-", "speed"), ("x", "model size"), ("t", "preview")],
        (Tab::Models, _) => vec![("←/→", "sensitivity"), ("b", "detector")],
        (Tab::Downloader, 1) => vec![("Enter", "list files"), ("/", "search")],
        (Tab::Downloader, _) => vec![("Enter", "download"), ("/", "search"), ("u", "URL")],
        (Tab::Persona, 1) => vec![("Enter", "edit"), ("a", "add"), ("d", "delete"), ("g", "generate")],
        (Tab::Persona, _) => vec![("Enter", "use"), ("n", "new"), ("e", "rename"), ("w", "wake word")],
        (Tab::Features, _) => vec![("Enter", "toggle"), ("←/→", "adjust"), ("l", "city")],
        (Tab::Diagnostics, _) => vec![("r", "run"), ("PgUp/PgDn", "scroll")],
        (Tab::Activity, 0) => vec![("Enter", "details"), ("c", "clear")],
        (Tab::Activity, _) => vec![("PgUp/PgDn", "scroll"), ("Esc", "turns")],
        (Tab::Calibration, _) => vec![("Enter", "start"), ("a", "use suggestions")],
        (Tab::Commands, 2) => vec![("PgUp/PgDn", "scroll"), ("p", "preview")],
        (Tab::Commands, _) => vec![("Enter", "edit"), ("n", "new"), ("d", "delete"), ("p", "preview")],
        (Tab::Plugins, _) => vec![("Space", "toggle"), ("e", "settings"), ("r", "retry")],
    };
    v.insert(1.min(v.len()), ("Ctrl+K", "actions"));
    if app.cancellable_now() {
        v.insert(0, ("Ctrl+X", "cancel"));
    }
    if app.is_dirty {
        v.push(("Ctrl+S", "apply"));
    }
    v.push(("?", "help"));
    v.push(("q", "quit"));
    v
}

impl App {
    pub fn cancellable_now(&self) -> bool {
        self.download_cancel_flag.is_some() || (self.operation_busy && self.active_tab == Tab::Calibration)
    }
}

fn draw_footer(frame: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    if area.is_empty() {
        return;
    }
    let hints = footer_hints(app);
    let keys = crate::navigation::shortcuts(&hints, area.width, Style::default().fg(t.accent), Style::default().fg(t.muted));
    frame.render_widget(Paragraph::new(keys).style(t.status()), Rect::new(area.x, area.y, area.width, 1));
    if area.height < 2 {
        return;
    }
    let line = Rect::new(area.x, area.y + 1, area.width, 1);
    let dl = app.download_progress_rx.borrow().clone();
    if dl.is_downloading {
        let ratio = if dl.total_bytes > 0 { dl.downloaded_bytes as f64 / dl.total_bytes as f64 } else { 0.0 };
        let label = format!(
            "{}  {} / {}  {}/s",
            truncate(&dl.filename, 28),
            crate::models::format_bytes(dl.downloaded_bytes),
            crate::models::format_bytes(dl.total_bytes),
            crate::models::format_bytes(dl.speed_bytes_sec)
        );
        frame.render_widget(
            Gauge::default().ratio(ratio.clamp(0.0, 1.0)).label(label).gauge_style(Style::default().fg(t.accent).bg(t.bg_raised)),
            line,
        );
        return;
    }
    let status: Line = if let Some((toast, _)) = &app.toast {
        Line::from(Span::styled(truncate(toast, area.width as usize), Style::default().fg(t.fg)))
    } else if app.approval().is_none() && app.daemon_status.as_ref().is_some_and(|s| !s.pending_approvals.is_empty()) {
        Line::from(vec![
            Span::styled("▲ An approval is waiting  ", Style::default().fg(t.yellow)),
            Span::styled("(y)", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
            Span::styled(" review", Style::default().fg(t.muted)),
        ])
    } else if app.operation_busy {
        Line::from(Span::styled("Working…", Style::default().fg(t.yellow)))
    } else if app.is_dirty {
        Line::from(vec![
            Span::styled("▲ Unsaved draft  ", Style::default().fg(t.yellow)),
            Span::styled("(Ctrl+S)", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
            Span::styled(" apply  ", Style::default().fg(t.muted)),
            Span::styled("(Ctrl+R)", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
            Span::styled(" apply and restart", Style::default().fg(t.muted)),
        ])
    } else {
        let restart = app.daemon_status.as_ref().map(|s| s.restart_required.join(", ")).unwrap_or_default();
        if restart.is_empty() {
            Line::from(Span::styled("Saved", Style::default().fg(t.muted)))
        } else {
            Line::from(Span::styled(format!("Restart needed for {restart}  (Ctrl+R)"), Style::default().fg(t.yellow)))
        }
    };
    frame.render_widget(Paragraph::new(status).style(t.status()), line);
}

// ---- Shared pieces for screens ---------------------------------------------------------

/// A pane frame: rounded, the accent and "▸ name" when focused. Returns the padded inside.
pub fn pane(frame: &mut Frame, app: &App, area: Rect, index: usize, name: &str) -> Rect {
    app.navigation.outline(frame, area, index, app.focused_pane(), name, app.skin());
    let inner = Block::bordered().inner(area);
    // A row of air under the title when there's height to spare.
    let top = u16::from(inner.height > 8);
    if inner.width > 4 {
        Rect::new(inner.x + 1, inner.y + top, inner.width - 2, inner.height - top)
    } else {
        inner
    }
}

/// A list in a pane, the selection kept in view, each row a mouse target. `selected` None
/// draws no highlight (a list whose pane isn't the one being browsed). Empty lists say why.
pub fn list(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    index: usize,
    items: Vec<ListItem<'static>>,
    selected: Option<usize>,
    empty: Line<'static>,
) {
    if area.is_empty() {
        return;
    }
    if items.is_empty() {
        frame.render_widget(Paragraph::new(empty).wrap(Wrap { trim: true }), area);
        return;
    }
    let count = items.len();
    let mut state = ListState::default().with_selected(selected.map(|s| s.min(count - 1)));
    let t = &app.theme;
    frame.render_stateful_widget(
        List::new(items)
            .style(t.base())
            .highlight_symbol("▸ ")
            .highlight_style(Style::default().bg(t.bg_raised).fg(t.accent))
            .highlight_spacing(ratatui::widgets::HighlightSpacing::Always),
        area,
        &mut state,
    );
    app.navigation.rows(area, index, state.offset(), 1, count.saturating_sub(state.offset()));
}

/// Wrapped, scrollable text in a pane; the wheel scrolls it. Clamps the stored offset.
pub fn text(frame: &mut Frame, app: &mut App, area: Rect, index: usize, lines: Vec<Line<'static>>) {
    if area.is_empty() {
        return;
    }
    let height = wrapped_height(&lines, area.width);
    let screen = app.screen();
    let max = height.saturating_sub(area.height);
    app.navigation.scroll[screen] = app.navigation.scroll[screen].min(max);
    let scroll = app.navigation.scroll[screen];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll, 0)), area);
    app.navigation.details(area, index);
    if max > 0 {
        let t = &app.theme;
        let mark = format!("{}/{}", scroll + 1, max + 1);
        let w = mark.width() as u16;
        if area.width > w + 2 {
            frame.render_widget(
                Paragraph::new(mark).style(Style::default().fg(t.muted)).alignment(Alignment::Right),
                Rect::new(area.right() - w, area.bottom().saturating_sub(1).max(area.y), w, 1),
            );
        }
    }
}

fn wrapped_height(lines: &[Line], width: u16) -> u16 {
    let width = width.max(1) as usize;
    lines.iter().map(|l| (l.width().max(1)).div_ceil(width) as u16).sum()
}

/// "label  value" with the label muted.
pub fn kv(t: &Theme, label: &str, value: impl Into<String>, style: Style) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{label:<13}"), Style::default().fg(t.muted)), Span::styled(value.into(), style)])
}

/// A key and what it does, in the shared style.
pub fn key(t: &Theme, k: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("({k})"), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {label}  "), Style::default().fg(t.muted)),
    ]
}

/// A section heading in a pane.
pub fn heading(t: &Theme, text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), Style::default().fg(t.fg).add_modifier(Modifier::BOLD)))
}

/// "● on" or "○ off".
pub fn on_off(t: &Theme, on: bool) -> Span<'static> {
    if on {
        Span::styled("● on", Style::default().fg(t.green))
    } else {
        Span::styled("○ off", Style::default().fg(t.muted))
    }
}

pub fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut used = 0;
    let mut out = String::new();
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// The end of `s` that fits `width` columns (a text field showing its cursor end).
fn tail(s: &str, width: usize) -> String {
    let mut used = 0;
    let mut chars = Vec::new();
    for c in s.chars().rev() {
        let w = c.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        chars.push(c);
        used += w;
    }
    chars.into_iter().rev().collect()
}

pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

/// A cleared, titled dialog with body lines; returns the inside.
fn dialog(frame: &mut Frame, t: &Theme, rect: Rect, title: &str, body: &[&str]) -> Rect {
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(title.to_string(), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)))
        .border_style(Style::default().fg(t.accent))
        .style(t.base());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let inner = Rect::new(inner.x + 1.min(inner.width), inner.y, inner.width.saturating_sub(2), inner.height);
    let lines: Vec<Line> = body.iter().map(|l| Line::from(l.to_string())).collect();
    frame
        .render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), Rect { height: inner.height.saturating_sub(2), ..inner });
    inner
}

fn edit_title(target: EditTarget, commands: bool) -> (&'static str, &'static str) {
    match target {
        EditTarget::PersonaCreateName => (" New persona ", "Name the persona; wake aliases are generated from it."),
        EditTarget::PersonaRename => (" Rename persona ", "The new name for the selected persona."),
        EditTarget::AliasAdd => (" Add wake alias ", "A phrase that wakes the assistant, as it sounds."),
        EditTarget::AliasEdit => (" Edit wake alias ", "Change the selected wake phrase."),
        EditTarget::HfSearchQuery => (" Search Hugging Face ", "Words to search GGUF repositories for."),
        EditTarget::CustomModelUrl => (" Download from URL ", "A Hugging Face …/resolve/… link to a single .gguf file."),
        EditTarget::RouteEntry if commands => {
            (" Edit command ", "JSON: phrases plus argument arrays. It's checked before it's kept.")
        }
        EditTarget::RouteEntry => (" Edit app ", "JSON: phrases plus argument arrays. It's checked before it's kept."),
        EditTarget::RouteNew if commands => {
            (" New command ", "JSON: phrases plus argument arrays. It's checked before it's kept.")
        }
        EditTarget::RouteNew => (" New app ", "JSON: phrases plus argument arrays. It's checked before it's kept."),
        EditTarget::RoutePreview => (" Preview routing ", "A phrase to route. Nothing runs; the match shows in Details."),
        EditTarget::LocationCity => (" City ", "Your city for weather and local search; empty uses your IP's city."),
        EditTarget::PluginSettings => (" Plugin settings ", "JSON settings for the selected plugin."),
        EditTarget::None => (" Edit ", ""),
    }
}

fn draw_edit(frame: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let (title, hint) = edit_title(app.edit_target, app.commands_pane);
    let rect = centered(area, 76, 9);
    let inner = dialog(frame, t, rect, title, &[hint]);
    if inner.height < 4 {
        return;
    }
    let field = Rect::new(inner.x, inner.y + inner.height.saturating_sub(3).min(2), inner.width, 1);
    let shown = tail(&app.edit_buffer, inner.width.saturating_sub(3) as usize);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::default().fg(t.accent)),
            Span::styled(shown, Style::default().fg(t.fg)),
            Span::styled("█", Style::default().fg(t.accent)),
        ]))
        .style(Style::default().bg(t.bg_raised)),
        field,
    );
    frame.render_widget(
        Paragraph::new(crate::navigation::shortcuts(
            &[("Enter", "save"), ("Esc", "cancel"), ("Ctrl+U", "clear")],
            inner.width,
            Style::default().fg(t.accent),
            Style::default().fg(t.muted),
        )),
        Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
    );
    // The dialog owns the mouse while it's open.
    app.navigation.clear_hits();
}

fn draw_approval(frame: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let Some((_, pending)) = app.approval().map(|(i, p)| (i.clone(), p.clone())) else { return };
    let rect = centered(area, 80, 20);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(" Approval needed ", Style::default().fg(t.yellow).add_modifier(Modifier::BOLD)))
        .border_style(Style::default().fg(t.yellow))
        .style(t.base());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let inner = Rect::new(inner.x + 1.min(inner.width), inner.y, inner.width.saturating_sub(2), inner.height);
    if inner.height < 5 {
        return;
    }
    app.navigation.clear_hits();
    let tool = pending["tool"].as_str().unwrap_or("?");
    let summary = pending["summary"].as_str().filter(|s| !s.is_empty()).unwrap_or(tool);
    // Approvals last 60 s; anything else isn't a real deadline.
    let expiry = pending["expires_at_ms"]
        .as_u64()
        .map(|e| e.saturating_sub(chrono::Utc::now().timestamp_millis() as u64) / 1000)
        .filter(|s| *s <= 600)
        .map(|s| format!(" · expires in {s} s"))
        .unwrap_or_default();
    let mut head = vec![
        Line::from(Span::styled(summary.to_string(), Style::default().fg(t.fg).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(format!("{tool}{expiry}"), Style::default().fg(t.muted))),
    ];
    if pending["voice_allowed"] == false {
        head.push(Line::from(Span::styled(
            "Saying “approve” doesn't count for this one; choose here.",
            Style::default().fg(t.yellow),
        )));
    }
    let head_h = head.len() as u16;
    frame.render_widget(Paragraph::new(head).wrap(Wrap { trim: true }), Rect { height: head_h.min(inner.height), ..inner });
    // Arguments, scrollable.
    let args = Rect::new(inner.x, inner.y + head_h + 1, inner.width, inner.height.saturating_sub(head_h + 3));
    if !args.is_empty() {
        let json = serde_json::to_string_pretty(&pending["arguments"]).unwrap_or_default();
        let lines: Vec<Line> =
            json.lines().map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(t.cyan)))).collect();
        let max = wrapped_height(&lines, args.width).saturating_sub(args.height);
        app.approval_scroll = app.approval_scroll.min(max);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((app.approval_scroll, 0))
                .style(Style::default().bg(t.bg_raised)),
            args,
        );
        app.navigation.hit(args, Intent::Details(APPROVAL_PANE));
    }
    // Buttons, each a mouse target.
    let y = inner.bottom() - 1;
    let mut x = inner.x;
    let mut buttons = vec![("y", "allow once", "Allow once")];
    if pending["task_scope_allowed"] == true {
        buttons.push(("t", "allow for this task", "Allow for task"));
    }
    buttons.push(("n", "deny", "Deny"));
    buttons.push(("Esc", "later", "Later"));
    for (k, label, id) in buttons {
        let text = format!("({k}) {label}");
        let w = text.width() as u16;
        if x + w > inner.right() {
            break;
        }
        frame.render_widget(Paragraph::new(Line::from(key(t, k, label))), Rect::new(x, y, w + 2, 1));
        app.navigation.hit(Rect::new(x, y, w, 1), Intent::Command(id));
        x += w + 2;
    }
}

fn draw_help(frame: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let rect = centered(area, 76, 30);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(" Help · (Esc) close ", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)))
        .border_style(Style::default().fg(t.accent))
        .style(t.base());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let inner = Rect::new(inner.x + 1.min(inner.width), inner.y, inner.width.saturating_sub(2), inner.height);
    let rows: &[(&str, &str)] = &[
        ("", "Everywhere"),
        ("Tab / Shift+Tab", "next / previous screen"),
        ("1–9, 0", "go to a screen"),
        ("F6 / Shift+F6", "next / previous pane"),
        ("F2", "Navigate: screens and panes"),
        ("Ctrl+K", "actions for here, searchable"),
        ("↑/↓  j/k", "move in the focused list"),
        ("PgUp / PgDn", "page the list, or scroll details"),
        ("Enter", "use, edit or open the selection"),
        ("Space", "toggle a feature or plugin"),
        ("Esc", "close the top dialog, or leave details"),
        ("Ctrl+X", "cancel a download or calibration"),
        ("q / Ctrl+C", "quit (asks first when the draft has changes)"),
        ("", "The draft"),
        ("Ctrl+S", "apply: save and reload the assistant"),
        ("Ctrl+R", "apply and restart (model, devices)"),
        ("", "The assistant"),
        ("s", "start or stop it (Overview)"),
        ("m", "mute or unmute the microphone"),
        ("t", "voice preview"),
        ("y", "review a waiting approval"),
        ("Ctrl+O", "conversation on or off"),
        ("Ctrl+D / Ctrl+P", "dictation on or off / pause or resume"),
        ("Ctrl+G / Ctrl+B", "browser takeover / resume"),
        ("Ctrl+T", "tool mode"),
        ("", "Approvals"),
        ("y / t / n", "allow once / for the task / deny"),
        ("Esc", "put it aside; it keeps waiting"),
        ("", "Themes (Ctrl+K)"),
        ("Alt+A / Alt+O / Alt+T", "Aurora / Follow Omarchy / Terminal"),
        ("", "Options"),
        ("--no-mouse", "leave the mouse to the terminal (select text)"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            if k.is_empty() {
                Line::from(Span::styled(v.to_string(), Style::default().fg(t.fg).add_modifier(Modifier::BOLD)))
            } else {
                Line::from(vec![
                    Span::styled(format!("  {k:<22}"), Style::default().fg(t.accent)),
                    Span::styled(v.to_string(), Style::default().fg(t.muted)),
                ])
            }
        })
        .collect();
    let max = wrapped_height(&lines, inner.width).saturating_sub(inner.height);
    app.help_scroll = app.help_scroll.min(max);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((app.help_scroll, 0)), inner);
    app.navigation.clear_hits();
}

#[cfg(test)]
mod tests;
