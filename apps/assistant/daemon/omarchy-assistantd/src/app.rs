use anyhow::{Context, Result};
use assistant_agent::{
    is_filler, route_with, strip_wake, Intent, KokoroProvider, PhraseChunker,
    VoxtypeProvider,
};
use assistant_audio::{
    aec_barge_threshold, f32_24k_to_i16_16k, record_until, record_until_vad, AudioEngine,
    Silero, StreamingVad, VadDecision, VadGate,
};
use assistant_core::apps::AppsFile;
use assistant_core::commands::CommandsFile;
use assistant_core::config::Config;
use assistant_core::event::Event;
use assistant_core::state::{AssistantState, StateMachine};
use assistant_core::timing::TurnTiming;
use assistant_inference::LlamaCppBackend;
use assistant_ipc::{Command, Ipc};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, Mutex, watch};
use tokio_util::sync::CancellationToken;

pub async fn run(cfg: Config) -> Result<()> {
    tracing::info!(prompt = %cfg.assistant.prompt, "loaded config.toml");
    let ipc = assistant_ipc::bind().await.context("ipc bind")?;
    let Ipc {
        mut commands,
        command_tx,
        events,
    } = ipc;
    let audio = tokio::task::spawn_blocking({
        let cfg = cfg.clone();
        move || AudioEngine::start(&cfg)
    })
    .await??;
    let stt = VoxtypeProvider::new(&cfg);
    let llm = LlamaCppBackend::spawn(&cfg).context("llama.cpp")?;
    let tts = tokio::task::spawn_blocking({
        let cfg = cfg.clone();
        move || KokoroProvider::load(&cfg)
    })
    .await??;

    let samples = audio.take_samples().context("capture channel")?;
    let aec_samples = audio.take_aec_samples();
    let silero = load_silero(&cfg);
    let (release_tx, release_rx) = watch::channel(true);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let apps = AppsFile::load().unwrap_or_else(|err| {
        tracing::warn!(%err, "apps.toml; using shipped defaults");
        AppsFile::shipped()
    });
    tracing::info!(n = apps.app.len(), "loaded voice apps");
    let voice_commands = CommandsFile::load().unwrap_or_else(|err| {
        tracing::warn!(%err, "commands.toml; using shipped defaults");
        CommandsFile::shipped()
    });
    tracing::info!(n = voice_commands.command.len(), "loaded voice commands");
    let app = Arc::new(App {
        cfg: std::sync::Mutex::new(cfg),
        apps: Mutex::new(apps),
        commands: Mutex::new(voice_commands),
        state: Mutex::new(StateMachine::new()),
        events,
        audio,
        stt,
        llm,
        tts: Arc::new(tts),
        samples: Mutex::new(samples),
        aec_samples: Mutex::new(aec_samples),
        silero: std::sync::Mutex::new(silero),
        released: release_tx,
        _released_rx: release_rx,
        turn_busy: AtomicBool::new(false),
        turn_gen: AtomicU64::new(0),
        need_keyup: AtomicBool::new(false),
        last_hold: std::sync::Mutex::new(Instant::now()),
        history: Mutex::new(Vec::new()),
        cancel: Mutex::new(CancellationToken::new()),
        barged_in: AtomicBool::new(false),
        shutdown: shutdown_tx,
        wake_hush_until: Mutex::new(Instant::now()),
    });

    if let Err(err) = assistant_omarchy::unmute_default_source().await {
        tracing::warn!(%err, "could not unmute default mic");
    }

    app.emit_state(AssistantState::Idle).await;
    app.emit_health();

    let _http_handle = if app.cfg().http.enabled {
        let app_status = Arc::clone(&app);
        let http_state = assistant_ipc::HttpState {
            cmd_tx: command_tx,
            ev_tx: app.events.clone(),
            status_fn: Arc::new(move || app_status.health_json()),
        };
        let http_cfg = app.cfg().http;
        match assistant_ipc::start_http_server(&http_cfg.bind, http_cfg.port, http_state).await {
            Ok(handle) => Some(handle),
            Err(err) => {
                tracing::warn!(%err, "failed to start HTTP server");
                None
            }
        }
    } else {
        None
    };

    {
        let app = Arc::clone(&app);
        let mut shutdown = app.shutdown.subscribe();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(50));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        app.hold_tick().await;
                        app.emit_level_if_listening();
                    }
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
        });
    }

    {
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            app.watch_wake().await;
        });
    }

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
    loop {
        tokio::select! {
            cmd = commands.recv() => {
                let Some(cmd) = cmd else { break };
                if matches!(cmd, Command::Shutdown) {
                    tracing::info!("shutdown requested");
                    let _ = app.shutdown.send(true);
                    break;
                }
                app.handle(cmd).await;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("SIGINT");
                break;
            }
            _ = sigterm.recv() => {
                tracing::info!("SIGTERM");
                break;
            }
            _ = sighup.recv() => {
                tracing::info!("SIGHUP");
                app.reload().await;
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
    if let Some(handle) = _http_handle {
        handle.abort();
    }
    Ok(())
}

struct App {
    cfg: std::sync::Mutex<Config>,
    apps: Mutex<AppsFile>,
    commands: Mutex<CommandsFile>,
    state: Mutex<StateMachine>,
    events: broadcast::Sender<Event>,
    audio: AudioEngine,
    stt: VoxtypeProvider,
    llm: LlamaCppBackend,
    tts: Arc<KokoroProvider>,
    samples: Mutex<tokio::sync::mpsc::Receiver<Vec<i16>>>,
    aec_samples: Mutex<Option<tokio::sync::mpsc::Receiver<Vec<i16>>>>,
    silero: std::sync::Mutex<Option<Arc<Silero>>>,
    released: watch::Sender<bool>,
    /// Must stay alive: tokio watch `send` is a no-op with zero receivers.
    _released_rx: watch::Receiver<bool>,
    turn_busy: AtomicBool,
    turn_gen: AtomicU64,
    need_keyup: AtomicBool,
    last_hold: std::sync::Mutex<Instant>,
    history: Mutex<Vec<(String, String)>>,
    cancel: Mutex<CancellationToken>,
    barged_in: AtomicBool,
    shutdown: watch::Sender<bool>,
    wake_hush_until: Mutex<Instant>,
}

fn load_silero(cfg: &Config) -> Option<Arc<Silero>> {
    if !cfg.vad.wants_silero() {
        tracing::info!("VAD backend energy");
        return None;
    }
    match Silero::load(&cfg.vad) {
        Ok(s) => {
            tracing::info!("Silero VAD ready");
            Some(Arc::new(s))
        }
        Err(err) => {
            tracing::warn!(%err, "Silero VAD failed; energy fallback");
            None
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl App {
    fn emit(&self, ev: Event) {
        let _ = self.events.send(ev);
    }

    fn cfg(&self) -> Config {
        self.cfg
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn streaming_vad(
        &self,
        hangover_ms: u64,
        confirm: u32,
        energy_threshold: f32,
    ) -> StreamingVad {
        let cfg = self.cfg();
        let silero = self
            .silero
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        StreamingVad::new(
            &cfg.vad,
            silero,
            hangover_ms,
            confirm.max(1),
            energy_threshold,
        )
    }

    fn emit_health(&self) {
        let state = self
            .state
            .try_lock()
            .map(|s| s.current())
            .unwrap_or(AssistantState::Idle);
        self.emit(Event::Health {
            ok: true,
            state,
            name: self.cfg().assistant.name.clone(),
            model: self.llm.model_path().display().to_string(),
            model_loaded: self.llm.is_loaded(),
        });
    }

    fn health_json(&self) -> serde_json::Value {
        let state = self
            .state
            .try_lock()
            .map(|s| s.current())
            .unwrap_or(AssistantState::Idle);
        serde_json::json!({
            "ok": true,
            "state": state,
            "name": self.cfg().assistant.name,
            "model": self.llm.model_path().display().to_string(),
            "model_loaded": self.llm.is_loaded(),
            "silero_loaded": self.silero.lock().unwrap_or_else(|e| e.into_inner()).is_some(),
            "tts_device": self.tts.device(),
            "tts_variant": self.tts.variant(),
            "tts_voice": self.tts.voice(),
        })
    }

    async fn reload(&self) {
        self.reload_apps().await;
        self.reload_commands().await;
        match Config::load() {
            Ok(new) => {
                let mut guard = self.cfg.lock().unwrap_or_else(|e| e.into_inner());
                if new.llm.model != guard.llm.model
                    || new.llm.n_gpu_layers != guard.llm.n_gpu_layers
                    || new.llm.backend != guard.llm.backend
                    || new.tts.device != guard.tts.device
                    || new.tts.provider != guard.tts.provider
                    || new.stt.provider != guard.stt.provider
                {
                    tracing::warn!(
                        "model/STT/TTS device changes need a restart; applying other settings"
                    );
                }
                tracing::info!(prompt = %new.assistant.prompt, "reloaded config.toml");
                let silero = load_silero(&new);
                *guard = new;
                drop(guard);
                *self.silero.lock().unwrap_or_else(|e| e.into_inner()) = silero;
            }
            Err(err) => {
                tracing::error!(%err, "reload config.toml failed; keeping previous config");
                self.emit(Event::Error {
                    message: format!("config.toml: {err}"),
                });
            }
        }
        self.emit_health();
    }

    async fn reload_apps(&self) {
        match AppsFile::load() {
            Ok(apps) => {
                let n = apps.app.len();
                *self.apps.lock().await = apps;
                tracing::info!(n, "reloaded voice apps");
            }
            Err(err) => {
                tracing::error!(%err, "reload apps.toml failed; keeping previous list");
                self.emit(Event::Error {
                    message: format!("apps.toml: {err}"),
                });
            }
        }
    }

    async fn reload_commands(&self) {
        match CommandsFile::load() {
            Ok(cmds) => {
                let n = cmds.command.len();
                *self.commands.lock().await = cmds;
                tracing::info!(n, "reloaded voice commands");
            }
            Err(err) => {
                tracing::error!(%err, "reload commands.toml failed; keeping previous list");
                self.emit(Event::Error {
                    message: format!("commands.toml: {err}"),
                });
            }
        }
    }

    async fn emit_state(&self, next: AssistantState) {
        let changed = {
            let mut sm = self.state.lock().await;
            sm.set(next)
        };
        if changed.is_some() {
            self.emit(Event::State {
                state: next,
                at_ms: now_ms(),
            });
        }
    }

    async fn missed_utterance(&self) {
        self.emit(Event::Error {
            message: "Didn't catch that. Hold Alt+Space and speak.".into(),
        });
        self.emit_state(AssistantState::Idle).await;
    }

    async fn missed_follow_up(&self) {
        self.emit(Event::Error {
            message: "Didn't catch that.".into(),
        });
        self.emit_state(AssistantState::Idle).await;
    }

    async fn handle(self: &Arc<Self>, cmd: Command) {
        match cmd {
            Command::PttStart => self.ptt_start().await,
            Command::PttStop => self.ptt_stop().await,
            Command::Cancel => self.cancel_turn().await,
            Command::Status => self.emit_health(),
            Command::Shutdown => {}
            Command::Say { text } => self.start_text_turn(text).await,
            Command::Demo => self.start_demo().await,
            Command::Reload => self.reload().await,
        }
    }

    async fn start_text_turn(self: &Arc<Self>, text: String) {
        if self.turn_busy.swap(true, Ordering::SeqCst) {
            self.emit(Event::Error {
                message: "busy".into(),
            });
            return;
        }
        let gen = self.turn_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let app = Arc::clone(self);
        tokio::spawn(async move {
            let mut timing = TurnTiming::new();
            timing.mark_vad_end();
            timing.stt_done = Some(Instant::now());
            if let Err(err) = app.respond(&text, timing).await {
                tracing::error!(%err, "say failed");
                app.emit(Event::Error {
                    message: err.to_string(),
                });
                app.emit_state(AssistantState::Error).await;
                app.emit_state(AssistantState::Idle).await;
            }
            app.finish_turn(gen);
        });
    }

    async fn start_demo(self: &Arc<Self>) {
        if self.turn_busy.swap(true, Ordering::SeqCst) {
            self.emit(Event::Error {
                message: "busy".into(),
            });
            return;
        }
        let gen = self.turn_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let app = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(err) = app.run_demo().await {
                tracing::error!(%err, "demo failed");
                app.emit(Event::Error {
                    message: err.to_string(),
                });
                app.emit_state(AssistantState::Error).await;
                app.emit_state(AssistantState::Idle).await;
            }
            app.finish_turn(gen);
        });
    }

    fn touch_hold(&self) {
        if let Ok(mut t) = self.last_hold.lock() {
            *t = Instant::now();
        }
    }

    fn hold_age(&self) -> Duration {
        self.last_hold
            .lock()
            .map(|t| t.elapsed())
            .unwrap_or(Duration::from_secs(1))
    }

    fn emit_level_if_listening(&self) {
        let listening = self
            .state
            .try_lock()
            .map(|s| s.current() == AssistantState::Listening)
            .unwrap_or(false);
        if !listening {
            return;
        }
        let rms = *self.audio.levels().borrow();
        self.emit(Event::Level { rms });
    }

    async fn reset_cancel(&self) -> CancellationToken {
        let mut g = self.cancel.lock().await;
        g.cancel();
        let t = CancellationToken::new();
        *g = t.clone();
        t
    }

    async fn hold_tick(&self) {
        if self.hold_age() <= Duration::from_millis(450) {
            return;
        }
        self.need_keyup.store(false, Ordering::SeqCst);
        let listening = self
            .state
            .try_lock()
            .map(|s| s.current() == AssistantState::Listening)
            .unwrap_or(false);
        if listening && !*self.released.borrow() {
            tracing::info!("PTT key-up (hold ended)");
            let _ = self.released.send(true);
        }
    }

    async fn ptt_start(self: &Arc<Self>) {
        self.touch_hold();
        if self.need_keyup.load(Ordering::SeqCst) {
            return;
        }
        if self.turn_busy.load(Ordering::SeqCst)
            && self
                .state
                .try_lock()
                .map(|s| s.current() == AssistantState::Listening)
                .unwrap_or(false)
        {
            return;
        }
        let cur = self.state.lock().await.current();
        if matches!(cur, AssistantState::Speaking | AssistantState::Thinking) {
            self.cancel_turn().await;
        }
        if self.turn_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let gen = self.turn_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.reset_cancel().await;
        if self.released.send(false).is_err() {
            tracing::error!("PTT watch closed");
            self.finish_turn(gen);
            return;
        }
        if let Err(err) = self.audio.start_capture() {
            tracing::error!(%err, "start capture");
            self.finish_turn(gen);
            return;
        }
        if let Err(err) = self.stt.begin().await {
            tracing::warn!(%err, "parakeet daemon listen; will cold-transcribe if needed");
        }
        self.emit_state(AssistantState::Listening).await;
        let (armed_tx, armed_rx) = tokio::sync::oneshot::channel();
        let app = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(err) = app.run_turn(armed_tx).await {
                tracing::error!(%err, "turn failed");
                app.emit(Event::Error {
                    message: err.to_string(),
                });
                app.emit_state(AssistantState::Error).await;
                app.emit_state(AssistantState::Idle).await;
            }
            app.need_keyup.store(true, Ordering::SeqCst);
            app.finish_turn(gen);
        });
        // Hold PttStop in the IPC queue until record_until is subscribed.
        let _ = armed_rx.await;
    }

    async fn ptt_stop(&self) {
        if self.hold_age() < Duration::from_millis(120) {
            tracing::debug!("ignoring ptt-stop while Alt+Space is still repeating");
            return;
        }
        let _ = self.released.send(true);
    }

    fn finish_turn(&self, gen: u64) {
        if self.turn_gen.load(Ordering::SeqCst) == gen {
            self.turn_busy.store(false, Ordering::SeqCst);
        }
    }

    async fn cancel_turn(&self) {
        let idle = !self.turn_busy.load(Ordering::SeqCst)
            && self
                .state
                .try_lock()
                .map(|s| s.current() == AssistantState::Idle)
                .unwrap_or(false);
        if idle {
            return;
        }
        tracing::info!("cancel");
        self.barged_in.store(false, Ordering::SeqCst);
        self.turn_gen.fetch_add(1, Ordering::SeqCst);
        self.cancel.lock().await.cancel();
        *self.cancel.lock().await = CancellationToken::new();
        let _ = self.audio.cancel_playback();
        let _ = self.audio.stop_capture();
        self.stt.cancel_listen().await;
        let _ = self.released.send(true);
        self.emit_state(AssistantState::Interrupted).await;
        self.emit_state(AssistantState::Idle).await;
        self.turn_busy.store(false, Ordering::SeqCst);
        self.hush_wake(self.cfg().wake_word.hush_ms / 3).await;
    }

    async fn run_turn(self: &Arc<Self>, armed: tokio::sync::oneshot::Sender<()>) -> Result<()> {
        let cfg = self.cfg();
        let fallback_energy = if cfg.vad.threshold <= 0.1 {
            cfg.vad.threshold
        } else {
            0.012
        };
        let mut vad = self.streaming_vad(
            cfg.vad.hangover_ms,
            cfg.vad.start_confirm_frames,
            fallback_energy,
        );
        let stop = self.released.subscribe();
        let _ = armed.send(());
        let (pcm, _by_vad) = {
            let mut samples = self.samples.lock().await;
            record_until(&mut samples, stop, &mut vad, 60).await?
        };
        let _ = self.audio.stop_capture();

        let mut timing = TurnTiming::new();
        timing.mark_vad_end();

        if pcm.len() < 3_200 {
            tracing::info!(samples = pcm.len(), "utterance too short, ignoring");
            self.missed_utterance().await;
            return Ok(());
        }

        self.emit_state(AssistantState::Transcribing).await;
        let cancel = self.cancel.lock().await.clone();
        let tr = self.stt.finish(&pcm, cancel.clone()).await?;
        timing.stt_done = Some(Instant::now());
        tracing::info!(transcript = %tr.text, "heard");
        self.emit(Event::Transcript {
            text: tr.text.clone(),
        });
        if tr.text.trim().is_empty() {
            tracing::info!("empty transcript");
            self.missed_utterance().await;
            return Ok(());
        }
        self.respond(&tr.text, timing).await
    }

    async fn run_demo(self: &Arc<Self>) -> Result<()> {
        const PROMPT: &str = "What is two plus two?";
        tracing::info!("demo: synthesizing probe utterance");
        let tts = Arc::clone(&self.tts);
        let pcm_f32 = tokio::task::spawn_blocking(move || tts.speak(PROMPT))
            .await
            .context("demo tts join")??;
        let pcm = f32_24k_to_i16_16k(&pcm_f32);

        let mut timing = TurnTiming::new();
        timing.mark_vad_end();
        self.emit_state(AssistantState::Transcribing).await;
        let cancel = self.cancel.lock().await.clone();
        let tr = self.stt.transcribe(&pcm, cancel).await?;
        timing.stt_done = Some(Instant::now());
        tracing::info!(transcript = %tr.text, "demo voxtype");
        self.emit(Event::Transcript {
            text: tr.text.clone(),
        });
        if tr.text.trim().is_empty() {
            anyhow::bail!("demo: voxtype returned empty transcript");
        }
        self.respond(&tr.text, timing).await
    }

    fn intent_allowed(&self, tool: &str) -> bool {
        let p = self.cfg().permissions;
        assistant_tools::tool_allowed(
            tool,
            &p.volume,
            &p.launch_app,
            &p.shutdown,
        )
    }

    async fn remember(&self, user: &str, assistant: &str) {
        let mut hist = self.history.lock().await;
        hist.push((user.to_string(), assistant.to_string()));
        const KEEP: usize = 8;
        if hist.len() > KEEP {
            let extra = hist.len() - KEEP;
            hist.drain(..extra);
        }
    }

    async fn respond(self: &Arc<Self>, user_text: &str, mut timing: TurnTiming) -> Result<()> {
        let cfg = self.cfg();
        let ask = strip_wake(
            user_text,
            &cfg.assistant.name,
            &cfg.wake_word.prefix_list(),
            &cfg.wake_word.phrase_list(&cfg.assistant.name),
        )
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| user_text.to_string());
        let user_text = ask.as_str();
        if let Some(spoken) = cfg.clock.spoken_reply(user_text) {
            tracing::info!(intent = "clock", spoken = %spoken, "routed");
            timing.route_done = Some(Instant::now());
            return self.speak_reply(&spoken, timing).await;
        }
        let apps = self.apps.lock().await.app.clone();
        let cmds = self.commands.lock().await.clone();
        let intent = route_with(user_text, &apps, &cmds);
        tracing::info!(?intent, "routed");
        match intent {
            Intent::Forget => {
                timing.route_done = Some(Instant::now());
                self.history.lock().await.clear();
                return self.speak_reply("Okay, starting fresh.", timing).await;
            }
            Intent::Say(spoken) => {
                timing.route_done = Some(Instant::now());
                return self.speak_reply(&spoken, timing).await;
            }
            Intent::Desktop {
                tool,
                spoken,
                omarchy,
            } => {
                timing.route_done = Some(Instant::now());
                if !self.intent_allowed(&tool) {
                    return self.speak_reply("I can't do that.", timing).await;
                }
                return self.run_desktop(&tool, &spoken, omarchy, timing).await;
            }
            Intent::AgentHandoff {
                agent,
                prompt,
                spoken,
            } => {
                timing.route_done = Some(Instant::now());
                if !cfg.agent.enabled {
                    return self.speak_reply("Coding agent handoff is disabled.", timing).await;
                }
                let target_agent = if agent.is_empty() {
                    &cfg.agent.default_agent
                } else {
                    &agent
                };
                if let Err(err) = assistant_tools::agent::launch_coding_agent(
                    target_agent,
                    &prompt,
                    &cfg.agent.terminal,
                )
                .await
                {
                    tracing::error!(%err, "failed to launch coding agent");
                    return self.speak_reply("Failed to launch coding agent.", timing).await;
                }
                return self.speak_reply(&spoken, timing).await;
            }
            Intent::Chat => {}
        }

        if is_filler(user_text) && self.history.lock().await.is_empty() {
            tracing::info!(transcript = %user_text, "ignoring filler");
            self.emit_state(AssistantState::Idle).await;
            return Ok(());
        }

        timing.route_done = Some(Instant::now());
        self.emit_state(AssistantState::Thinking).await;

        let history = self.history.lock().await.clone();
        let cancel = self.cancel.lock().await.clone();
        let gen_cancel = Arc::new(AtomicBool::new(false));
        let gc = gen_cancel.clone();
        let cancel_watch = cancel.clone();
        let cancel_task = tokio::spawn(async move {
            cancel_watch.cancelled().await;
            gc.store(true, Ordering::Relaxed);
        });

        let mut tokens = self
            .llm
            .generate(
                &cfg.system_prompt(),
                &history,
                user_text,
                cfg.llm.n_predict,
                cfg.llm.temperature,
                gen_cancel,
            )
            .await?;

        let mut chunker = PhraseChunker::new();
        let mut first_llm = true;
        let mut spoken = String::new();
        let tts = Arc::clone(&self.tts);
        let barge = self.spawn_barge_in(cancel.clone());
        let timing = Arc::new(Mutex::new(timing));
        let first_audio = Arc::new(AtomicBool::new(true));
        let (phrase_tx, mut phrase_rx) = mpsc::channel::<String>(8);
        let tts_app = Arc::clone(self);
        let tts_w = Arc::clone(&tts);
        let timing_w = Arc::clone(&timing);
        let first_w = Arc::clone(&first_audio);
        let tts_cancel = cancel.clone();
        let tts_worker = tokio::spawn(async move {
            while let Some(phrase) = phrase_rx.recv().await {
                if tts_cancel.is_cancelled() {
                    break;
                }
                tts_app
                    .speak_phrase(&tts_w, &phrase, &timing_w, &first_w)
                    .await?;
            }
            Ok::<(), anyhow::Error>(())
        });

        while let Some(tok) = tokens.recv().await {
            if cancel.is_cancelled() {
                break;
            }
            if first_llm {
                timing.lock().await.llm_first = Some(Instant::now());
                first_llm = false;
            }
            spoken.push_str(&tok);
            self.emit(Event::AssistantText {
                text: spoken.clone(),
                done: false,
            });
            if let Some(phrase) = chunker.push(&tok) {
                if phrase_tx.send(phrase).await.is_err() {
                    break;
                }
            }
        }
        if !cancel.is_cancelled() {
            if let Some(phrase) = chunker.flush() {
                let _ = phrase_tx.send(phrase).await;
            }
        }
        drop(phrase_tx);
        if cancel.is_cancelled() {
            tts_worker.abort();
            let _ = self.audio.cancel_playback();
        } else {
            tts_worker.await.context("tts worker join")??;
            self.audio.wait_playback().await;
        }
        barge.abort();
        cancel_task.abort();
        tracing::info!(reply = %spoken, "spoken reply");
        if !cancel.is_cancelled() && !spoken.trim().is_empty() {
            self.remember(user_text, &spoken).await;
        }
        self.emit(Event::AssistantText {
            text: spoken,
            done: true,
        });
        let timing = timing.lock().await;
        tracing::info!("{}", timing.format_trace());
        self.emit(Event::from_timing(&timing));
        drop(timing);
        self.finish_speech(&cancel).await
    }

    async fn run_desktop(
        self: &Arc<Self>,
        tool: &str,
        spoken: &str,
        omarchy: Vec<String>,
        timing: TurnTiming,
    ) -> Result<()> {
        tracing::info!(tool, "routed desktop command");
        self.emit_state(AssistantState::Executing).await;
        let args: Vec<&str> = omarchy.iter().map(String::as_str).collect();
        let result = if args.first() == Some(&"hyprctl") {
            assistant_omarchy::hyprctl(&args[1..]).await
        } else {
            assistant_omarchy::omarchy(&args).await
        };
        if let Err(err) = result {
            tracing::error!(%err, tool, "desktop command failed");
            self.emit(Event::Error {
                message: err.to_string(),
            });
            return self.speak_reply("That failed.", timing).await;
        }
        self.remember(&format!("(command {tool})"), spoken).await;
        self.speak_reply(spoken, timing).await
    }

    async fn speak_reply(
        self: &Arc<Self>,
        spoken: &str,
        timing: TurnTiming,
    ) -> Result<()> {
        self.emit(Event::AssistantText {
            text: spoken.to_string(),
            done: false,
        });
        let tts = Arc::clone(&self.tts);
        let first_audio = AtomicBool::new(true);
        let cancel = self.cancel.lock().await.clone();
        let barge = self.spawn_barge_in(cancel.clone());
        let timing = Mutex::new(timing);
        self.speak_phrase(&tts, spoken, &timing, &first_audio)
            .await?;
        if cancel.is_cancelled() {
            let _ = self.audio.cancel_playback();
        } else {
            self.audio.wait_playback().await;
        }
        barge.abort();
        self.emit(Event::AssistantText {
            text: spoken.to_string(),
            done: true,
        });
        let timing = timing.into_inner();
        tracing::info!("{}", timing.format_trace());
        self.emit(Event::from_timing(&timing));
        self.finish_speech(&cancel).await
    }

    async fn speak_phrase(
        &self,
        tts: &Arc<KokoroProvider>,
        phrase: &str,
        timing: &Mutex<TurnTiming>,
        first_audio: &AtomicBool,
    ) -> Result<()> {
        if self.cancel.lock().await.is_cancelled() {
            return Ok(());
        }
        let phrase = phrase.to_string();
        let tts = Arc::clone(tts);
        let pcm = tokio::task::spawn_blocking(move || tts.speak(&phrase))
            .await
            .context("tts join")??;
        if pcm.is_empty() {
            return Ok(());
        }
        if self.cancel.lock().await.is_cancelled() {
            return Ok(());
        }
        if first_audio.swap(false, Ordering::SeqCst) {
            timing.lock().await.tts_begin = Some(Instant::now());
            self.emit_state(AssistantState::Speaking).await;
        }
        {
            let mut timing = timing.lock().await;
            if timing.audio_play.is_none() {
                timing.audio_play = Some(Instant::now());
            }
        }
        self.audio.play(pcm)?;
        Ok(())
    }

    fn spawn_barge_in(self: &Arc<Self>, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
        let app = Arc::clone(self);
        tokio::spawn(async move {
            app.watch_barge_in(cancel).await;
        })
    }

    async fn watch_barge_in(&self, cancel: CancellationToken) {
        let barge = self.cfg().barge_in;
        if !barge.enabled {
            return;
        }
        let _ = self.audio.start_capture();
        let aec = self.audio.aec_levels().is_some();
        let (mut levels, configured) = match self.audio.aec_levels() {
            Some(rx) => (rx, barge.threshold.min(0.04)),
            None => (self.audio.levels(), barge.threshold),
        };
        let grace_end =
            tokio::time::Instant::now() + Duration::from_millis(barge.grace_ms.max(1));
        let mut residual = 0.0f32;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep_until(grace_end) => break,
                changed = levels.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    residual = residual.max(*levels.borrow());
                }
            }
        }
        let threshold = if aec {
            aec_barge_threshold(residual, configured)
        } else {
            configured
        };
        tracing::info!(residual, threshold, aec, "barge-in armed");
        if self.cfg().vad.wants_silero()
            && self
                .silero
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
        {
            self.watch_barge_in_silero(cancel, barge.hangover_ms, threshold).await;
            return;
        }
        let need = (barge.hangover_ms / 32).max(1) as u32;
        let mut loud = 0u32;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                changed = levels.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let rms = *levels.borrow();
                    if rms >= threshold {
                        loud = loud.saturating_add(1);
                        if loud >= need {
                            tracing::info!(rms, "barge-in");
                            self.trip_barge_in(&cancel).await;
                            return;
                        }
                    } else {
                        loud = 0;
                    }
                }
            }
        }
    }

    async fn watch_barge_in_silero(&self, cancel: CancellationToken, hangover_ms: u64, min_rms: f32) {
        let mut vad = self.streaming_vad(hangover_ms, self.cfg().vad.start_confirm_frames, 0.08);
        let mut aec = self.aec_samples.lock().await;
        let Some(rx) = aec.as_mut() else {
            tracing::warn!("no AEC pcm; barge-in silero skipped");
            return;
        };
        while rx.try_recv().is_ok() {}
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                chunk = rx.recv() => {
                    let Some(chunk) = chunk else { return };
                    if matches!(vad.push(&chunk), VadDecision::SpeechStart) {
                        let mut acc = 0.0f32;
                        for s in &chunk {
                            let x = *s as f32 / 32768.0;
                            acc += x * x;
                        }
                        let rms = (acc / chunk.len().max(1) as f32).sqrt();
                        if rms >= min_rms {
                            tracing::info!(rms, min_rms, "barge-in silero");
                            drop(aec);
                            self.trip_barge_in(&cancel).await;
                            return;
                        }
                    }
                }
            }
        }
    }

    async fn trip_barge_in(&self, cancel: &CancellationToken) {
        self.barged_in.store(true, Ordering::SeqCst);
        let _ = self.audio.cancel_playback();
        cancel.cancel();
        if let Err(err) = self.stt.begin().await {
            tracing::warn!(%err, "barge-in parakeet listen");
        }
        self.emit_state(AssistantState::Interrupted).await;
    }

    async fn finish_speech(self: &Arc<Self>, cancel: &CancellationToken) -> Result<()> {
        if cancel.is_cancelled() {
            let taken_over = {
                let s = self.state.lock().await.current();
                matches!(
                    s,
                    AssistantState::Listening | AssistantState::Transcribing
                )
            };
            if !taken_over && self.barged_in.swap(false, Ordering::SeqCst) {
                return self.follow_up_listen().await;
            }
            self.barged_in.store(false, Ordering::SeqCst);
            return Ok(());
        }
        let _ = self.audio.stop_capture();
        self.discard_pending_samples().await;
        self.hush_wake(self.cfg().wake_word.hush_ms).await;
        self.emit_state(AssistantState::Idle).await;
        Ok(())
    }

    async fn hush_wake(&self, ms: u64) {
        let d = Duration::from_millis(ms.max(1));
        let until = Instant::now() + d;
        if let Ok(mut g) = self.wake_hush_until.try_lock() {
            if until > *g {
                *g = until;
            }
        } else {
            *self.wake_hush_until.lock().await = until;
        }
    }

    fn wake_hushed(&self) -> bool {
        self.wake_hush_until
            .try_lock()
            .map(|g| Instant::now() < *g)
            .unwrap_or(true)
    }

    async fn discard_pending_samples(&self) {
        let mut samples = self.samples.lock().await;
        while samples.try_recv().is_ok() {}
    }

    async fn watch_wake(self: Arc<Self>) {
        let mut shutdown = self.shutdown.subscribe();
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        return;
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
            if !self.cfg().wake_word.enabled {
                if !self.turn_busy.load(Ordering::SeqCst)
                    && self
                        .state
                        .try_lock()
                        .map(|s| s.current() == AssistantState::Idle)
                        .unwrap_or(false)
                {
                    let _ = self.audio.stop_capture();
                }
                continue;
            }
            if self.turn_busy.load(Ordering::SeqCst) {
                continue;
            }
            if !self
                .state
                .try_lock()
                .map(|s| s.current() == AssistantState::Idle)
                .unwrap_or(false)
            {
                continue;
            }
            let _ = self.audio.start_capture();
            if self.wake_hushed() {
                self.discard_pending_samples().await;
                continue;
            }
            let Some(pcm) = self.collect_wake_pcm().await else {
                continue;
            };
            if self.turn_busy.swap(true, Ordering::SeqCst) {
                continue;
            }
            let gen = self.turn_gen.fetch_add(1, Ordering::SeqCst) + 1;
            let app = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(err) = app.run_wake(pcm).await {
                    tracing::warn!(%err, "wake");
                    app.emit_state(AssistantState::Idle).await;
                }
                app.finish_turn(gen);
            });
        }
    }

    async fn collect_wake_pcm(&self) -> Option<Vec<i16>> {
        let w = self.cfg().wake_word;
        let rate = self.cfg().audio.capture_rate.max(1) as usize;
        let fallback_energy = if w.threshold <= 0.1 {
            w.threshold
        } else {
            0.012
        };
        let mut vad = self.streaming_vad(
            w.hangover_ms,
            w.start_confirm_frames,
            fallback_energy,
        );
        let mut preroll = Vec::<i16>::new();
        let mut pcm = Vec::<i16>::new();
        let mut in_utt = false;
        let mut began = false;
        let wait_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut utt_deadline = tokio::time::Instant::now();
        let preroll_n = rate * w.preroll_ms as usize / 1000;
        let min_n = rate * w.min_ms as usize / 1000;
        let max_n = rate * w.max_ms.max(w.min_ms) as usize / 1000;
        loop {
            if self.turn_busy.load(Ordering::SeqCst) || self.wake_hushed() {
                if began && !self.turn_busy.load(Ordering::SeqCst) {
                    self.stt.cancel_listen().await;
                }
                return None;
            }
            if !self.cfg().wake_word.enabled {
                if began && !self.turn_busy.load(Ordering::SeqCst) {
                    self.stt.cancel_listen().await;
                }
                return None;
            }
            let chunk = {
                let mut samples = self.samples.lock().await;
                match tokio::time::timeout(Duration::from_millis(50), samples.recv()).await {
                    Ok(Some(c)) => c,
                    Ok(None) => return None,
                    Err(_) => {
                        if in_utt && tokio::time::Instant::now() > utt_deadline {
                            break;
                        }
                        if !in_utt && tokio::time::Instant::now() > wait_deadline {
                            return None;
                        }
                        continue;
                    }
                }
            };
            if self.turn_busy.load(Ordering::SeqCst) || self.wake_hushed() {
                if began && !self.turn_busy.load(Ordering::SeqCst) {
                    self.stt.cancel_listen().await;
                }
                return None;
            }
            let d = vad.push(&chunk);
            match d {
                VadDecision::SpeechStart => {
                    in_utt = true;
                    utt_deadline =
                        tokio::time::Instant::now() + Duration::from_millis(w.max_ms.max(1));
                    pcm.extend_from_slice(&preroll);
                    pcm.extend_from_slice(&chunk);
                    preroll.clear();
                    if self.stt.begin().await.is_ok() {
                        began = true;
                    }
                }
                VadDecision::Speech | VadDecision::SpeechEnd if in_utt => {
                    pcm.extend_from_slice(&chunk);
                    if pcm.len() > max_n
                        || matches!(d, VadDecision::SpeechEnd)
                        || tokio::time::Instant::now() > utt_deadline
                    {
                        break;
                    }
                }
                _ => {
                    preroll.extend_from_slice(&chunk);
                    if preroll.len() > preroll_n {
                        let extra = preroll.len() - preroll_n;
                        preroll.drain(..extra);
                    }
                    if tokio::time::Instant::now() > wait_deadline {
                        return None;
                    }
                }
            }
        }
        if pcm.len() < min_n {
            if began && !self.turn_busy.load(Ordering::SeqCst) {
                self.stt.cancel_listen().await;
            }
            return None;
        }
        Some(pcm)
    }

    async fn run_wake(self: &Arc<Self>, pcm: Vec<i16>) -> Result<()> {
        let cancel = self.reset_cancel().await;
        let cfg = self.cfg();
        let name = cfg.assistant.name.clone();
        let prefixes = cfg.wake_word.prefix_list();
        let phrases = cfg.wake_word.phrase_list(&name);
        let min_n = cfg.audio.capture_rate.max(1) as usize * cfg.wake_word.min_ms as usize / 1000;
        let mut tr = self.stt.finish(&pcm, cancel.clone()).await?;
        let mut rest = strip_wake(&tr.text, &name, &prefixes, &phrases);
        if rest.is_none() && tr.text.trim().is_empty() && pcm.len() >= min_n {
            tracing::info!("wake empty; retry pcm");
            if let Ok(t2) = self.stt.transcribe(&pcm, cancel).await {
                rest = strip_wake(&t2.text, &name, &prefixes, &phrases);
                tr = t2;
            }
        }
        let Some(rest) = rest else {
            tracing::info!(transcript = %tr.text, "wake ignored");
            self.hush_wake(cfg.wake_word.hush_ms / 3).await;
            return Ok(());
        };
        tracing::info!(transcript = %tr.text, rest = %rest, "wake");
        self.emit(Event::Transcript {
            text: tr.text.clone(),
        });
        if rest.trim().is_empty() {
            if let Err(err) = self.stt.begin().await {
                tracing::warn!(%err, "wake follow-up parakeet listen");
            }
            return self.follow_up_listen().await;
        }
        let mut timing = TurnTiming::new();
        timing.mark_vad_end();
        timing.stt_done = Some(Instant::now());
        self.respond(&rest, timing).await
    }

    async fn follow_up_listen(self: &Arc<Self>) -> Result<()> {
        tracing::info!("wake follow-up listen");
        self.emit_state(AssistantState::Listening).await;
        let _ = self.audio.start_capture();
        let cfg = self.cfg();
        let fallback_energy = if cfg.wake_word.threshold.min(cfg.vad.threshold) <= 0.1 {
            cfg.wake_word.threshold.min(cfg.vad.threshold)
        } else {
            0.012
        };
        let mut vad = self.streaming_vad(
            cfg.wake_word.hangover_ms.max(cfg.vad.hangover_ms),
            cfg.vad.start_confirm_frames.max(1),
            fallback_energy,
        );
        let cancel = self.reset_cancel().await;
        let (stop_tx, stop_rx) = watch::channel(false);
        let watch_cancel = cancel.clone();
        let watch_task = tokio::spawn(async move {
            watch_cancel.cancelled().await;
            let _ = stop_tx.send(true);
        });
        let (pcm, _ended) = {
            let mut samples = self.samples.lock().await;
            record_until_vad(&mut samples, stop_rx, &mut vad, 8, true).await?
        };
        watch_task.abort();
        let _ = self.audio.stop_capture();
        if cancel.is_cancelled() {
            return Ok(());
        }
        let min_n = cfg.audio.capture_rate.max(1) as usize * cfg.wake_word.min_ms as usize / 1000;
        if pcm.len() < min_n.max(1) {
            tracing::info!(samples = pcm.len(), "wake follow-up too short");
            self.missed_follow_up().await;
            return Ok(());
        }
        self.emit_state(AssistantState::Transcribing).await;
        let tr = self.stt.finish(&pcm, cancel).await?;
        tracing::info!(transcript = %tr.text, "heard barge-in");
        self.emit(Event::Transcript {
            text: tr.text.clone(),
        });
        if tr.text.trim().is_empty() {
            tracing::info!("empty barge-in transcript");
            self.missed_follow_up().await;
            return Ok(());
        }
        let mut timing = TurnTiming::new();
        timing.mark_vad_end();
        timing.stt_done = Some(Instant::now());
        Box::pin(self.respond(&tr.text, timing)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn silero_detects_speech_audio() {
        let cfg = Config::default();
        let silero = match load_silero(&cfg) {
            Some(s) => s,
            None => return,
        };
        let tts = KokoroProvider::load(&cfg).expect("load kokoro");
        let pcm_f32 = tts.speak("Hey Nyx, what time is it?").expect("synthesize");
        let pcm_16k = f32_24k_to_i16_16k(&pcm_f32);
        let mut gate = assistant_audio::StreamingVad::new(
            &cfg.vad,
            Some(silero.clone()),
            cfg.wake_word.hangover_ms,
            cfg.wake_word.start_confirm_frames,
            cfg.wake_word.threshold,
        );
        let mut decisions = Vec::new();
        for chunk in pcm_16k.chunks(512) {
            let dec = gate.push(chunk);
            decisions.push(dec);
        }
        let speech_starts = decisions.iter().filter(|d| matches!(d, VadDecision::SpeechStart)).count();
        assert!(speech_starts > 0, "Silero should detect speech start in synthesized speech");
    }
}
