use super::*;
use crate::session::{dictation_control, punctuation};
impl App {
    pub(super) async fn locate_for_turn(&self) -> Result<assistant_core::location::Location> {
        let cfg = self.cfg();
        anyhow::ensure!(cfg.location.enabled, "location is disabled");
        let cancel = self.cancel.lock().await.clone();
        let result = self
            .registry
            .call_allowed("location.current", json!({}), &cfg, &cancel)
            .await?;
        let location = serde_json::from_value(result["structuredContent"].clone())?;
        Ok(location)
    }
    pub(super) async fn location_control(
        &self,
        action: &str,
        city: Option<String>,
    ) -> Result<Value> {
        if matches!(action, "show" | "refresh") {
            let result = self
                .registry
                .call_allowed(
                    "location.current",
                    json!({}),
                    &self.cfg(),
                    &CancellationToken::new(),
                )
                .await?;
            return Ok(result["structuredContent"].clone());
        }
        let _guard = self.reload_lock.lock().await;
        let mut saved = Config::load()?;
        match action {
            "set" => {
                saved.location.city = city.context("location city required")?.trim().into();
                anyhow::ensure!(!saved.location.city.is_empty(), "location city required");
                saved.location.latitude = None;
                saved.location.longitude = None;
                saved.location.enabled = true;
            }
            "clear" => {
                saved.location.city.clear();
                saved.location.latitude = None;
                saved.location.longitude = None;
            }
            "off" => saved.location.enabled = false,
            "on" => saved.location.enabled = true,
            _ => anyhow::bail!("location action must be show, refresh, set, clear, on, or off"),
        }
        saved.save()?;
        self.cfg.lock().unwrap().location = saved.location.clone();
        if let Some(plugins) = &self.registry.plugins {
            let action = if saved.location.enabled {
                "enable"
            } else {
                "disable"
            };
            plugins.change(action, "location", &self.cfg()).await?;
            if saved.location.enabled {
                plugins.change("reload", "location", &self.cfg()).await?;
            }
        }
        Ok(json!({"settings":saved.location}))
    }
    pub(super) async fn resume_browser(self: &Arc<Self>) -> Result<Value> {
        let result = self
            .dispatch_plugin("browser.resume", json!({}), &CancellationToken::new())
            .await?;
        if result["structuredContent"]["resumed"] == true {
            let pending = self.pending_browser.lock().unwrap().take();
            if let Some(text) = pending {
                let app = self.clone();
                tokio::spawn(async move {
                    for _ in 0..100 {
                        if !app.turn_busy.load(Ordering::SeqCst)
                            || app.state.lock().await.current() == AssistantState::Listening
                        {
                            let _ = app.command_tx.send(Command::Say { text }).await;
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    *app.pending_browser.lock().unwrap() = Some(text);
                });
            }
        }
        Ok(result)
    }
    pub(super) async fn take_over_session(&self) {
        let mode = self.session.lock().unwrap().mode;
        // Release the state guard before cancellation acquires it again.
        let listening = self.state.lock().await.current() == AssistantState::Listening;
        if matches!(mode, Mode::FollowUp | Mode::Conversation) && listening {
            self.cancel_turn().await;
            drop(self.samples.lock().await);
            if mode == Mode::Conversation {
                self.session
                    .lock()
                    .unwrap()
                    .begin(mode, self.cfg().conversation.idle_secs);
            }
        }
    }
    pub(super) fn session_json(&self) -> Value {
        self.session.lock().unwrap().json()
    }
    pub(super) fn emit_session(&self) {
        let session = self.session.lock().unwrap();
        self.emit(Event::Session {
            session_id: session.id,
            mode: session.mode.name().into(),
            remaining_secs: session.remaining(),
            target: session.target["title"].as_str().unwrap_or("").into(),
            draft: session.draft.clone(),
        });
    }
    pub(super) async fn session_control(&self, kind: &str, action: &str) -> Result<Value> {
        if kind == "conversation" {
            self.cancel_turn().await;
            if action == "start" {
                anyhow::ensure!(self.cfg().conversation.enabled, "conversation is disabled");
                self.session
                    .lock()
                    .unwrap()
                    .begin(Mode::Conversation, self.cfg().conversation.idle_secs);
            } else {
                self.session.lock().unwrap().stop();
            }
        } else {
            anyhow::ensure!(self.cfg().dictation.enabled, "dictation is disabled");
            if action == "start" {
                let target = active_window().await?;
                anyhow::ensure!(
                    target["address"].as_str().is_some_and(|s| !s.is_empty()),
                    "select an application before starting dictation"
                );
                self.cancel_turn().await;
                let mut s = self.session.lock().unwrap();
                s.draft.clear();
                s.target = target;
                s.begin(Mode::Dictation, self.cfg().dictation.idle_secs);
            } else if action == "stop" {
                self.cancel_turn().await;
            } else if action == "pause" {
                self.session
                    .lock()
                    .unwrap()
                    .begin(Mode::Paused, self.cfg().dictation.idle_secs);
            } else if action == "resume" {
                let target = active_window().await?;
                let mut s = self.session.lock().unwrap();
                anyhow::ensure!(s.mode == Mode::Paused, "dictation is not paused");
                s.target = target;
                s.begin(Mode::Dictation, self.cfg().dictation.idle_secs);
            } else if action == "insert" {
                let (target, draft) = {
                    let s = self.session.lock().unwrap();
                    (s.target.clone(), s.draft.clone())
                };
                anyhow::ensure!(!draft.is_empty(), "no dictation draft");
                let text = draft.replace(['\n', '\r'], " ");
                type_target(&target, &text).await?;
                self.session.lock().unwrap().draft.clear();
            } else {
                anyhow::bail!("dictation action must be start, pause, resume, stop, or insert");
            }
        }
        self.emit_session();
        Ok(self.session_json())
    }
    pub(super) async fn listen_pcm(
        &self,
        deadline: Instant,
        session_id: Option<u64>,
        cancel: &CancellationToken,
    ) -> Result<Vec<i16>> {
        let cfg = self.cfg();
        let mut gate = self.streaming_vad(
            cfg.vad.hangover_ms,
            cfg.vad.start_confirm_frames,
            cfg.vad.energy_threshold,
        );
        let mut raw = self.samples.lock().await;
        let mut aec = if cfg.barge_in.aec {
            Some(self.aec_samples.lock().await)
        } else {
            None
        };
        let samples = match aec.as_mut().and_then(|a| a.as_mut()) {
            Some(samples) => samples,
            None => &mut *raw,
        };
        while samples.try_recv().is_ok() {}
        self.audio.start_capture()?;
        let mut pcm = Vec::new();
        let mut pre = std::collections::VecDeque::new();
        let mut speaking = false;
        let mut end = deadline;
        let mut last_tick = Instant::now();
        loop {
            if session_id.is_some() && last_tick.elapsed() >= Duration::from_secs(1) {
                self.emit_session();
                last_tick = Instant::now();
            }
            if session_id.is_some_and(|id| self.session.lock().unwrap().id != id) {
                break;
            }
            let frame = tokio::select! {_=cancel.cancelled()=>break,_=tokio::time::sleep(Duration::from_millis(100))=>{if Instant::now()>=end{break;}continue;},v=samples.recv()=>v.context("audio disconnected")?};
            let decision = gate.push(&frame);
            if !speaking {
                pre.extend(frame.iter().copied());
                let max = cfg.audio.capture_rate as usize / 4;
                while pre.len() > max {
                    pre.pop_front();
                }
            }
            if matches!(decision, VadDecision::SpeechStart) {
                speaking = true;
                end = Instant::now() + Duration::from_secs(60);
                pcm.extend(pre.drain(..));
            } else if speaking {
                pcm.extend_from_slice(&frame);
            }
            if speaking && matches!(decision, VadDecision::SpeechEnd) {
                break;
            }
            if Instant::now() >= end {
                break;
            }
        }
        let _ = self.audio.stop_capture();
        Ok(pcm)
    }
    pub(super) async fn run_session(self: &Arc<Self>, id: u64) -> Result<()> {
        let (mode, deadline) = {
            let s = self.session.lock().unwrap();
            (s.mode, s.deadline)
        };
        let cancel = self.reset_cancel().await;
        self.emit_state(AssistantState::Listening).await;
        self.emit_session();
        let pcm = self.listen_pcm(deadline, Some(id), &cancel).await?;
        if cancel.is_cancelled() || self.session.lock().unwrap().id != id {
            return Ok(());
        }
        if pcm.len() < 3200 {
            self.session.lock().unwrap().stop();
            self.emit_state(AssistantState::Idle).await;
            self.emit_session();
            return Ok(());
        }
        let mut timing = TurnTiming::new();
        timing.mark_vad_end();
        self.emit_state(AssistantState::Transcribing).await;
        let tr = self.stt.finish(&pcm, cancel.clone()).await?;
        if cancel.is_cancelled() || self.session.lock().unwrap().id != id {
            return Ok(());
        }
        if tr.text.trim().is_empty() {
            self.emit_state(AssistantState::Idle).await;
            return Ok(());
        }
        self.emit(Event::Transcript {
            text: tr.text.clone(),
        });
        if matches!(mode, Mode::Dictation | Mode::Paused) {
            if let Some(action) = dictation_control(&tr.text) {
                self.session_control("dictation", action).await?;
            } else if mode == Mode::Dictation {
                let text = punctuation(&tr.text);
                let target = {
                    let mut s = self.session.lock().unwrap();
                    s.draft.push_str(&text);
                    s.deadline =
                        Instant::now() + Duration::from_secs(self.cfg().dictation.idle_secs);
                    s.target.clone()
                };
                let active = active_window().await?;
                if active["address"] != target["address"] {
                    self.session.lock().unwrap().mode = Mode::Paused;
                    self.emit(Event::Error {
                        message: "Dictation paused: application focus changed. Draft retained."
                            .into(),
                    });
                } else if !terminal_target(&target) {
                    match type_target(&target, &text).await {
                        Ok(()) => self.session.lock().unwrap().draft.clear(),
                        Err(err) => {
                            self.session.lock().unwrap().mode = Mode::Paused;
                            self.emit(Event::Error {
                                message: format!("Dictation paused: {err}. Draft retained."),
                            });
                        }
                    }
                }
            }
            self.emit_state(AssistantState::Idle).await;
            self.emit_session();
            return Ok(());
        }
        timing.stt_done = Some(Instant::now());
        self.respond(&tr.text, timing).await
    }
    pub(super) async fn voice_approval(
        &self,
        id: &str,
        tool: &str,
        arguments: &Value,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        self.emit_session_mode("approval");
        let spoken = format!(
            "Approval needed for {}. {}. Say approve or deny.",
            tool,
            arguments.to_string().chars().take(240).collect::<String>()
        );
        let tts = self.tts.clone();
        let pcm = tokio::task::spawn_blocking(move || tts.speak(&spoken)).await??;
        if cancel.is_cancelled() {
            return Ok(false);
        }
        self.audio.play(pcm)?;
        tokio::select! {_=cancel.cancelled()=>return Ok(false),_=self.audio.wait_playback()=>{}}
        let deadline = Instant::now() + Duration::from_secs(55);
        loop {
            if !self.pending.lock().unwrap().contains_key(id) || Instant::now() >= deadline {
                return Ok(false);
            }
            let pcm = self.listen_pcm(deadline, None, cancel).await?;
            if pcm.len() < 3200 || cancel.is_cancelled() {
                return Ok(false);
            }
            let tr = self.stt.transcribe(&pcm, cancel.clone()).await?;
            match tr
                .text
                .trim()
                .trim_end_matches(['.', '!', '?'])
                .to_lowercase()
                .as_str()
            {
                "approve" | "yes approve" | "yes" | "allow once" => return Ok(true),
                "deny" | "no" | "cancel" => return Ok(false),
                _ => {}
            }
        }
    }
    pub(super) fn emit_session_mode(&self, mode: &str) {
        let s = self.session.lock().unwrap();
        self.emit(Event::Session {
            session_id: s.id,
            mode: mode.into(),
            remaining_secs: 60,
            target: String::new(),
            draft: s.draft.clone(),
        });
    }
    pub(super) async fn request_approval(
        &self,
        id: String,
        tool: &str,
        args: &Value,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.approvals.lock().await.insert(id.clone(), tx);
        let expires = now_ms() + 60_000;
        self.pending.lock().unwrap().insert(
            id.clone(),
            json!({"tool":tool,"arguments":args,"expires_at_ms":expires}),
        );
        self.emit(Event::ToolApproval {
            approval_id: id.clone(),
            turn_id: self.turn_id(),
            tool: tool.into(),
            arguments: args.clone(),
            expires_at_ms: expires,
        });
        let plugin_cancel = self
            .registry
            .plugins
            .as_ref()
            .map(|p| p.token(tool))
            .unwrap_or_default();
        let approved = tokio::select! {_=plugin_cancel.cancelled()=>false,_=cancel.cancelled()=>false,r=tokio::time::timeout(Duration::from_secs(60),rx)=>matches!(r,Ok(Ok(true))),r=async {match self.voice_approval(&id,tool,args,cancel).await {Ok(v)=>v,Err(err)=>{tracing::warn!(%err,"voice approval unavailable; keyboard approval remains available");std::future::pending::<bool>().await}}}=>r};
        let _ = self.audio.stop_capture();
        let _ = self.audio.cancel_playback();
        self.approvals.lock().await.remove(&id);
        self.pending.lock().unwrap().remove(&id);
        self.emit_session();
        Ok(approved && !cancel.is_cancelled() && now_ms() < expires)
    }
    pub(super) async fn control_phrase(
        self: &Arc<Self>,
        text: &str,
        timing: &TurnTiming,
    ) -> Result<bool> {
        let clean = text.trim().trim_end_matches(['.', '!', '?']).to_lowercase();
        let action = match clean.as_str() {
            "start conversation" | "start conversation mode" => Some(("conversation", "start")),
            "end conversation" | "stop conversation" | "stop listening" => {
                Some(("conversation", "stop"))
            }
            "start dictating" | "start dictation" => Some(("dictation", "start")),
            "stop dictating" | "end dictation" => Some(("dictation", "stop")),
            "pause dictation" => Some(("dictation", "pause")),
            "resume dictation" => Some(("dictation", "resume")),
            "insert draft" | "insert dictation draft" => Some(("dictation", "insert")),
            _ => None,
        };
        if let Some((kind, action)) = action {
            // Do not invalidate this turn while processing its own spoken control.
            let target = if kind == "dictation" && matches!(action, "start" | "resume") {
                {
                    let target = active_window().await?;
                    anyhow::ensure!(
                        target["address"].as_str().is_some_and(|s| !s.is_empty()),
                        "select an application before dictation"
                    );
                    Some(target)
                }
            } else {
                None
            };
            {
                let mut s = self.session.lock().unwrap();
                match (kind, action) {
                    ("conversation", "start") => {
                        anyhow::ensure!(self.cfg().conversation.enabled, "conversation disabled");
                        s.begin(Mode::Conversation, self.cfg().conversation.idle_secs);
                    }
                    ("dictation", "start") => {
                        anyhow::ensure!(self.cfg().dictation.enabled, "dictation disabled");
                        s.draft.clear();
                        s.target = target.unwrap();
                        s.begin(Mode::Dictation, self.cfg().dictation.idle_secs);
                    }
                    (_, "stop") => s.stop(),
                    (_, "pause") => s.mode = Mode::Paused,
                    (_, "resume") => {
                        s.target = target.unwrap();
                        s.begin(Mode::Dictation, self.cfg().dictation.idle_secs);
                    }
                    _ => {}
                }
            }
            if action == "insert" {
                self.session_control(kind, action).await?;
            }
            self.emit_session();
            let spoken = format!("{kind} {action}");
            let phase = Mutex::new(timing.clone());
            let first = AtomicBool::new(true);
            self.speak_phrase(&self.tts, &spoken, &phase, &first)
                .await?;
            let cancel = self.cancel.lock().await.clone();
            tokio::select! {_=cancel.cancelled()=>{let _=self.audio.cancel_playback();},_=self.audio.wait_playback()=>{}}
            self.emit_state(AssistantState::Idle).await;
            // Acknowledgments must not re-arm a stopped conversation.
            self.emit(Event::AssistantText {
                text: format!("{kind} {action}"),
                done: true,
            });
            self.emit(Event::from_timing(timing));
            return Ok(true);
        }
        if clean == "resume browser" || clean == "take over browser" {
            let name = if clean == "resume browser" {
                "browser.resume"
            } else {
                "browser.takeover"
            };
            let data = if clean == "resume browser" {
                self.resume_browser().await?
            } else {
                self.registry
                    .call_allowed(
                        name,
                        json!({}),
                        &self.cfg(),
                        &CancellationToken::new(),
                    )
                    .await?
            };
            self.emit_tool(name, &json!({}), &data);
            self.speak_reply(
                if clean == "resume browser" {
                    "Browser resumed."
                } else {
                    "Browser paused. Complete the task in that window, then say resume browser."
                },
                timing.clone(),
            )
            .await?;
            return Ok(true);
        }
        if clean == "open that source" || clean == "open the source" || clean == "open first result"
        {
            let result = self
                .registry
                .call_allowed(
                    "browser.open_source",
                    json!({"index":0}),
                    &self.cfg(),
                    &CancellationToken::new(),
                )
                .await?;
            self.emit_tool("browser.open_source", &json!({"index":0}), &result);
            self.speak_reply(
                if result["isError"] == true {
                    "I couldn't open that source."
                } else {
                    "Opening the source."
                },
                timing.clone(),
            )
            .await?;
            return Ok(true);
        }
        if matches!(
            clean.as_str(),
            "where am i" | "what is my location" | "what's my location" | "where are we"
        ) {
            self.emit_state(AssistantState::Thinking).await;
            let spoken=match self.locate_for_turn().await {Ok(v)=>format!("{} {}.",if v.source=="saved" {"Your saved location is"}else{"Your approximate location is"},v.label),Err(_)=>"I couldn't determine your location. Say remember my location followed by your city, or enable location if it is disabled.".into()};
            self.speak_reply(&spoken, timing.clone()).await?;
            return Ok(true);
        }
        if matches!(
            clean.as_str(),
            "disable location" | "enable location" | "forget my location"
        ) {
            let action = match clean.as_str() {
                "disable location" => "off",
                "enable location" => "on",
                _ => "clear",
            };
            self.location_control(action, None).await?;
            self.speak_reply(
                if action == "off" {
                    "Location disabled."
                } else if action == "on" {
                    "Location enabled."
                } else {
                    "Saved location cleared. I'll use approximate network location when needed."
                },
                timing.clone(),
            )
            .await?;
            return Ok(true);
        }
        for prefix in [
            "remember my location ",
            "set my location to ",
            "my location is ",
        ] {
            if clean.starts_with(prefix) {
                let city = text.trim()[prefix.len()..]
                    .trim()
                    .trim_end_matches(['.', '!', '?']);
                self.location_control("set", Some(city.into())).await?;
                self.speak_reply("Location saved.", timing.clone()).await?;
                return Ok(true);
            }
        }
        if let Some(location) = text.trim().strip_prefix("remember weather location ") {
            let mut cfg = self.cfg();
            cfg.browser.weather_location = location.trim().into();
            cfg.save()?;
            *self.cfg.lock().unwrap() = cfg;
            self.speak_reply("Weather location saved.", timing.clone())
                .await?;
            return Ok(true);
        }
        Ok(false)
    }
    pub(super) fn emit_tool(&self, name: &str, args: &Value, data: &Value) {
        let serialized = data.to_string();
        let bounded = if serialized.len() > 16_384 {
            json!({"truncated":true,"text":serialized.chars().take(8000).collect::<String>()})
        } else {
            data.clone()
        };
        let event =
            json!({"type":"tool","tool":name,"arguments":args,"result":bounded,"status":"done"});
        self.record_activity(self.turn_id(), event.clone());
        self.emit(Event::Turn {
            turn_id: self.turn_id(),
            event,
        });
    }
    pub(super) async fn web_answer(&self, ask: &str) -> Result<Value> {
        let cfg = self.cfg();
        let cancel = self.cancel.lock().await.clone();
        if assistant_core::weather::is_weather(ask) {
            let city = assistant_core::weather::place(ask);
            if city.is_none()
                && (!cfg.location.enabled
                    || !self
                        .registry
                        .plugins
                        .as_ref()
                        .is_some_and(|p| p.available("location.current")))
            {
                return Ok(json!({"needs_location":true}));
            }
            let args = json!({"city":city.unwrap_or_default(),"celsius":ask.to_lowercase().contains("celsius")});
            let result = self
                .registry
                .call_allowed("weather.lookup", args.clone(), &cfg, &cancel)
                .await;
            return match result {
                Ok(result) => {
                    self.emit_tool("weather.lookup", &args, &result);
                    Ok(json!({"weather":result["structuredContent"]}))
                }
                Err(error) => {
                    let result = json!({"isError":true,"error":error.to_string()});
                    self.emit_tool("weather.lookup", &args, &result);
                    Ok(json!({"weather_failed":true}))
                }
            };
        }
        let mut query = ask.to_string();
        if assistant_core::location::needs_location(ask)
            && !assistant_core::location::explicit_place(ask)
        {
            let location = if cfg.location.enabled
                && (ask.to_lowercase().contains("weather")
                    || ask.to_lowercase().contains("forecast"))
                && !cfg.browser.weather_location.is_empty()
            {
                Some(cfg.browser.weather_location.clone())
            } else {
                self.locate_for_turn().await.ok().map(|v| v.label)
            };
            match location {
                Some(location) => query = format!("{ask}; location: {location}"),
                None => return Ok(json!({"needs_location":true})),
            }
        }
        let today = chrono_date();
        if ["news", "latest", "weather", "forecast", "today", "current"]
            .iter()
            .any(|x| query.to_lowercase().contains(x))
        {
            query = format!("{query} {today}");
        }
        let args = json!({"query":query});
        let search_tool = if cfg.browser.search_tool.is_empty() {
            "web.search"
        } else {
            &cfg.browser.search_tool
        };
        let result = self
            .registry
            .call_allowed(search_tool, args.clone(), &cfg, &cancel)
            .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => json!({"isError":true,"error":error.to_string()}),
        };
        self.emit_tool(search_tool, &args, &result);
        let mut sources = Vec::new();
        let mut search = result["structuredContent"].clone();
        if let Some(results) = search["results"].as_array_mut() {
            for source in results {
                if let Some(text) = source["text"].as_str() {
                    source["text"] = text.chars().take(1200).collect::<String>().into();
                }
                if let Some(text) = source["snippet"].as_str() {
                    source["snippet"] = text.chars().take(150).collect::<String>().into();
                }
            }
        }
        let data = &search;
        if result["isError"] == true
            || data["needs_takeover"] == true
            || data["results"].as_array().is_none_or(|r| r.is_empty())
        {
            if search_tool == "web.search" {
                *self.pending_browser.lock().unwrap() = Some(ask.into());
            }
            return Ok(
                json!({"retrieval_failed":true,"details":result,"note":"No current facts verified. Offer browser takeover; do not answer from memory."}),
            );
        }
        if let Some(results) = data["results"].as_array() {
            if cfg.browser.enabled {
                let citations: Vec<_> = results
                    .iter()
                    .take(5)
                    .map(|v| json!({"title":v["title"],"url":v["url"]}))
                    .collect();
                let _ = self
                    .registry
                    .call_allowed(
                        "browser.remember_sources",
                        json!({"sources":citations}),
                        &cfg,
                        &cancel,
                    )
                    .await;
            }
            for source in results.iter().take(3) {
                if let Some(text) = source["text"].as_str().filter(|s| !s.is_empty()) {
                    sources.push(json!({"source":{"title":source["title"],"url":source["url"]},"page":{"structuredContent":{"text":text.chars().take(1200).collect::<String>(),"published_at":source["published_at"],"retrieved_at":data["retrieved_at"]}}}));
                    continue;
                }
                let args = json!({"url":source["url"]});
                let r = self
                    .registry
                    .call_allowed("web.read", args.clone(), &cfg, &cancel)
                    .await;
                let mut r = match r {
                    Ok(v) => v,
                    Err(e) => json!({"isError":true,"error":e.to_string()}),
                };
                if let Some(text) = r["structuredContent"]["text"].as_str() {
                    let text = text.chars().take(1000).collect::<String>();
                    r["structuredContent"]["text"] = text.into();
                    r["content"] = Value::Null;
                }
                self.emit_tool("web.read", &args, &r);
                sources.push(json!({"source":source,"page":r}));
            }
        }
        Ok(
            json!({"search":data,"sources":sources,"instructions":"Answer only supported retrieved facts. For weather specify location, forecast period and units; for news compare publication and event dates. Mention sources briefly. If pages fail, distinguish search snippets from verified page content."}),
        )
    }
    pub(super) async fn needs_lookup(&self, ask: &str) -> Result<bool> {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut tokens=self.llm.generate_with_grammar("Decide whether answering requires current web facts or knowledge you cannot confidently provide. Use web for changing news, weather, schedules, prices, current office holders and unfamiliar factual questions. Do not use web for arithmetic, conversation, creative writing or stable facts you know. Reply only true (web needed) or false (local answer).",&[],ask,8,0.0,cancel.clone(),Some("root ::= \"true\" | \"false\"".into())).await?;
        let token = self.cancel.lock().await.clone();
        let mut out = String::new();
        let r=tokio::time::timeout(Duration::from_secs(15),async{loop{tokio::select!{_=token.cancelled()=>anyhow::bail!("cancelled"),v=tokens.recv()=>match v{Some(v)=>out.push_str(&v?),None=>break}}}Ok::<_,anyhow::Error>(())}).await;
        cancel.store(true, Ordering::Relaxed);
        r??;
        Ok(out.trim() == "true")
    }
}
async fn active_window() -> Result<Value> {
    let out = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new("hyprctl")
            .args(["activewindow", "-j"])
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    anyhow::ensure!(out.status.success(), "cannot read selected application");
    Ok(serde_json::from_slice(&out.stdout)?)
}
fn terminal_target(target: &Value) -> bool {
    let class = target["class"].as_str().unwrap_or("").to_lowercase();
    [
        "foot",
        "kitty",
        "alacritty",
        "ghostty",
        "wezterm",
        "xterm",
        "terminal",
    ]
    .iter()
    .any(|s| class.contains(s))
}
async fn type_target(target: &Value, text: &str) -> Result<()> {
    let active = active_window().await?;
    anyhow::ensure!(
        active["address"] == target["address"] && !target["address"].is_null(),
        "application focus changed"
    );
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("wtype")
            .args(["--", text])
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    anyhow::ensure!(
        output.status.success(),
        "typing failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
fn chrono_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub(super) fn compact_result(value: &Value) -> Value {
    let mut data = value["structuredContent"].clone();
    if data.is_null() {
        return json!({"isError":value["isError"],"text":value.to_string().chars().take(1800).collect::<String>()});
    }
    if let Some(text) = data["text"].as_str() {
        data["text"] = text.chars().take(1400).collect::<String>().into();
    }
    if let Some(text) = data["output"].as_str() {
        data["output"] = text
            .chars()
            .rev()
            .take(2000)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
            .into();
    }
    if let Some(elements) = data["elements"].as_array_mut() {
        elements.truncate(30);
    }
    json!({"isError":value["isError"],"data":data})
}

impl App {
    pub(super) async fn dispatch_plugin(
        &self,
        name: &str,
        args: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        self.dispatch_plugin_depth(name, args, cancel, 0, 4).await
    }
    pub(super) async fn dispatch_plugin_depth(
        &self,
        name: &str,
        args: Value,
        cancel: &CancellationToken,
        depth: usize,
        budget: usize,
    ) -> Result<Value> {
        anyhow::ensure!(depth <= 1, "nested host dispatch limit");
        *self.last_activity.lock().unwrap() = Instant::now();
        let cfg = self.cfg();
        self.registry.validate(name, &args)?;
        let generation = self
            .registry
            .plugins
            .as_ref()
            .and_then(|p| p.generation(name));
        let verdict = self.registry.verdict(name, &cfg);
        if verdict.decision() == assistant_policy::Decision::Deny {
            return Err(self.registry.refuse(&verdict, &args));
        }
        let policy = verdict.decision().as_str();
        let mut args = args;
        let mut review_args = args.clone();
        if name == "routines.run" {
            let preview = self
                .registry
                .call_allowed(
                    "routines.preview",
                    json!({"name":args["name"]}),
                    &self.cfg(),
                    cancel,
                )
                .await?;
            anyhow::ensure!(preview["isError"] != true, "routine preview failed");
            review_args["steps"] = preview["structuredContent"]["steps"].clone();
            args["expected_digest"] = preview["structuredContent"]["digest"].clone();
        }
        self.registry.validate(name, &args)?;
        let mut approval_ms = 0;
        let grant = match self.registry.authorize(name, &args, &cfg)? {
            Authorization::Granted(grant) => grant,
            Authorization::NeedsApproval(verdict) => {
                let approval_start = Instant::now();
                let id = format!("plugin-{}-{}", self.turn_id(), now_ms());
                let approved = self
                    .request_approval(id.clone(), name, &review_args, cancel)
                    .await?;
                let mut record = self.registry.record("approval", &verdict, &review_args);
                record.turn = Some(self.turn_id().to_string());
                record.approval_id = Some(id.clone());
                record.outcome = if approved { "approved" } else { "denied" }.into();
                self.registry.journal(record);
                anyhow::ensure!(approved, "approval denied, expired or cancelled");
                approval_ms = approval_start.elapsed().as_millis() as u64;
                // Bound to the final arguments (routines.run adds the reviewed digest).
                Grant::approved(&verdict, &args, &id)?
            }
        }
        .for_turn(self.turn_id().to_string());
        anyhow::ensure!(
            generation
                == self
                    .registry
                    .plugins
                    .as_ref()
                    .and_then(|p| p.generation(name)),
            "plugin changed during approval; retry"
        );
        anyhow::ensure!(
            self.registry.policy(name, &self.cfg()) == policy,
            "tool permissions changed during review; retry"
        );
        let result = self
            .registry
            .call(name, args.clone(), &self.cfg(), cancel, &grant)
            .await?;
        let mut result = self
            .host_extensions(name, result, cancel, depth, budget)
            .await?;
        let child_approval_ms =
            result["structuredContent"]["completed"]
                .as_array()
                .map_or(0, |steps| {
                    steps
                        .iter()
                        .filter_map(|s| s["result"]["assistant_approval_ms"].as_u64())
                        .sum::<u64>()
                });
        result["assistant_approval_ms"] = json!(approval_ms + child_approval_ms);
        self.remember_action(name, &args, &result)?;
        self.emit_tool(name, &args, &result);
        Ok(result)
    }
    pub(super) async fn plugin_events(self: &Arc<Self>) {
        let Some(plugins) = &self.registry.plugins else {
            return;
        };
        if plugins.refresh_health() {
            self.emit(Event::Plugins {
                state: plugins.inspect(),
            });
        }
        let events = plugins.events();
        for (id, event) in events {
            if !event["id"].is_string() || !event["title"].is_string() || !event["text"].is_string()
            {
                continue;
            }
            // Due delivery remains pending in the plugin until the host acknowledges it.
            if self.muted.load(Ordering::SeqCst) {
                continue;
            }
            let dnd = tokio::time::timeout(
                Duration::from_secs(1),
                tokio::process::Command::new("omarchy-shell")
                    .args(["notifications", "isDnd"])
                    .kill_on_drop(true)
                    .output(),
            )
            .await;
            if matches!(dnd,Ok(Ok(ref output)) if output.status.success() && String::from_utf8_lossy(&output.stdout).trim()=="on")
            {
                continue;
            }
            let result = tokio::time::timeout(
                Duration::from_secs(3),
                tokio::process::Command::new("notify-send")
                    .args([
                        "--app-name=Assistant",
                        "--",
                        event["title"].as_str().unwrap(),
                        event["text"].as_str().unwrap(),
                    ])
                    .kill_on_drop(true)
                    .output(),
            )
            .await;
            if !matches!(result,Ok(Ok(ref o)) if o.status.success()) {
                self.emit(Event::Error {
                    message: format!("{id}: notification service unavailable; delivery retained"),
                });
                continue;
            }
            if let Err(e) = plugins.ack_event(&id, event["id"].as_str().unwrap()).await {
                self.emit(Event::Error {
                    message: format!("{id}: event acknowledgement failed: {e}"),
                });
                continue;
            }
            let quiet = self.pending.lock().unwrap().is_empty()
                && self.session.lock().unwrap().mode == Mode::Wake
                && self
                    .state
                    .try_lock()
                    .is_ok_and(|s| s.current() == AssistantState::Idle);
            if quiet && !self.turn_busy.swap(true, Ordering::SeqCst) {
                if let Some(speech) = event["speech"].as_str() {
                    let _ = self.speak_reply(speech, TurnTiming::new()).await;
                }
                self.turn_busy.store(false, Ordering::SeqCst);
            }
        }
    }
}
