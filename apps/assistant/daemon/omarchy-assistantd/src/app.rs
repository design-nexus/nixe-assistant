use anyhow::{Context, Result};
use assistant_agent::{route, Intent, KokoroProvider, PhraseChunker, VoxtypeProvider};
use assistant_audio::{f32_24k_to_i16_16k, record_until, AudioEngine, EnergyVad};
use assistant_core::config::Config;
use assistant_core::event::Event;
use assistant_core::state::{AssistantState, StateMachine};
use assistant_core::timing::TurnTiming;
use assistant_inference::LlamaCppBackend;
use assistant_ipc::{Command, Ipc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, Mutex, watch};
use tokio_util::sync::CancellationToken;

pub async fn run(cfg: Config) -> Result<()> {
    let ipc = assistant_ipc::bind().await.context("ipc bind")?;
    let Ipc {
        mut commands,
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
    let (release_tx, release_rx) = watch::channel(true);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let app = Arc::new(App {
        cfg,
        state: Mutex::new(StateMachine::new()),
        events,
        audio,
        stt,
        llm,
        tts: Arc::new(tts),
        samples: Mutex::new(samples),
        released: release_tx,
        _released_rx: release_rx,
        turn_busy: AtomicBool::new(false),
        need_keyup: AtomicBool::new(false),
        last_hold: std::sync::Mutex::new(Instant::now()),
        history: Mutex::new(Vec::new()),
        cancel: Mutex::new(CancellationToken::new()),
        shutdown: shutdown_tx,
    });

    if let Err(err) = assistant_omarchy::unmute_default_source().await {
        tracing::warn!(%err, "could not unmute default mic");
    }

    app.emit_state(AssistantState::Idle).await;
    app.emit_health();

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
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
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                app.hold_tick().await;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("SIGINT");
                break;
            }
            _ = sigterm.recv() => {
                tracing::info!("SIGTERM");
                break;
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
    Ok(())
}

struct App {
    cfg: Config,
    state: Mutex<StateMachine>,
    events: broadcast::Sender<Event>,
    audio: AudioEngine,
    stt: VoxtypeProvider,
    llm: LlamaCppBackend,
    tts: Arc<KokoroProvider>,
    samples: Mutex<tokio::sync::mpsc::Receiver<Vec<i16>>>,
    released: watch::Sender<bool>,
    /// Must stay alive: tokio watch `send` is a no-op with zero receivers.
    _released_rx: watch::Receiver<bool>,
    turn_busy: AtomicBool,
    need_keyup: AtomicBool,
    last_hold: std::sync::Mutex<Instant>,
    history: Mutex<Vec<(String, String)>>,
    cancel: Mutex<CancellationToken>,
    shutdown: watch::Sender<bool>,
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

    fn emit_health(&self) {
        let state = self
            .state
            .try_lock()
            .map(|s| s.current())
            .unwrap_or(AssistantState::Idle);
        self.emit(Event::Health {
            ok: true,
            state,
            name: self.cfg.assistant.name.clone(),
            model: self.llm.model_path().display().to_string(),
            model_loaded: self.llm.is_loaded(),
        });
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

    async fn handle(self: &Arc<Self>, cmd: Command) {
        match cmd {
            Command::PttStart => self.ptt_start().await,
            Command::PttStop => self.ptt_stop().await,
            Command::Cancel => self.cancel_turn().await,
            Command::Status => self.emit_health(),
            Command::Shutdown => {}
            Command::Say { text } => self.start_text_turn(text).await,
            Command::Demo => self.start_demo().await,
        }
    }

    async fn start_text_turn(self: &Arc<Self>, text: String) {
        if self.turn_busy.swap(true, Ordering::SeqCst) {
            self.emit(Event::Error {
                message: "busy".into(),
            });
            return;
        }
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
            app.turn_busy.store(false, Ordering::SeqCst);
        });
    }

    async fn start_demo(self: &Arc<Self>) {
        if self.turn_busy.swap(true, Ordering::SeqCst) {
            self.emit(Event::Error {
                message: "busy".into(),
            });
            return;
        }
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
            app.turn_busy.store(false, Ordering::SeqCst);
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
        if listening {
            tracing::info!("PTT key-up (hold ended)");
            let _ = self.released.send(true);
        }
    }

    async fn ptt_start(self: &Arc<Self>) {
        self.touch_hold();
        if self.need_keyup.load(Ordering::SeqCst) {
            return;
        }
        let cur = self.state.lock().await.current();
        if matches!(cur, AssistantState::Speaking | AssistantState::Thinking) {
            self.cancel_turn().await;
        }
        if self.turn_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.released.send(false).is_err() {
            tracing::error!("PTT watch closed");
            self.turn_busy.store(false, Ordering::SeqCst);
            return;
        }
        if let Err(err) = self.audio.start_capture() {
            tracing::error!(%err, "start capture");
            self.turn_busy.store(false, Ordering::SeqCst);
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
            app.turn_busy.store(false, Ordering::SeqCst);
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

    async fn cancel_turn(&self) {
        self.cancel.lock().await.cancel();
        *self.cancel.lock().await = CancellationToken::new();
        let _ = self.audio.cancel_playback();
        let _ = self.audio.stop_capture();
        self.stt.cancel_listen().await;
        let _ = self.released.send(true);
        self.emit_state(AssistantState::Interrupted).await;
        self.emit_state(AssistantState::Idle).await;
        self.turn_busy.store(false, Ordering::SeqCst);
    }

    async fn run_turn(self: &Arc<Self>, armed: tokio::sync::oneshot::Sender<()>) -> Result<()> {
        let mut vad = EnergyVad::new(self.cfg.vad.threshold, self.cfg.vad.hangover_ms, 32);
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
            self.emit(Event::Error {
                message: "Didn't catch that. Hold Alt+Space and speak.".into(),
            });
            self.emit_state(AssistantState::Idle).await;
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
            self.emit_state(AssistantState::Idle).await;
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
        match tool {
            "audio.volume_down" | "audio.volume_up" | "audio.mute" => {
                self.cfg.permissions.volume == "allow"
            }
            "app.launch" => self.cfg.permissions.launch_app == "allow",
            "system.lock"
            | "capture.screenshot"
            | "toggle.nightlight"
            | "display.brightness_up"
            | "display.brightness_down"
            | "theme.bg_next"
            | "hypr.workspace"
            | "bluetooth.toggle"
            | "toggle.dnd"
            | "toggle.idle" => true,
            _ => false,
        }
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
        let intent = route(user_text);
        tracing::info!(?intent, "routed");
        match intent {
            Intent::Forget => {
                timing.route_done = Some(Instant::now());
                self.history.lock().await.clear();
                return self.speak_reply("Okay, starting fresh.", timing).await;
            }
            Intent::Say(spoken) => {
                timing.route_done = Some(Instant::now());
                return self.speak_reply(spoken, timing).await;
            }
            Intent::Desktop {
                tool,
                spoken,
                omarchy,
            } => {
                timing.route_done = Some(Instant::now());
                if !self.intent_allowed(tool) {
                    return self.speak_reply("I can't do that.", timing).await;
                }
                return self.run_desktop(tool, spoken, omarchy, timing).await;
            }
            Intent::Chat => {}
        }

        timing.route_done = Some(Instant::now());
        self.emit_state(AssistantState::Thinking).await;

        let history = self.history.lock().await.clone();
        let cancel = self.cancel.lock().await.clone();
        let gen_cancel = Arc::new(AtomicBool::new(false));
        let gc = gen_cancel.clone();
        let cancel_watch = cancel.clone();
        tokio::spawn(async move {
            cancel_watch.cancelled().await;
            gc.store(true, Ordering::Relaxed);
        });

        let mut tokens = self
            .llm
            .generate(
                &self.cfg.system_prompt(),
                &history,
                user_text,
                self.cfg.llm.n_predict,
                self.cfg.llm.temperature,
                gen_cancel,
            )
            .await?;

        let mut chunker = PhraseChunker::new();
        let mut first_llm = true;
        let mut first_audio = true;
        let mut spoken = String::new();
        let tts = Arc::clone(&self.tts);

        while let Some(tok) = tokens.recv().await {
            if cancel.is_cancelled() {
                break;
            }
            if first_llm {
                timing.llm_first = Some(Instant::now());
                first_llm = false;
            }
            spoken.push_str(&tok);
            self.emit(Event::AssistantText {
                text: spoken.clone(),
                done: false,
            });
            if let Some(phrase) = chunker.push(&tok) {
                self.speak_phrase(&tts, &phrase, &mut timing, &mut first_audio)
                    .await?;
            }
        }
        if let Some(phrase) = chunker.flush() {
            self.speak_phrase(&tts, &phrase, &mut timing, &mut first_audio)
                .await?;
        }

        self.audio.wait_playback().await;
        tracing::info!(reply = %spoken, "spoken reply");
        if !cancel.is_cancelled() && !spoken.trim().is_empty() {
            self.remember(user_text, &spoken).await;
        }
        self.emit(Event::AssistantText {
            text: spoken,
            done: true,
        });
        tracing::info!("{}", timing.format_trace());
        self.emit(Event::from_timing(&timing));
        self.emit_state(AssistantState::Idle).await;
        Ok(())
    }

    async fn run_desktop(
        self: &Arc<Self>,
        tool: &str,
        spoken: &str,
        omarchy: &[&str],
        timing: TurnTiming,
    ) -> Result<()> {
        tracing::info!(tool, "routed desktop command");
        self.emit_state(AssistantState::Executing).await;
        let result = if omarchy.first() == Some(&"hyprctl") {
            assistant_omarchy::hyprctl(&omarchy[1..]).await
        } else {
            assistant_omarchy::omarchy(omarchy).await
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
        mut timing: TurnTiming,
    ) -> Result<()> {
        self.emit(Event::AssistantText {
            text: spoken.to_string(),
            done: false,
        });
        let tts = Arc::clone(&self.tts);
        let mut first_audio = true;
        self.speak_phrase(&tts, spoken, &mut timing, &mut first_audio)
            .await?;
        self.audio.wait_playback().await;
        self.emit(Event::AssistantText {
            text: spoken.to_string(),
            done: true,
        });
        tracing::info!("{}", timing.format_trace());
        self.emit(Event::from_timing(&timing));
        self.emit_state(AssistantState::Idle).await;
        Ok(())
    }

    async fn speak_phrase(
        &self,
        tts: &Arc<KokoroProvider>,
        phrase: &str,
        timing: &mut TurnTiming,
        first_audio: &mut bool,
    ) -> Result<()> {
        let phrase = phrase.to_string();
        let tts = Arc::clone(tts);
        let pcm = tokio::task::spawn_blocking(move || tts.speak(&phrase))
            .await
            .context("tts join")??;
        if pcm.is_empty() {
            return Ok(());
        }
        if *first_audio {
            timing.tts_begin = Some(Instant::now());
            self.emit_state(AssistantState::Speaking).await;
            *first_audio = false;
        }
        if timing.audio_play.is_none() {
            timing.audio_play = Some(Instant::now());
        }
        self.audio.play(pcm)?;
        Ok(())
    }
}
