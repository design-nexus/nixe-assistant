pub mod dashboard;
pub mod downloader_tab;
pub mod features_tab;
pub mod models_tab;
pub mod persona_tab;
use crate::app::{App, EditTarget, Tab};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(5),
    ])
    .split(f.area());
    let title = format!(
        "{} · {} · {}{}",
        app.config.assistant.name,
        app.active_tab.title(),
        if app.is_dirty {
            "UNSAVED draft"
        } else {
            "Saved"
        },
        if app.operation_busy {
            " · Working…"
        } else {
            ""
        }
    );
    let navigation = if chunks[0].width >= 150 {
        Tab::ALL
            .iter()
            .map(|tab| tab.title())
            .collect::<Vec<_>>()
            .join("  ")
    } else {
        format!("Tab/Shift+Tab or 1–9/0 · {}", app.active_tab.title())
    };
    f.render_widget(
        Paragraph::new(navigation)
            .style(app.theme.tab_active())
            .block(Block::default().borders(Borders::ALL).title(title)),
        chunks[0],
    );
    match app.active_tab {
        Tab::Dashboard=>dashboard::render(f,app,chunks[1]),Tab::Models=>models_tab::render(f,app,chunks[1]),Tab::Downloader=>downloader_tab::render(f,app,chunks[1]),Tab::Persona=>persona_tab::render(f,app,chunks[1]),Tab::Features=>features_tab::render(f,app,chunks[1]),
        Tab::Diagnostics=>text(f,app,chunks[1],"Diagnostics · Enter/r Refresh",&serde_json::to_string_pretty(&app.diagnostics).unwrap_or_default()),
        Tab::Activity=>{let text=app.activity["turns"].as_array().map(|turns|turns.iter().rev().map(|turn|serde_json::to_string_pretty(turn).unwrap_or_default()).collect::<Vec<_>>().join("\n\n")).unwrap_or_default();self::text(f,app,chunks[1],"Activity · latest 100 turns · c Clear · Ctrl+T Tool mode · PgUp/PgDn Scroll",&text);},
        Tab::Calibration=>text(f,app,chunks[1],"Audio Calibration · Enter Start · a Copy suggestions to draft · c Cancel",&format!("Stay quiet for five seconds, then speak normally for five seconds. Watch the phase prompt below.\n\n{}",serde_json::to_string_pretty(&app.calibration).unwrap_or_default())),
        Tab::Plugins=>plugins(f,app,chunks[1]),
        Tab::Commands=>commands(f,app,chunks[1]),
    }
    let restart = app
        .daemon_status
        .as_ref()
        .map(|s| s.restart_required.join(", "))
        .unwrap_or_else(|| "daemon offline".into());
    let status = app
        .toast
        .as_ref()
        .map(|(s, _)| s.clone())
        .unwrap_or_else(|| {
            format!(
                "Restart required: {}",
                if restart.is_empty() { "none" } else { &restart }
            )
        });
    let session = app
        .daemon_status
        .as_ref()
        .map(|s| {
            format!(
                "{} · {}s · {} · Draft: {}",
                s.session["mode"].as_str().unwrap_or("wake"),
                s.session["remaining_secs"],
                s.session["target"]["title"].as_str().unwrap_or(""),
                s.session["draft"].as_str().unwrap_or("")
            )
        })
        .unwrap_or_default();
    f.render_widget(Paragraph::new(vec![Line::from(session),Line::from("Ctrl+O Conversation · Ctrl+D Dictation · Ctrl+P Pause/resume · Ctrl+G/B Browser takeover/resume"),Line::from("Ctrl+S Apply · Ctrl+R Apply & Restart · t Voice Preview · Ctrl+T Tool mode · q Quit"),Line::from(status)]).style(app.theme.tab_inactive()).block(Block::default().borders(Borders::ALL)),chunks[2]);
    if app.edit_target != EditTarget::None {
        let area = popup(f.area());
        f.render_widget(Clear, area);
        let hint = match app.edit_target {
            EditTarget::RouteEntry | EditTarget::RouteNew => {
                "JSON entry: phrases + argument arrays. Ctrl+U clear, Enter validate, Esc cancel"
            }
            _ => "Enter confirm · Esc cancel · Ctrl+U clear",
        };
        let value = format!("{}█", app.edit_buffer);
        let available = area.width.saturating_sub(4) as usize;
        let displayed = tail_width(&value, available);
        f.render_widget(
            Paragraph::new(format!("{hint}\n\n{displayed}"))
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!("{:?}", app.edit_target)),
                ),
            area,
        );
    } else if let Some((id, pending)) = app
        .daemon_status
        .as_ref()
        .and_then(|s| s.pending_approvals.iter().next())
    {
        let area = popup(f.area());
        f.render_widget(Clear, area);
        let content=format!("{}\n{}\n\nApproval {} · y Allow once / n Deny · expires after 60 seconds\nPgUp/PgDn to read all arguments",pending["tool"],serde_json::to_string_pretty(&pending["arguments"]).unwrap_or_default(),id);
        f.render_widget(
            Paragraph::new(content)
                .wrap(Wrap { trim: false })
                .scroll((app.scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Tool approval — exact arguments"),
                ),
            area,
        );
    }
}
fn popup(r: Rect) -> Rect {
    let width = r.width.saturating_sub(4).max(1);
    let height = r.height.saturating_sub(6).clamp(1, 20);
    Rect::new(
        r.x + (r.width - width) / 2,
        r.y + (r.height - height) / 2,
        width,
        height,
    )
}
pub fn truncate(s: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let mut used = 0;
    let mut out = String::new();
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width {
            out.push('…');
            break;
        }
        out.push(c);
        used += w;
    }
    out
}
fn tail_width(s: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
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
pub fn list(f: &mut Frame, app: &App, widget: List<'_>, area: Rect, selected: usize, key: usize) {
    let mut states = app.list_states.borrow_mut();
    states[key].select(Some(selected));
    f.render_stateful_widget(widget, area, &mut states[key]);
}
fn text(f: &mut Frame, app: &App, area: Rect, title: &str, value: &str) {
    f.render_widget(
        Paragraph::new(value)
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0))
            .style(app.theme.base())
            .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}
