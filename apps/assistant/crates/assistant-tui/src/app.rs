//! Draft state plus background operations. The event loop never awaits network/model work.
use crate::daemon::{DaemonClient, DaemonStatus};
use crate::models::{self, DownloadStatus, HfModelEntry, CURATED_MODELS};
use crate::theme::Theme;
use anyhow::{Context, Result};
use assistant_core::{paths, AppsFile, CommandsFile, Config, Persona};
use assistant_ipc::{request, Control};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::{mpsc, watch};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Models,
    Downloader,
    Persona,
    Features,
    Diagnostics,
    Activity,
    Calibration,
    Commands,
    Plugins,
}
impl Tab {
    pub const ALL: [Self; 10] = [
        Self::Dashboard,
        Self::Models,
        Self::Downloader,
        Self::Persona,
        Self::Features,
        Self::Diagnostics,
        Self::Activity,
        Self::Calibration,
        Self::Commands,
        Self::Plugins,
    ];
    pub fn index(self) -> usize {
        self as usize
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersonaPane {
    Personas,
    Aliases,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditTarget {
    None,
    PersonaCreateName,
    PersonaRename,
    AliasAdd,
    AliasEdit,
    HfSearchQuery,
    CustomModelUrl,
    RouteEntry,
    RouteNew,
    RoutePreview,
    LocationCity,
    PluginSettings,
}
pub enum Update {
    Poll(Option<Box<DaemonStatus>>, bool, Value),
    Result(String, Result<Value>),
    Applied(Box<Config>, Vec<Persona>, String, String, Result<Value>),
    Hf(Result<Vec<HfModelEntry>>),
    Files(String, Result<Vec<String>>),
    Event(assistant_core::Event),
}
pub struct App {
    pub active_tab: Tab,
    pub theme: Theme,
    pub config: Config,
    pub saved: Config,
    pub is_dirty: bool,
    pub daemon_status: Option<DaemonStatus>,
    pub service_active: bool,
    pub installed_models: Vec<PathBuf>,
    pub selected_model_idx: usize,
    pub selected_curated_idx: usize,
    pub selected_feature_idx: usize,
    pub selected_voice_idx: usize,
    pub voices: Vec<&'static str>,
    pub personas: Vec<Persona>,
    pub saved_personas: Vec<Persona>,
    pub selected_persona_idx: usize,
    pub persona_pane: PersonaPane,
    pub selected_alias_idx: usize,
    pub download_progress_rx: watch::Receiver<DownloadStatus>,
    pub download_progress_tx: watch::Sender<DownloadStatus>,
    pub download_cancel_flag: Option<Arc<AtomicBool>>,
    pub toast: Option<(String, std::time::Instant)>,
    pub edit_target: EditTarget,
    pub edit_buffer: String,
    pub hf_query: String,
    pub hf_results: Vec<HfModelEntry>,
    pub selected_hf_idx: usize,
    pub hf_files: Vec<String>,
    pub selected_file_idx: usize,
    pub hf_repo: String,
    pub download_pane: usize,
    pub updates: mpsc::UnboundedReceiver<Update>,
    tx: mpsc::UnboundedSender<Update>,
    pub poll_busy: bool,
    pub operation_busy: bool,
    pub diagnostics: Value,
    pub activity: Value,
    pub calibration: Value,
    pub quit_pending: bool,
    pub apps_text: String,
    pub commands_text: String,
    pub commands_pane: bool,
    pub selected_route: usize,
    pub route_preview: String,
    pub plugin_draft: std::collections::BTreeMap<String, assistant_tools::plugins::Saved>,
    pub plugin_saved: std::collections::BTreeMap<String, assistant_tools::plugins::Saved>,
    pub selected_plugin: usize,
    /// Screens, panes, mouse targets and the F2 menu.
    pub navigation: crate::navigation::State,
    /// Ctrl+K actions.
    pub commands: crate::commands::Palette,
    pub help: bool,
    pub help_scroll: u16,
    /// "Discard the draft and quit?"
    pub quit_confirm: bool,
    /// Set by a confirmed quit from the mouse.
    pub quit_now: bool,
    /// The approval the person put aside with Esc; it stays pending in the daemon.
    pub approval_hidden: Option<String>,
    pub approval_scroll: u16,
    pub selected_turn: usize,
    /// `aurora`, `omarchy`, `terminal` or a built-in palette (`tui.toml`).
    pub theme_id: String,
    pub omarchy: Option<Theme>,
    pub mouse: bool,
}
impl App {
    pub fn new(theme_id: String, omarchy: Option<Theme>) -> Result<Self> {
        let theme = crate::theme::resolve(&theme_id, omarchy.as_ref());
        let mut config = Config::load()?;
        let personas = Persona::load_all()?;
        if config.assistant.persona_id.is_empty() {
            if let Some(p) = personas
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(&config.assistant.name))
            {
                config.assistant.persona_id = p.id.clone();
            }
        }
        let voices = vec![
            "af_heart",
            "af_bella",
            "af_sarah",
            "af_nicole",
            "af_sky",
            "am_adam",
            "am_michael",
            "bf_emma",
            "bf_isabella",
            "bm_george",
            "bm_lewis",
        ];
        let selected_voice_idx = voices
            .iter()
            .position(|v| *v == config.tts.voice)
            .unwrap_or(0);
        let selected_persona_idx = personas
            .iter()
            .position(|p| p.id == config.assistant.persona_id)
            .unwrap_or(0);
        let (tx, updates) = mpsc::unbounded_channel();
        let (download_progress_tx, download_progress_rx) =
            watch::channel(DownloadStatus::default());
        let apps_text = if paths::apps_path().exists() {
            std::fs::read_to_string(paths::apps_path())?
        } else {
            include_str!("../../../config/apps.toml").into()
        };
        let commands_text = if paths::commands_path().exists() {
            std::fs::read_to_string(paths::commands_path())?
        } else {
            include_str!("../../../config/commands.toml").into()
        };
        let app = Self {
            active_tab: Tab::Dashboard,
            theme,
            saved: config.clone(),
            config,
            is_dirty: false,
            daemon_status: None,
            service_active: false,
            installed_models: Config::scan_local_models(),
            selected_model_idx: 0,
            selected_curated_idx: 0,
            selected_feature_idx: 0,
            selected_voice_idx,
            voices,
            saved_personas: personas.clone(),
            personas,
            selected_persona_idx,
            persona_pane: PersonaPane::Personas,
            selected_alias_idx: 0,
            download_progress_rx,
            download_progress_tx,
            download_cancel_flag: None,
            toast: None,
            edit_target: EditTarget::None,
            edit_buffer: String::new(),
            hf_query: String::new(),
            hf_results: Vec::new(),
            selected_hf_idx: 0,
            hf_files: Vec::new(),
            selected_file_idx: 0,
            hf_repo: String::new(),
            download_pane: 0,
            updates,
            tx,
            poll_busy: false,
            operation_busy: false,
            diagnostics: Value::Null,
            activity: json!({"turns":[]}),
            calibration: Value::Null,
            quit_pending: false,
            apps_text,
            commands_text,
            commands_pane: false,
            selected_route: 0,
            route_preview: String::new(),
            plugin_draft: assistant_tools::plugins::saved()?,
            plugin_saved: assistant_tools::plugins::saved()?,
            selected_plugin: 0,
            navigation: crate::navigation::State::default(),
            commands: crate::commands::Palette::default(),
            help: false,
            help_scroll: 0,
            quit_confirm: false,
            quit_now: false,
            approval_hidden: None,
            approval_scroll: 0,
            selected_turn: 0,
            theme_id,
            omarchy,
            mouse: true,
        };
        app.routes()?;
        let event_tx = app.tx.clone();
        tokio::spawn(async move {
            loop {
                let tx = event_tx.clone();
                let _ = assistant_ipc::stream_events(None, move |ev| {
                    let _ = tx.send(Update::Event(ev));
                })
                .await;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        });
        Ok(app)
    }
    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), std::time::Instant::now()));
    }
    pub fn clear_expired_toast(&mut self) {
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed().as_secs() > 8)
        {
            self.toast = None;
        }
    }
    pub fn changed(&mut self) {
        self.is_dirty = true;
        self.quit_pending = false;
    }
    pub fn poll_daemon(&mut self) {
        if self.poll_busy {
            return;
        }
        self.poll_busy = true;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let status = DaemonClient::new(0).fetch_status().await.ok();
            let active = DaemonClient::is_service_active().await;
            let activity = request(Control::Activity, 1200)
                .await
                .unwrap_or(Value::Null);
            let _ = tx.send(Update::Poll(status.map(Box::new), active, activity));
        });
    }
    pub fn control(&mut self, label: &str, action: Control, timeout: u64) {
        if self.operation_busy {
            self.set_toast("An operation is already running");
            return;
        }
        self.operation_busy = true;
        self.set_toast(format!("{label}…"));
        let label = label.to_string();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = request(action, timeout).await;
            let _ = tx.send(Update::Result(label, result));
        });
    }
    pub fn apply(&mut self, restart: bool) {
        if self.operation_busy {
            self.set_toast("An operation is already running");
            return;
        }
        if let Err(e) = self.validate_draft() {
            self.set_toast(format!("Apply failed: {e:#}"));
            return;
        }
        self.operation_busy = true;
        let config = self.config.clone();
        let personas = self.personas.clone();
        let old = self.saved_personas.clone();
        let tx = self.tx.clone();
        let apps = self.apps_text.clone();
        let commands = self.commands_text.clone();
        let plugin_state = self.plugin_draft.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking({
                let config = config.clone();
                let personas = personas.clone();
                let apps = apps.clone();
                let commands = commands.clone();
                move || {
                    config.save()?;
                    assistant_core::config::atomic_write(&paths::apps_path(), apps.as_bytes())?;
                    assistant_core::config::atomic_write(
                        &paths::commands_path(),
                        commands.as_bytes(),
                    )?;
                    for p in &personas {
                        p.save()?;
                    }
                    for p in old {
                        if !personas.iter().any(|new| new.id == p.id) {
                            p.delete()?;
                        }
                    }
                    if let Some(p) = personas
                        .iter()
                        .find(|p| p.id == config.assistant.persona_id)
                    {
                        assistant_core::sync_plugin_manifest(p)?;
                    }
                    Ok::<_, anyhow::Error>(())
                }
            })
            .await;
            let result = match result {
                Ok(Ok(())) => {
                    let result = request(
                        Control::PluginApply {
                            state: serde_json::to_value(&plugin_state).unwrap(),
                        },
                        30_000,
                    )
                    .await;
                    if let Err(e) = result {
                        let _ = tx.send(Update::Applied(
                            Box::new(config),
                            personas,
                            apps,
                            commands,
                            Err(e),
                        ));
                        return;
                    }

                    if restart {
                        DaemonClient::restart_service()
                            .await
                            .map(|_| json!({"restarted":true}))
                    } else {
                        DaemonClient::new(0).reload().await
                    }
                }
                Ok(Err(e)) => Err(e.context("save failed")),
                Err(e) => Err(e.into()),
            };
            let _ = tx.send(Update::Applied(
                Box::new(config),
                personas,
                apps,
                commands,
                result,
            ));
        });
    }
    pub fn validate_draft(&self) -> Result<()> {
        self.config.validate()?;
        for (id, s) in &self.plugin_draft {
            let m = assistant_tools::plugins::Manifest::read(
                &assistant_tools::plugins::package_dir().join(id),
            )?;
            if s.enabled {
                anyhow::ensure!(
                    m.dependencies
                        .iter()
                        .all(|d| self.plugin_draft.get(d).is_some_and(|s| s.enabled)),
                    "disable dependents too"
                );
            }
        }
        let (a, c) = self.routes()?;
        assistant_core::routing::validate(&a, &c)?;
        Ok(())
    }
    pub fn update(&mut self, update: Update) {
        match update {
            Update::Poll(status, active, activity) => {
                self.poll_busy = false;
                if let Some(status) = &status {
                    if let Some(plugins) = status.plugins.as_array() {
                        for plugin in plugins {
                            if let (Some(id), Ok(saved)) = (
                                plugin["id"].as_str(),
                                serde_json::from_value::<assistant_tools::plugins::Saved>(
                                    plugin["saved"].clone(),
                                ),
                            ) {
                                self.plugin_saved.insert(id.into(), saved.clone());
                                if !self.is_dirty || !self.plugin_draft.contains_key(id) {
                                    self.plugin_draft.insert(id.into(), saved);
                                }
                            }
                        }
                        if !self.is_dirty {
                            self.plugin_draft
                                .retain(|id, _| plugins.iter().any(|p| p["id"] == id.as_str()));
                        }
                    }
                }
                self.daemon_status = status.map(|s| *s);
                self.service_active = active;
                if !activity.is_null() {
                    self.activity = activity;
                }
            }
            Update::Result(label, result) => {
                self.operation_busy = false;
                match result {
                    Ok(value) => {
                        match label.as_str() {
                            "Diagnostics" => self.diagnostics = value,
                            "Calibration" => self.calibration = value,
                            "Clear Activity" => self.activity = json!({"turns":[]}),
                            _ => {}
                        }
                        self.set_toast(format!("{label} complete"));
                        self.poll_daemon();
                    }
                    Err(e) => self.set_toast(format!("{label}: {e:#}")),
                }
            }
            Update::Applied(config, personas, apps, commands, result) => {
                if result.is_ok() {
                    self.plugin_saved = self.plugin_draft.clone();
                }
                self.operation_busy = false;
                // Confirm disk content, even if the daemon was offline or reload failed.
                let saved = Config::load().ok().is_some_and(|c| {
                    serde_json::to_value(&c).ok() == serde_json::to_value(&*config).ok()
                });
                if saved {
                    self.saved = *config;
                    self.saved_personas = personas;
                    let same = serde_json::to_value(&self.config).ok()
                        == serde_json::to_value(&self.saved).ok()
                        && self.personas == self.saved_personas
                        && self.apps_text == apps
                        && self.commands_text == commands;
                    self.is_dirty = !same;
                }
                match result {
                    Ok(v) => self.set_toast(format!(
                        "Applied. Restart required: {}",
                        v["restart_required"]
                            .as_array()
                            .map(|a| a
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", "))
                            .unwrap_or_else(|| "none".into())
                    )),
                    Err(e) => {
                        self.is_dirty = true;
                        self.set_toast(format!("Apply: {e:#}"));
                    }
                }
                self.poll_daemon();
            }
            Update::Hf(result) => {
                self.operation_busy = false;
                match result {
                    Ok(r) => {
                        self.hf_results = r;
                        self.selected_hf_idx = 0;
                        self.hf_files.clear();
                        self.download_pane = 1;
                        self.navigation.focus[Tab::Downloader.index()] = 1;
                        self.set_toast("Select a repository; (Enter) lists its GGUF files");
                    }
                    Err(e) => self.set_toast(format!("Search: {e}")),
                }
            }
            Update::Files(repo, result) => {
                self.operation_busy = false;
                match result {
                    Ok(files) => {
                        self.hf_repo = repo;
                        self.hf_files = files;
                        self.selected_file_idx = 0;
                        self.download_pane = 2;
                        self.navigation.focus[Tab::Downloader.index()] = 2;
                    }
                    Err(e) => self.set_toast(format!("Files: {e}")),
                }
            }
            Update::Event(ev) => match ev {
                assistant_core::Event::ToolApproval {
                    tool,
                    arguments,
                    task_scope_allowed,
                    summary,
                    ..
                } => {
                    let _ = task_scope_allowed;
                    // A new request shows its dialog even if an earlier one was put aside.
                    self.approval_hidden = None;
                    self.approval_scroll = 0;
                    self.set_toast(format!(
                        "Approval needed: {}",
                        if summary.is_empty() { format!("{tool} {arguments}") } else { summary }
                    ));
                    self.poll_daemon();
                }
                assistant_core::Event::Turn { event, .. } if event["type"] == "calibration" => {
                    self.set_toast(format!("Calibration: {} for five seconds", event["phase"]))
                }
                _ => {}
            },
        }
    }
    pub fn trigger_preview(&mut self) {
        self.control(
            "Voice Preview",
            Control::Preview {
                voice: self.config.tts.voice.clone(),
                speed: self.config.tts.speed,
            },
            30_000,
        );
    }
    pub fn approve(&mut self, allow: bool, scope: assistant_ipc::ApprovalScope) {
        if let Some(id) = self
            .daemon_status
            .as_ref()
            .and_then(|s| s.pending_approvals.keys().next())
            .cloned()
        {
            self.control(
                "Tool Approval",
                Control::Approve {
                    approval_id: id,
                    allow,
                    scope,
                },
                2000,
            );
        }
    }
    pub fn service(&mut self) {
        if self.operation_busy {
            return;
        }
        self.operation_busy = true;
        let active = self.service_active;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = if active {
                DaemonClient::stop_service().await
            } else {
                DaemonClient::start_service().await
            };
            let _ = tx.send(Update::Result(
                "Service".into(),
                result.map(|_| Value::Null),
            ));
        });
    }
    pub fn mute(&mut self) {
        let target = self.daemon_status.as_ref().map(|s| !s.muted);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = DaemonClient::new(0).toggle_mute(target).await;
            let _ = tx.send(Update::Result("Mute".into(), result.map(|_| Value::Null)));
        });
    }
    pub fn start_download(&mut self, url: String, target: PathBuf) {
        if self.download_cancel_flag.is_some() {
            self.set_toast("A download is already active");
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.download_cancel_flag = Some(cancel.clone());
        let tx = self.download_progress_tx.clone();
        tx.send_replace(DownloadStatus {
            state: models::DownloadState::Connecting,
            is_downloading: true,
            ..Default::default()
        });
        tokio::spawn(async move {
            let _ = models::download_file(&url, target, tx, cancel).await;
        });
    }
    pub fn start_curated_download(&mut self) {
        let m = &CURATED_MODELS[self.selected_curated_idx];
        self.start_download(m.download_url.into(), paths::models_dir().join(m.filename));
    }
    pub fn custom_download(&mut self, url: &str) {
        match models::download_target(url) {
            Ok((url, path)) => self.start_download(url, path),
            Err(e) => self.set_toast(e.to_string()),
        }
    }
    pub fn download_enter(&mut self) {
        match self.download_pane {
            0 => self.start_curated_download(),
            1 => {
                if self.operation_busy {
                    return;
                }
                if let Some(repo) = self.hf_results.get(self.selected_hf_idx) {
                    let repo = repo.id.clone();
                    let tx = self.tx.clone();
                    self.operation_busy = true;
                    tokio::spawn(async move {
                        let result = models::hf_files(&repo).await;
                        let _ = tx.send(Update::Files(repo, result));
                    });
                }
            }
            _ => {
                if let Some(file) = self.hf_files.get(self.selected_file_idx) {
                    if models::split_model(file) {
                        self.set_toast("Split models are unsupported; choose a single GGUF");
                        return;
                    }
                    let url = format!(
                        "https://huggingface.co/{}/resolve/main/{file}",
                        self.hf_repo
                    );
                    self.custom_download(&url);
                }
            }
        }
    }
    pub fn search_hf(&mut self) {
        if self.operation_busy {
            return;
        }
        self.operation_busy = true;
        let query = self.hf_query.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Update::Hf(models::search_huggingface(&query).await));
        });
    }
    pub fn cancel_download(&mut self) {
        if let Some(flag) = &self.download_cancel_flag {
            flag.store(true, Ordering::SeqCst);
            self.set_toast("Cancelling download…");
        }
    }
    pub fn tick_download(&mut self) {
        let status = self.download_progress_rx.borrow().clone();
        if self.download_cancel_flag.is_some() && !status.is_downloading {
            self.download_cancel_flag = None;
            if status.state == models::DownloadState::Installed {
                self.installed_models = Config::scan_local_models();
                self.set_toast("Model installed");
            } else {
                self.set_toast(format!(
                    "Download {:?}: {}",
                    status.state,
                    status.error.unwrap_or_default()
                ));
            }
        }
    }
    pub fn vad_threshold(&self) -> f32 {
        if self.config.vad.backend == "silero" {
            self.config.vad.threshold
        } else {
            self.config.vad.energy_threshold
        }
    }
    pub fn vad_sensitivity_label(&self) -> &'static str {
        if self.config.vad.backend == "energy" {
            "RMS amplitude"
        } else {
            Self::sensitivity_label(self.config.vad.threshold)
        }
    }
    pub fn sensitivity_label(v: f32) -> &'static str {
        if v <= 0.35 {
            "Sensitive"
        } else if v <= 0.65 {
            "Balanced"
        } else {
            "Strict"
        }
    }
    pub fn adjust_vad(&mut self, direction: f32) {
        if self.config.vad.backend == "energy" {
            self.config.vad.energy_threshold =
                (self.config.vad.energy_threshold + direction * 0.001).clamp(0.0001, 0.5);
        } else {
            self.config.vad.threshold =
                (self.config.vad.threshold + direction * 0.05).clamp(0.05, 0.95);
        }
        self.changed();
    }
    pub fn toggle_feature(&mut self) {
        match self.selected_feature_idx {
            0 => self.config.wake_word.enabled = !self.config.wake_word.enabled,
            1 => {
                self.set_toast("PTT is the supported capture mode");
                return;
            }
            2 => self.config.barge_in.enabled = !self.config.barge_in.enabled,
            3 => self.config.barge_in.aec = !self.config.barge_in.aec,
            4 => self.adjust_vad(1.0),
            5 => self.config.agent.enabled = !self.config.agent.enabled,
            6 => {
                // Only agents installed here are offered.
                let agents = assistant_tools::agent::installed_agents();
                if agents.is_empty() {
                    self.set_toast("No coding agents are installed (see omarchy default agent)");
                    return;
                }
                let idx = agents
                    .iter()
                    .position(|a| a.id == self.config.agent.default_agent)
                    .map_or(0, |i| (i + 1) % agents.len());
                self.config.agent.default_agent = agents[idx].id.into();
            }
            7 => self.config.llm.flash_attn = !self.config.llm.flash_attn,
            8 => self.config.llm.prefix_cache = !self.config.llm.prefix_cache,
            9 => self.config.http.enabled = !self.config.http.enabled,
            10 => self.config.ui.enabled = !self.config.ui.enabled,
            11 => self.config.clock.enabled = !self.config.clock.enabled,
            12 => self.config.location.enabled = !self.config.location.enabled,
            _ => {}
        }
        self.changed();
    }
    pub fn cycle_voice(&mut self) {
        self.selected_voice_idx = (self.selected_voice_idx + 1) % self.voices.len();
        self.config.tts.voice = self.voices[self.selected_voice_idx].into();
        if let Some(p) = self
            .personas
            .iter_mut()
            .find(|p| p.id == self.config.assistant.persona_id)
        {
            p.voice = self.config.tts.voice.clone();
        }
        self.changed();
    }
    pub fn selected_persona(&self) -> Option<&Persona> {
        self.personas.get(self.selected_persona_idx)
    }
    pub fn activate_persona(&mut self) {
        if let Some(p) = self.selected_persona().cloned() {
            self.config.activate_persona(&p);
            self.changed();
        }
    }
    pub fn create_persona(&mut self, name: &str) {
        let p = Persona::new_with_generated(name);
        if self
            .personas
            .iter()
            .any(|old| old.id == p.id || old.name.eq_ignore_ascii_case(name))
            || paths::personas_dir()
                .join(format!("{}.toml", p.id))
                .exists()
        {
            self.set_toast("Persona name/ID already exists");
            return;
        }
        self.personas.push(p);
        self.selected_persona_idx = self.personas.len() - 1;
        self.changed();
    }
    pub fn delete_persona(&mut self) {
        if let Some(p) = self.selected_persona() {
            if Persona::is_prebuilt(&p.id) || p.id == self.config.assistant.persona_id {
                self.set_toast("Cannot delete a built-in or active persona");
                return;
            }
            self.personas.remove(self.selected_persona_idx);
            self.selected_persona_idx = self.selected_persona_idx.saturating_sub(1);
            self.changed();
        }
    }
    pub fn rename_persona(&mut self, name: &str) {
        if self
            .personas
            .iter()
            .enumerate()
            .any(|(i, p)| i != self.selected_persona_idx && p.name.eq_ignore_ascii_case(name))
        {
            self.set_toast("Persona name already exists");
            return;
        }
        if let Some(p) = self.personas.get_mut(self.selected_persona_idx) {
            let active = p.id == self.config.assistant.persona_id;
            p.name = name.into();
            if active {
                self.config.activate_persona(p);
            }
            self.changed();
        }
    }
    pub fn alias(&mut self, target: EditTarget, text: &str) {
        if let Some(p) = self.personas.get_mut(self.selected_persona_idx) {
            let text = text.trim().to_lowercase();
            match target {
                EditTarget::AliasAdd => {
                    if !p.aliases.contains(&text) {
                        p.aliases.push(text);
                    }
                }
                EditTarget::AliasEdit => {
                    if p.aliases
                        .iter()
                        .enumerate()
                        .any(|(i, a)| i != self.selected_alias_idx && a == &text)
                    {
                        self.set_toast("Alias already exists");
                        return;
                    }
                    if let Some(a) = p.aliases.get_mut(self.selected_alias_idx) {
                        *a = text;
                    }
                }
                _ => {
                    if self.selected_alias_idx < p.aliases.len() {
                        p.aliases.remove(self.selected_alias_idx);
                        self.selected_alias_idx = self.selected_alias_idx.saturating_sub(1);
                    }
                }
            }
            if p.id == self.config.assistant.persona_id {
                self.config.wake_word.phrases = p.aliases.clone();
            }
            self.changed();
        }
    }
    pub fn routes(&self) -> Result<(AppsFile, CommandsFile)> {
        Ok((
            toml::from_str(&self.apps_text)?,
            toml::from_str(&self.commands_text)?,
        ))
    }
    pub fn route_entries(&self) -> Vec<Value> {
        self.route_list(self.commands_pane)
    }
    /// The apps (false) or commands (true) in the draft, as JSON.
    pub fn route_list(&self, commands: bool) -> Vec<Value> {
        match self.routes() {
            Ok((a, c)) => {
                if commands {
                    c.command
                        .iter()
                        .filter_map(|v| serde_json::to_value(v).ok())
                        .collect()
                } else {
                    a.app
                        .iter()
                        .filter_map(|v| serde_json::to_value(v).ok())
                        .collect()
                }
            }
            Err(_) => Vec::new(),
        }
    }
    pub fn edit_route(&mut self, text: &str, new: bool) -> Result<()> {
        let value: Value = serde_json::from_str(text)?;
        let (key, raw) = if self.commands_pane {
            ("command", &self.commands_text)
        } else {
            ("app", &self.apps_text)
        };
        let mut doc = raw.parse::<toml_edit::DocumentMut>()?;
        let encoded = if self.commands_pane {
            toml::to_string(&serde_json::from_value::<assistant_core::CommandPhrase>(
                value,
            )?)?
        } else {
            toml::to_string(&serde_json::from_value::<assistant_core::AppLaunch>(value)?)?
        };
        let source = encoded.parse::<toml_edit::DocumentMut>()?;
        let array = doc[key]
            .as_array_of_tables_mut()
            .context("missing entries")?;
        if new {
            array.push(source.as_table().clone());
        } else {
            let table = array
                .get_mut(self.selected_route)
                .context("missing entry")?;
            assistant_core::config::merge_table(table, source.as_table());
        }
        let draft = doc.to_string();
        let (a, c) = if self.commands_pane {
            (
                toml::from_str::<AppsFile>(&self.apps_text)?,
                toml::from_str::<CommandsFile>(&draft)?,
            )
        } else {
            (
                toml::from_str::<AppsFile>(&draft)?,
                toml::from_str::<CommandsFile>(&self.commands_text)?,
            )
        };
        assistant_core::routing::validate(&a, &c)?;
        if self.commands_pane {
            self.commands_text = draft;
        } else {
            self.apps_text = draft;
        }
        self.changed();
        Ok(())
    }
    pub fn route_test(&mut self, text: &str) -> Result<()> {
        let (a, c) = self.routes()?;
        let intent = assistant_agent::route_with(text, &a.app, &c);
        self.route_preview = format!("{intent:?} (preview only; nothing executed)");
        Ok(())
    }
    pub fn apply_calibration(&mut self) {
        let s = &self.calibration["suggestions"];
        if let Some(v) = s["energy_threshold"].as_f64() {
            self.config.vad.energy_threshold = v as f32;
        }
        if let Some(v) = s["wake_threshold"].as_f64() {
            self.config.wake_word.threshold = v as f32;
        }
        if let Some(v) = s["silero_threshold"].as_f64() {
            self.config.vad.threshold = v as f32;
        }
        self.changed();
        self.set_toast("Suggestions copied to draft; Apply to save");
    }
}

impl App {
    pub fn delete_route(&mut self) -> Result<()> {
        let (key, raw) = if self.commands_pane {
            ("command", &self.commands_text)
        } else {
            ("app", &self.apps_text)
        };
        let mut doc = raw.parse::<toml_edit::DocumentMut>()?;
        let array = doc[key]
            .as_array_of_tables_mut()
            .context("missing entries")?;
        if self.selected_route < array.len() {
            array.remove(self.selected_route);
        }
        if self.commands_pane {
            self.commands_text = doc.to_string();
        } else {
            self.apps_text = doc.to_string();
        }
        self.selected_route = self.selected_route.saturating_sub(1);
        self.changed();
        Ok(())
    }
}

impl App {
    pub fn model_path(&self) -> PathBuf {
        if self.config.llm.model == "auto" {
            Config::auto_model(&self.installed_models)
        } else {
            PathBuf::from(&self.config.llm.model)
        }
    }
}
