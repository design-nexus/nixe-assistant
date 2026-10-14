mod app;
mod daemon;
mod models;
mod theme;
mod ui;
use app::{App, EditTarget, PersonaPane, Tab};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io::stdout, time::Duration};
struct TtyGuard;
impl Drop for TtyGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout(),
            LeaveAlternateScreen,
            DisableBracketedPaste,
            event::DisableMouseCapture,
            crossterm::cursor::Show
        );
    }
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (theme_name, theme, watch_dir) = match theme::find_omarchy() {
        Some((n, t, p)) => (n, t, Some(p)),
        None => ("Default Dark".into(), theme::default_dark(), None),
    };
    let mut app = App::new(theme)?;
    let (theme_tx, mut theme_rx) = tokio::sync::mpsc::unbounded_channel();
    let _watcher = watch_dir.and_then(|path| {
        theme::spawn_watcher(path.clone(), move || {
            if let Some((_, theme)) = theme::load_from(&path) {
                let _ = theme_tx.send(theme);
            }
        })
    });
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout(),
            LeaveAlternateScreen,
            DisableBracketedPaste,
            crossterm::cursor::Show
        );
        previous_hook(info);
    }));
    enable_raw_mode()?;
    let _guard = TtyGuard;
    execute!(
        stdout(),
        EnterAlternateScreen,
        EnableBracketedPaste,
        event::EnableMouseCapture
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    app.set_toast(format!(
        "Loaded draft · {theme_name} · Ctrl+S Apply / Ctrl+R Apply & Restart"
    ));
    app.poll_daemon();
    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    let mut poll = tokio::time::interval(Duration::from_millis(1500));
    loop {
        terminal.draw(|frame| ui::render(frame, &app))?;
        tokio::select! {
            Some(theme)=theme_rx.recv()=>app.theme=theme,
            Some(update)=app.updates.recv()=>app.update(update),
            _=poll.tick()=>app.poll_daemon(),
            _=ticker.tick()=>{app.clear_expired_toast();app.tick_download();},
            ev=events.next()=>match ev {
                Some(Ok(Event::Key(key))) if key.kind!=KeyEventKind::Release=>if handle_key(&mut app,key){break;},
                Some(Ok(Event::Paste(text))) if app.edit_target!=EditTarget::None=>app.edit_buffer.push_str(&text.replace(['\n','\r']," ")),
                Some(Ok(Event::Mouse(mouse)))=>match mouse.kind{event::MouseEventKind::ScrollDown=>app.scroll=app.scroll.saturating_add(3),event::MouseEventKind::ScrollUp=>app.scroll=app.scroll.saturating_sub(3),_=>{}},
                Some(Err(err))=>return Err(err.into()),None=>break,_=>{},
            }
        }
    }
    app.cancel_download();
    Ok(())
}
fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if app.edit_target != EditTarget::None {
        match key.code {
            KeyCode::Esc => {
                app.edit_target = EditTarget::None;
                app.edit_buffer.clear();
            }
            KeyCode::Enter => {
                let text = app.edit_buffer.trim().to_string();
                let target = app.edit_target;
                let result: anyhow::Result<()> = match target {
                    EditTarget::PersonaCreateName => {
                        if !text.is_empty() {
                            app.create_persona(&text);
                        }
                        Ok(())
                    }
                    EditTarget::PersonaRename => {
                        if !text.is_empty() {
                            app.rename_persona(&text);
                        }
                        Ok(())
                    }
                    EditTarget::AliasAdd | EditTarget::AliasEdit => {
                        if !text.is_empty() {
                            app.alias(target, &text);
                        }
                        Ok(())
                    }
                    EditTarget::HfSearchQuery => {
                        app.hf_query = text;
                        app.search_hf();
                        Ok(())
                    }
                    EditTarget::CustomModelUrl => {
                        app.custom_download(&text);
                        Ok(())
                    }
                    EditTarget::RouteEntry => app.edit_route(&text, false),
                    EditTarget::RouteNew => app.edit_route(&text, true),
                    EditTarget::RoutePreview => app.route_test(&text),
                    EditTarget::PluginSettings => serde_json::from_str(&text)
                        .map_err(anyhow::Error::from)
                        .map(|settings| {
                            if let Some((_, s)) =
                                app.plugin_draft.iter_mut().nth(app.selected_plugin)
                            {
                                s.settings = settings;
                                app.changed();
                            }
                        }),
                    EditTarget::LocationCity => {
                        app.config.location.city = text.trim().into();
                        app.config.location.latitude = None;
                        app.config.location.longitude = None;
                        app.changed();
                        Ok(())
                    }
                    _ => Ok(()),
                };
                match result {
                    Ok(()) => {
                        app.edit_target = EditTarget::None;
                        app.edit_buffer.clear();
                    }
                    Err(e) => app.set_toast(format!("Edit: {e:#}")),
                }
            }
            KeyCode::Backspace => {
                app.edit_buffer.pop();
            }
            KeyCode::Char('u') if ctrl => app.edit_buffer.clear(),
            KeyCode::Char(c) if !ctrl => app.edit_buffer.push(c),
            _ => {}
        }
        return false;
    }
    if ctrl {
        match key.code {
            KeyCode::Char('g') => {
                app.control(
                    "Browser Takeover",
                    assistant_ipc::Control::BrowserTakeover,
                    20_000,
                );
                return false;
            }
            KeyCode::Char('o') => {
                let enabled = !app
                    .daemon_status
                    .as_ref()
                    .is_some_and(|s| s.session["mode"] == "conversation");
                app.control(
                    "Conversation",
                    assistant_ipc::Control::Conversation { enabled },
                    5000,
                );
                return false;
            }
            KeyCode::Char('d') => {
                let action = if app
                    .daemon_status
                    .as_ref()
                    .is_some_and(|s| s.session["mode"] == "dictation")
                {
                    "stop"
                } else {
                    "start"
                };
                app.control(
                    "Dictation",
                    assistant_ipc::Control::Dictation {
                        action: action.into(),
                    },
                    5000,
                );
                return false;
            }
            KeyCode::Char('p') => {
                let action = if app
                    .daemon_status
                    .as_ref()
                    .is_some_and(|s| s.session["mode"] == "dictation_paused")
                {
                    "resume"
                } else {
                    "pause"
                };
                app.control(
                    "Dictation",
                    assistant_ipc::Control::Dictation {
                        action: action.into(),
                    },
                    5000,
                );
                return false;
            }
            KeyCode::Char('b') => {
                app.control(
                    "Browser Resume",
                    assistant_ipc::Control::BrowserResume,
                    20_000,
                );
                return false;
            }
            KeyCode::Char('s') => {
                app.apply(false);
                return false;
            }
            KeyCode::Char('r') => {
                app.apply(true);
                return false;
            }
            KeyCode::Char('t') => {
                let enabled = !app.daemon_status.as_ref().is_some_and(|s| s.tool_mode);
                app.control(
                    "Tool Mode",
                    assistant_ipc::Control::ToolMode { enabled },
                    2000,
                );
                return false;
            }
            KeyCode::Char('c') => return quit(app),
            _ => {}
        }
    }
    if app
        .daemon_status
        .as_ref()
        .is_some_and(|s| !s.pending_approvals.is_empty())
    {
        if key.code == KeyCode::Char('y') {
            app.approve(true, assistant_ipc::ApprovalScope::Once);
            return false;
        }
        if key.code == KeyCode::Char('t') {
            app.approve(true, assistant_ipc::ApprovalScope::Task);
            return false;
        }
        if key.code == KeyCode::Char('n') {
            app.approve(false, assistant_ipc::ApprovalScope::Once);
            return false;
        }
    }
    match key.code {
        KeyCode::Char('q')|KeyCode::Esc=>return quit(app),
        KeyCode::Tab=>{app.active_tab=app.active_tab.next();app.scroll=0;},KeyCode::BackTab=>{app.active_tab=app.active_tab.prev();app.scroll=0;},
        KeyCode::Char('0')=>{app.active_tab=Tab::Plugins;app.scroll=0;},
        KeyCode::Char(c) if ('1'..='9').contains(&c)=>{app.active_tab=Tab::ALL[(c as u8-b'1')as usize];app.scroll=0;},
        KeyCode::PageDown=>app.scroll=app.scroll.saturating_add(10),KeyCode::PageUp=>app.scroll=app.scroll.saturating_sub(10),
        KeyCode::Char('m')=>app.mute(),KeyCode::Char('t')=>app.trigger_preview(),
        KeyCode::Char('s') if app.active_tab==Tab::Dashboard=>app.service(),
        _=>match app.active_tab {
            Tab::Dashboard|Tab::Models=>match key.code {
                KeyCode::Up|KeyCode::Char('k')=>app.selected_model_idx=app.selected_model_idx.saturating_sub(1),
                KeyCode::Down|KeyCode::Char('j')=>app.selected_model_idx=(app.selected_model_idx+1).min(app.installed_models.len().saturating_sub(1)),
                KeyCode::Enter=>if let Some(path)=app.installed_models.get(app.selected_model_idx){app.config.llm.model=path.display().to_string();app.changed();},
                KeyCode::Left|KeyCode::Char('[')=>app.adjust_vad(-1.0),KeyCode::Right|KeyCode::Char(']')=>app.adjust_vad(1.0),
                KeyCode::Char('b')=>{app.config.vad.backend=if app.config.vad.backend=="silero"{"energy"}else{"silero"}.into();app.changed();},
                KeyCode::Char('v')=>app.cycle_voice(),KeyCode::Char('x')=>{app.config.tts.variant=if app.config.tts.variant=="fp32"{"quantized"}else{"fp32"}.into();app.changed();},
                KeyCode::Char('+')|KeyCode::Char('=')=>{app.config.tts.speed=(app.config.tts.speed+0.05).min(2.0);app.changed();},KeyCode::Char('-')=>{app.config.tts.speed=(app.config.tts.speed-0.05).max(0.5);app.changed();},_=>{}
            },
            Tab::Downloader=>match key.code {
                KeyCode::Left=>app.download_pane=app.download_pane.saturating_sub(1),KeyCode::Right=>app.download_pane=(app.download_pane+1).min(if app.hf_files.is_empty(){1}else{2}),
                KeyCode::Up|KeyCode::Char('k')=>{let index=match app.download_pane{0=>&mut app.selected_curated_idx,1=>&mut app.selected_hf_idx,_=>&mut app.selected_file_idx};*index=index.saturating_sub(1);},
                KeyCode::Down|KeyCode::Char('j')=>{let (index,len)=match app.download_pane{0=>(&mut app.selected_curated_idx,models::CURATED_MODELS.len()),1=>(&mut app.selected_hf_idx,app.hf_results.len()),_=>(&mut app.selected_file_idx,app.hf_files.len())};*index=(*index+1).min(len.saturating_sub(1));},
                KeyCode::Enter|KeyCode::Char('d')=>app.download_enter(),KeyCode::Char('c')=>app.cancel_download(),
                KeyCode::Char('/')=>edit(app,EditTarget::HfSearchQuery,app.hf_query.clone()),KeyCode::Char('u')=>edit(app,EditTarget::CustomModelUrl,String::new()),_=>{}
            },
            Tab::Persona=>match key.code {
                KeyCode::Left|KeyCode::Char('h')=>app.persona_pane=PersonaPane::Personas,KeyCode::Right|KeyCode::Char('l')=>app.persona_pane=PersonaPane::Aliases,
                KeyCode::Char('w')=>{app.config.wake_word.enabled = !app.config.wake_word.enabled;app.changed();},
                _=>match app.persona_pane {
                    PersonaPane::Personas=>match key.code {
                        KeyCode::Up|KeyCode::Char('k')=>{app.selected_persona_idx=app.selected_persona_idx.saturating_sub(1);app.selected_alias_idx=0;},KeyCode::Down|KeyCode::Char('j')=>{app.selected_persona_idx=(app.selected_persona_idx+1).min(app.personas.len().saturating_sub(1));app.selected_alias_idx=0;},
                        KeyCode::Enter|KeyCode::Char(' ')=>app.activate_persona(),KeyCode::Char('n')=>edit(app,EditTarget::PersonaCreateName,String::new()),KeyCode::Char('d')|KeyCode::Delete=>app.delete_persona(),
                        KeyCode::Char('e')=>{let name=app.selected_persona().map(|p|p.name.clone()).unwrap_or_default();edit(app,EditTarget::PersonaRename,name);},
                        KeyCode::Char('v')=>{let voices=app.voices.clone();let active=app.config.assistant.persona_id.clone();if let Some(p)=app.personas.get_mut(app.selected_persona_idx){let index=voices.iter().position(|v|*v==p.voice).unwrap_or(0);p.voice=voices[(index+1)%voices.len()].into();if p.id==active{app.config.tts.voice=p.voice.clone();}app.changed();}},_=>{}
                    },
                    PersonaPane::Aliases=>match key.code {
                        KeyCode::Up|KeyCode::Char('k')=>app.selected_alias_idx=app.selected_alias_idx.saturating_sub(1),KeyCode::Down|KeyCode::Char('j')=>app.selected_alias_idx=(app.selected_alias_idx+1).min(app.selected_persona().map(|p|p.aliases.len().saturating_sub(1)).unwrap_or(0)),
                        KeyCode::Char('a')=>edit(app,EditTarget::AliasAdd,String::new()),KeyCode::Char('d')|KeyCode::Delete=>app.alias(EditTarget::None,""),KeyCode::Char('e')|KeyCode::Enter=>{let text=app.selected_persona().and_then(|p|p.aliases.get(app.selected_alias_idx)).cloned().unwrap_or_default();edit(app,EditTarget::AliasEdit,text);},
                        KeyCode::Char('g')=>{if let Some(p)=app.personas.get_mut(app.selected_persona_idx){for a in assistant_core::Persona::generate_aliases(&p.name){if !p.aliases.contains(&a){p.aliases.push(a);}}if p.id==app.config.assistant.persona_id{app.config.wake_word.phrases=p.aliases.clone();}app.changed();}},_=>{}
                    }
                }
            },
            Tab::Features=>match key.code{KeyCode::Char('l')=>edit(app,EditTarget::LocationCity,app.config.location.city.clone()),KeyCode::Up|KeyCode::Char('k')=>app.selected_feature_idx=app.selected_feature_idx.saturating_sub(1),KeyCode::Down|KeyCode::Char('j')=>app.selected_feature_idx=(app.selected_feature_idx+1).min(12),KeyCode::Enter|KeyCode::Char(' ')=>app.toggle_feature(),KeyCode::Left=>app.adjust_vad(-1.0),KeyCode::Right=>app.adjust_vad(1.0),_=>{}},
            Tab::Plugins=>match key.code {
                KeyCode::Up|KeyCode::Char('k')=>app.selected_plugin=app.selected_plugin.saturating_sub(1),
                KeyCode::Down|KeyCode::Char('j')=>app.selected_plugin=(app.selected_plugin+1).min(app.plugin_draft.len().saturating_sub(1)),
                KeyCode::Enter|KeyCode::Char(' ')=>{if let Some((_,s))=app.plugin_draft.iter_mut().nth(app.selected_plugin){s.enabled = !s.enabled;app.changed();}},
                KeyCode::Char('e')=>{if let Some((_,s))=app.plugin_draft.iter().nth(app.selected_plugin){edit(app,EditTarget::PluginSettings,s.settings.to_string());}},
                KeyCode::Char('r')=>{if let Some((id,_))=app.plugin_draft.iter().nth(app.selected_plugin){app.control("Plugin Retry",assistant_ipc::Control::Plugins{action:"reload".into(),id:Some(id.clone())},30_000);}},
                _=>{}
            },
            Tab::Diagnostics=>if matches!(key.code,KeyCode::Enter|KeyCode::Char('r')){app.control("Diagnostics",assistant_ipc::Control::Diagnostics,15_000);},
            Tab::Activity=>match key.code {KeyCode::Char('c')=>app.control("Clear Activity",assistant_ipc::Control::ClearActivity,2000),KeyCode::Up=>app.scroll=app.scroll.saturating_sub(1),KeyCode::Down=>app.scroll=app.scroll.saturating_add(1),_=>{}},
            Tab::Calibration=>match key.code{KeyCode::Enter=>app.control("Calibration",assistant_ipc::Control::Calibrate,30_000),KeyCode::Char('a')=>app.apply_calibration(),KeyCode::Char('c')=>{tokio::spawn(async{let _=assistant_ipc::send_command(&assistant_ipc::Command::Cancel).await;});},_=>{}},
            Tab::Commands=>match key.code {
                KeyCode::Left|KeyCode::Right=>{app.commands_pane = !app.commands_pane;app.selected_route=0;},KeyCode::Up|KeyCode::Char('k')=>app.selected_route=app.selected_route.saturating_sub(1),KeyCode::Down|KeyCode::Char('j')=>app.selected_route=(app.selected_route+1).min(app.route_entries().len().saturating_sub(1)),
                KeyCode::Enter|KeyCode::Char('e')=>{let text=app.route_entries().get(app.selected_route).map(ValueExt::json_string).unwrap_or_default();edit(app,EditTarget::RouteEntry,text);},
                KeyCode::Char('n')=>edit(app,EditTarget::RouteNew,if app.commands_pane{r#"{"phrases":["my command"],"tool":"system.lock","reply":"Locking.","omarchy":["system","lock"]}"#}else{r#"{"phrases":["my app"],"reply":"Opening.","exec":["program","argument with spaces"]}"#}.into()),
                KeyCode::Char('p')=>edit(app,EditTarget::RoutePreview,String::new()),KeyCode::Delete|KeyCode::Char('d')=>{if let Err(e)=app.delete_route(){app.set_toast(e.to_string());}},_=>{}
            }
        }
    }
    false
}
trait ValueExt {
    fn json_string(&self) -> String;
}
impl ValueExt for serde_json::Value {
    fn json_string(&self) -> String {
        self.to_string()
    }
}
fn edit(app: &mut App, target: EditTarget, text: String) {
    app.edit_target = target;
    app.edit_buffer = text;
}
fn quit(app: &mut App) -> bool {
    if app.operation_busy {
        app.set_toast("Wait for the operation to finish; calibration can be cancelled with c");
        return false;
    }
    if app.is_dirty && !app.quit_pending {
        app.quit_pending = true;
        app.set_toast("Unsaved draft. Ctrl+S Apply, or q again to discard and quit.");
        return false;
    }
    true
}