fn commands(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);
    let entries = app.route_entries();
    let items = entries
        .iter()
        .enumerate()
        .map(|(i, v)| ListItem::new(format!("{} {}", i + 1, v["phrases"])))
        .collect::<Vec<_>>();
    list(
        f,
        app,
        List::new(items)
            .highlight_style(app.theme.selected())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(if app.commands_pane {
                        "Commands · ←/→ Apps · e Edit · n New · d Delete · p Preview"
                    } else {
                        "Apps · ←/→ Commands · e Edit · n New · d Delete · p Preview"
                    }),
            ),
        chunks[0],
        app.selected_route,
        7,
    );
    let selected = entries.get(app.selected_route).cloned().unwrap_or_default();
    let tool = selected["tool"].as_str().unwrap_or("app.launch");
    let permissions = &app.config.permissions;
    let policy = if [
        "audio.volume_up",
        "audio.volume_down",
        "audio.mute",
        "audio.output_switch",
    ]
    .contains(&tool)
    {
        permissions.volume.as_str()
    } else if tool == "app.launch" {
        permissions.launch_app.as_str()
    } else if tool == "system.shutdown" {
        permissions.shutdown.as_str()
    } else if assistant_tools::builtin_tools()
        .iter()
        .any(|t| t.name == tool)
    {
        "allow"
    } else {
        "deny"
    };
    text(
        f,
        app,
        chunks[1],
        "Arguments & permission · routing preview executes nothing",
        &format!(
            "Permission: {policy}\n{}\n\n{}",
            serde_json::to_string_pretty(&selected).unwrap_or_default(),
            app.route_preview
        ),
    );
}

fn plugins(f: &mut Frame, app: &App, area: Rect) {
    let chunks =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).split(area);
    let installed = app
        .daemon_status
        .as_ref()
        .map(|s| s.plugins.clone())
        .unwrap_or_else(|| assistant_tools::plugins::PluginManager::new().inspect());
    let entries = installed.as_array().cloned().unwrap_or_default();
    let rows = app
        .plugin_draft
        .iter()
        .map(|(id, s)| {
            let status = entries.iter().find(|p| p["id"] == id.as_str());
            ListItem::new(format!(
                "{} {} · {}",
                if s.enabled { "[x]" } else { "[ ]" },
                id,
                status
                    .and_then(|s| s["health"].as_str())
                    .unwrap_or("offline")
            ))
        })
        .collect::<Vec<_>>();
    list(
        f,
        app,
        List::new(rows).highlight_style(app.theme.selected()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Plugins · draft"),
        ),
        chunks[0],
        app.selected_plugin,
        8,
    );
    let detail=app.plugin_draft.iter().nth(app.selected_plugin).map(|(id,draft)|{
        let status=entries.iter().find(|p|p["id"]==id.as_str()).cloned().unwrap_or_default();
        let names=status["manifest"]["tools"].as_array().cloned().unwrap_or_default();
        let permissions=app.daemon_status.as_ref().and_then(|s|s.mcp_tools.as_array()).map(|tools|tools.iter().filter(|t|names.contains(&t["name"])).map(|t|format!("{}: {}",t["name"],t["permission"])).collect::<Vec<_>>().join("\n")).unwrap_or_default();
        let m=&status["manifest"];
        format!("{} · version {} · {}\nSaved enabled: {} · Running enabled: {}\nDraft enabled: {}\n\nSpace: toggle draft · e: settings JSON · r: retry\nCtrl+S: Apply · q twice: discard and quit\n\nDependencies: {}\nOptional dependencies: {}\nRecent error: {}\n\nPermissions:\n{permissions}\n\nDraft settings: {}\nSettings schema: {}\n\nStartup: {} {}\nSource: {}\nData: {}",id,m["version"].as_str().unwrap_or("?"),status["health"].as_str().unwrap_or("offline"),status["saved"]["enabled"],status["running_enabled"],draft.enabled,m["dependencies"],m["optional_dependencies"],status["error"],draft.settings,m["settings_schema"],m["executable"],m["args"],status["saved"]["source"],status["data_directory"])

    }).unwrap_or_else(||"No plugins installed. Use plugins migrate or plugins install.".into());
    text(
        f,
        app,
        chunks[1],
        "Version, health, dependencies, tools and recent errors",
        &detail,
    );
}
