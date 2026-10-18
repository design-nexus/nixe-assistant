//! Listening while speaking (plan §4.5). While a reply plays, the echo-cancelled microphone
//! goes through Silero; at speech onset the voice ducks and the words are streamed to the
//! recognizer, then classified:
//!
//! - **echo**: the words are the ones being spoken (AEC let some through): carry on.
//! - **backchannel**: "mm-hm", "yeah", "okay": carry on.
//! - **noise**: no words at all (a cough, a door): carry on.
//! - **stop**: "stop", "wait", "never mind": end the reply and listen for a follow-up.
//! - **redirect**: anything else: end the reply; the words become the next request.
//!
//! The decision is rule-based (no LLM call), so it is quick. An interruption can only end
//! or redirect a reply: approvals never go through here.

use super::*;
use assistant_audio::PlayedTo;
use assistant_core::asr::Reply;
use assistant_core::config::DuplexCfg;
use assistant_core::state::{Mic, Playback, Recognition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Not enough words yet.
    Pending,
    Echo,
    Backchannel,
    Noise,
    Stop,
    Redirect,
}

impl Verdict {
    /// Whether the reply carries on.
    pub fn carries_on(self) -> bool {
        matches!(self, Self::Echo | Self::Backchannel | Self::Noise)
    }
}

/// Words that carry no request on their own: they neither start nor block a redirect.
const FILLER: &[&str] = &[
    "please", "a", "the", "second", "sec", "moment", "minute", "bit", "thanks", "thank", "you", "no", "hey", "um", "uh",
    "er", "ah", "oh", "just", "now", "there",
];

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('-', " ")
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether `tokens` is made of the given phrases, one after another ("okay stop", "wait wait").
fn covered(tokens: &[String], phrases: &[Vec<String>]) -> bool {
    if tokens.is_empty() {
        return true;
    }
    phrases.iter().any(|p| !p.is_empty() && tokens.len() >= p.len() && tokens[..p.len()] == p[..] && covered(&tokens[p.len()..], phrases))
}

pub struct Classifier {
    stop: Vec<Vec<String>>,
    backchannel: Vec<Vec<String>>,
    name: String,
    noise_ms: u64,
}

impl Classifier {
    pub fn new(cfg: &DuplexCfg, assistant_name: &str) -> Self {
        let phrases = |list: &[String]| list.iter().map(|p| words(p)).filter(|w| !w.is_empty()).collect();
        Self { stop: phrases(&cfg.stop_phrases), backchannel: phrases(&cfg.backchannels), name: assistant_name.to_lowercase(), noise_ms: cfg.noise_ms }
    }

    /// `speaking` is the text of the phrases playing now; `since_onset_ms` how long ago the
    /// speech started.
    pub fn classify(&self, text: &str, is_final: bool, since_onset_ms: u64, speaking: &str) -> Verdict {
        let w = words(text);
        if w.is_empty() {
            return if is_final || since_onset_ms >= self.noise_ms { Verdict::Noise } else { Verdict::Pending };
        }
        // Fillers and the assistant's name ("Echo, stop") don't change what was said.
        let core: Vec<String> = w.iter().filter(|t| !FILLER.contains(&t.as_str()) && **t != self.name).cloned().collect();
        // Phrases may contain filler words ("uh huh"), so they're matched with and without them.
        let named: Vec<String> = w.iter().filter(|t| **t != self.name).cloned().collect();
        let made_of = |phrases: &[Vec<String>]| (!core.is_empty() && covered(&core, phrases)) || (!named.is_empty() && covered(&named, phrases));
        let is_backchannel = made_of(&self.backchannel);
        // "okay, stop": stop and backchannel words together, with a stop among them.
        let either: Vec<Vec<String>> = self.stop.iter().chain(&self.backchannel).cloned().collect();
        let is_stop = !is_backchannel && made_of(&either);
        if !is_stop {
            let said = words(speaking);
            let echoed = w.iter().filter(|t| said.contains(t)).count();
            if echoed * 10 >= w.len() * 8 {
                return Verdict::Echo;
            }
        }
        // Three words of request are enough to stop talking, before the speaker finishes.
        let content = core.iter().filter(|t| !covered(std::slice::from_ref(*t), &self.stop) && !covered(std::slice::from_ref(*t), &self.backchannel)).count();
        if content >= 3 {
            return Verdict::Redirect;
        }
        if !is_final {
            return Verdict::Pending;
        }
        if is_stop {
            Verdict::Stop
        } else if is_backchannel {
            Verdict::Backchannel
        } else if core.is_empty() && !is_backchannel {
            Verdict::Noise
        } else {
            Verdict::Redirect
        }
    }
}

/// How long the recognizer's words must stay the same before they count without its final
/// result: the echo of the reply can keep the voice detector from ever hearing the end.
pub const SETTLED_MS: u64 = 350;

impl Classifier {
    /// The verdict for words that stopped changing before the recognizer finished: a stop,
    /// a backchannel or an echo is clear already; a short redirect may still be growing, so
    /// it waits for the final text.
    pub fn classify_settled(&self, text: &str, since_onset_ms: u64, speaking: &str) -> Verdict {
        match self.classify(text, true, since_onset_ms, speaking) {
            v @ (Verdict::Stop | Verdict::Backchannel | Verdict::Echo) if !words(text).is_empty() => v,
            _ => Verdict::Pending,
        }
    }
}

/// The text of the phrase playing now and the one before it, for the echo check.
pub fn speaking_now(track: &ReplyTrack, played: PlayedTo) -> String {
    if played.generation != track.generation || track.phrases.is_empty() {
        return track.phrases.last().cloned().unwrap_or_default();
    }
    let i = (played.phrase as usize).min(track.phrases.len() - 1);
    let from = i.saturating_sub(1);
    track.phrases[from..=(i + 1).min(track.phrases.len() - 1)].join(" ")
}

/// What the listener heard of a reply before it was cut off: the phrases played to the
/// end, plus the played share of the current one, cut back to a whole word.
pub fn heard_text(track: &ReplyTrack, played: PlayedTo) -> String {
    if played.generation != track.generation || track.phrases.is_empty() {
        return String::new();
    }
    let i = (played.phrase as usize).min(track.phrases.len() - 1);
    let mut parts: Vec<&str> = track.phrases[..i].iter().map(String::as_str).collect();
    let current = &track.phrases[i];
    let share = if played.total == 0 { 1.0 } else { played.samples as f64 / played.total as f64 };
    let cut = if share >= 0.98 {
        current.as_str()
    } else {
        let n = (current.len() as f64 * share) as usize;
        let mut end = n.min(current.len());
        while end > 0 && !current.is_char_boundary(end) {
            end -= 1;
        }
        // Back to the end of the last whole word.
        match current[..end].rfind(char::is_whitespace) {
            Some(space) if end < current.len() && !current[end..].starts_with(char::is_whitespace) => &current[..space],
            _ => &current[..end],
        }
    };
    if !cut.trim().is_empty() {
        parts.push(cut.trim());
    }
    parts.join(" ")
}

/// Pre-roll kept before an onset, so the recognizer hears the start of the first word.
const PREROLL_SAMPLES: usize = 4_800;

fn rms(chunk: &[i16]) -> f32 {
    let acc: f32 = chunk.iter().map(|&s| (s as f32 / 32768.0).powi(2)).sum();
    (acc / chunk.len().max(1) as f32).sqrt()
}

/// With `NIXE_ASSISTANT_DUPLEX_CLIPS=1`, the last five interruption clips are kept for tuning
/// (`$XDG_RUNTIME_DIR/nixe-assistant/duplex-N.wav`, private and cleared at logout).
fn dump_clip(pcm: &[i16]) {
    if std::env::var_os("NIXE_ASSISTANT_DUPLEX_CLIPS").is_none_or(|v| v != "1") {
        return;
    }
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed) % 5;
    let path = assistant_core::paths::runtime_dir().join(format!("duplex-{n}.wav"));
    let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    if let Ok(mut w) = hound::WavWriter::create(&path, spec) {
        for &s in pcm {
            let _ = w.write_sample(s);
        }
        let _ = w.finalize();
    }
}

/// What the history keeps of an interrupted reply.
pub fn interrupted_note(heard: &str) -> String {
    if heard.trim().is_empty() {
        "(interrupted before answering)".into()
    } else {
        format!("{} — [interrupted]", heard.trim())
    }
}

impl App {
    /// Whether this reply can be listened to while it plays: streaming speech, AEC and
    /// `[duplex]` on. Otherwise the plain barge-in (any speech stops the reply) runs.
    pub(super) fn duplex_ready(&self) -> bool {
        let cfg = self.cfg();
        cfg.duplex.enabled && cfg.barge_in.enabled && self.stt.asr().is_some() && self.audio.aec_levels().is_some()
    }

    fn unduck(&self) {
        self.audio.set_gain(1.0);
        self.audio.set_paused(false);
        self.update_activity(|a| {
            if matches!(a.playback, Playback::Ducked | Playback::Paused) {
                a.playback = Playback::Playing;
            }
            a.mic = Mic::Monitoring;
            a.recognition = Recognition::Idle;
        });
    }

    /// Runs alongside a reply until it ends (the turn aborts this task) or an interruption
    /// has been handled.
    pub(super) async fn watch_duplex(self: &Arc<Self>, cancel: CancellationToken) {
        let cfg = self.cfg();
        let Some(asr) = self.stt.asr() else { return };
        let Some(mut levels) = self.audio.aec_levels() else { return };
        let classifier = Classifier::new(&cfg.duplex, &cfg.assistant.name);
        let barge = cfg.barge_in.clone();
        // The first moments of a reply set how much echo gets through AEC.
        let grace_end = tokio::time::Instant::now() + Duration::from_millis(barge.grace_ms.max(1));
        let mut residual = 0.0f32;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep_until(grace_end) => break,
                changed = levels.changed() => {
                    if changed.is_err() { return; }
                    residual = residual.max(*levels.borrow());
                }
            }
        }
        // Just above the echo AEC lets through: a false onset only ducks the voice briefly
        // (the words decide), while the old barge-in gate missed normal speaking levels.
        let threshold = residual.mul_add(1.2, 0.001).clamp(cfg.duplex.min_rms, 0.04);
        self.update_activity(|a| a.mic = Mic::Monitoring);
        tracing::info!(residual, threshold, "duplex listening");
        let mut guard = self.aec_samples.lock().await;
        let Some(rx) = guard.as_mut() else { return };
        while rx.try_recv().is_ok() {}
        let mut pre = std::collections::VecDeque::<i16>::new();
        loop {
            // Long enough to ride over a comma ("actually, …"), short enough for a quick "stop".
            let mut vad = self.streaming_vad(cfg.vad.hangover_ms.max(450), cfg.vad.start_confirm_frames, 0.08);
            // Wait for someone to start speaking.
            loop {
                let chunk = tokio::select! {
                    _ = cancel.cancelled() => return,
                    c = rx.recv() => match c { Some(c) => c, None => return },
                };
                pre.extend(chunk.iter().copied());
                while pre.len() > PREROLL_SAMPLES {
                    pre.pop_front();
                }
                if matches!(vad.push(&chunk), VadDecision::SpeechStart) && rms(&chunk) >= threshold {
                    break;
                }
            }
            let onset = Instant::now();
            let pause = cfg.duplex.on_speech == "pause";
            if pause {
                self.audio.set_paused(true);
            } else {
                self.audio.set_gain(cfg.duplex.duck_gain);
            }
            self.update_activity(|a| {
                a.playback = if pause { Playback::Paused } else { Playback::Ducked };
                a.recognition = Recognition::Streaming;
            });
            let react_us = onset.elapsed().as_micros() as u64;
            tracing::info!(us = react_us, pause, "duplex onset");
            let u = match asr.begin().await {
                Ok(u) => u,
                Err(err) => {
                    tracing::warn!("duplex: {err:#}");
                    self.unduck();
                    return;
                }
            };
            let mut replies = asr.subscribe();
            let mut heard_pcm: Vec<i16> = pre.make_contiguous().to_vec();
            asr.feed(u, &heard_pcm);
            pre.clear();
            let speaking = || {
                let track = self.reply.lock().unwrap().clone();
                speaking_now(&track, self.audio.played())
            };
            let mut text = String::new();
            let mut changed = Instant::now();
            let mut ended = false;
            let mut verdict = Verdict::Pending;
            while verdict == Verdict::Pending && !ended {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        asr.abort(u);
                        return;
                    }
                    c = rx.recv() => {
                        let Some(chunk) = c else { return };
                        asr.feed(u, &chunk);
                        heard_pcm.extend_from_slice(&chunk);
                        ended = matches!(vad.push(&chunk), VadDecision::SpeechEnd);
                    }
                    r = replies.recv() => {
                        if let Ok(Reply::Partial { u: id, text: t, .. }) = r {
                            if id == u {
                                if t != text {
                                    changed = Instant::now();
                                }
                                text = t;
                                verdict = classifier.classify(&text, false, onset.elapsed().as_millis() as u64, &speaking());
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        let since = onset.elapsed().as_millis() as u64;
                        verdict = classifier.classify(&text, false, since, &speaking());
                        // "Stop" said over the reply: its words settle well before the voice
                        // detector hears an end through the echo.
                        if verdict == Verdict::Pending && changed.elapsed() >= Duration::from_millis(SETTLED_MS) {
                            verdict = classifier.classify_settled(&text, since, &speaking());
                        }
                    }
                }
            }
            if verdict == Verdict::Stop && !ended {
                // Decided from settled words: the recognizer's take isn't needed (a redirect
                // keeps it for the rest of the request; carrying on drops it below).
                asr.abort(u);
            } else if verdict == Verdict::Pending {
                // The speaker stopped before the words decided it: the final text does.
                text = match asr.finish(u, &cancel).await {
                    Ok(t) => t,
                    Err(_) if cancel.is_cancelled() => return,
                    Err(err) => {
                        tracing::warn!("duplex: {err:#}");
                        String::new()
                    }
                };
                verdict = classifier.classify(&text, true, onset.elapsed().as_millis() as u64, &speaking());
            }
            tracing::info!(?verdict, text = %text, ms = onset.elapsed().as_millis() as u64, rms = rms(&heard_pcm), "duplex decided");
            dump_clip(&heard_pcm);
            self.emit(Event::Interruption {
                verdict: format!("{verdict:?}").to_lowercase(),
                text: text.clone(),
                decide_ms: onset.elapsed().as_millis() as u64,
                react_us,
            });
            if verdict.carries_on() {
                if !ended {
                    asr.abort(u);
                    // Let this stretch of speech finish before listening for the next.
                    let until = Instant::now() + Duration::from_secs(3);
                    while Instant::now() < until {
                        let chunk = tokio::select! {
                            _ = cancel.cancelled() => return,
                            c = rx.recv() => match c { Some(c) => c, None => return },
                        };
                        if matches!(vad.push(&chunk), VadDecision::SpeechEnd) {
                            break;
                        }
                    }
                }
                self.unduck();
                continue;
            }
            // Stop or redirect: note what was heard, drop the reply's audio, then cancel the
            // turn (which ends this task, so everything else happens first).
            let (heard, generation) = {
                let track = self.reply.lock().unwrap().clone();
                (heard_text(&track, self.audio.played()), track.generation)
            };
            *self.interrupted.lock().unwrap() = Some(heard);
            self.audio.cancel_generation(generation);
            self.audio.set_paused(false);
            self.audio.set_gain(1.0);
            tracing::info!(?verdict, onset_to_cancel_ms = onset.elapsed().as_millis() as u64, "duplex interrupted the reply");
            drop(guard);
            if verdict == Verdict::Stop {
                self.barged_in.store(true, Ordering::SeqCst);
            } else {
                self.barged_in.store(false, Ordering::SeqCst);
                let app = Arc::clone(self);
                let final_text = if ended { Some(text) } else { None };
                tokio::spawn(async move { app.finish_redirect(u, vad, final_text).await });
            }
            self.emit_state(AssistantState::Interrupted).await;
            cancel.cancel();
            return;
        }
    }

    /// Listen up to 1.2 s for more speech after a short redirect and append it.
    async fn continue_redirect(&self, text: String) -> String {
        let Some(asr) = self.stt.asr() else { return text };
        let cfg = self.cfg();
        let mut vad = self.streaming_vad(cfg.vad.hangover_ms.max(450), cfg.vad.start_confirm_frames, 0.08);
        let mut guard = self.aec_samples.lock().await;
        let Some(rx) = guard.as_mut() else { return text };
        let Ok(u) = asr.begin().await else { return text };
        let mut pre = Vec::new();
        let wait_until = Instant::now() + Duration::from_millis(1200);
        let mut speaking = false;
        let hard_stop = Instant::now() + Duration::from_secs(20);
        while Instant::now() < hard_stop && (speaking || Instant::now() < wait_until) {
            let Ok(Some(chunk)) = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await else { continue };
            let d = vad.push(&chunk);
            if !speaking {
                pre.extend_from_slice(&chunk);
                if matches!(d, VadDecision::SpeechStart) {
                    speaking = true;
                    let keep = pre.len().saturating_sub(PREROLL_SAMPLES);
                    asr.feed(u, &pre[keep..]);
                }
                continue;
            }
            asr.feed(u, &chunk);
            if matches!(d, VadDecision::SpeechEnd) {
                break;
            }
        }
        drop(guard);
        if !speaking {
            asr.abort(u);
            return text;
        }
        match asr.finish(u, &CancellationToken::new()).await {
            Ok(more) if !more.trim().is_empty() => format!("{} {}", text.trim_end_matches(['.', ',', '?', '!']), more.trim()),
            _ => text,
        }
    }

    /// The rest of a redirecting utterance, then a turn for it once the cut-off reply ends.
    async fn finish_redirect(self: Arc<Self>, u: u32, mut vad: StreamingVad, text: Option<String>) {
        let Some(asr) = self.stt.asr() else { return };
        let none = CancellationToken::new();
        let text = match text {
            Some(t) => t,
            None => {
                {
                    let mut guard = self.aec_samples.lock().await;
                    if let Some(rx) = guard.as_mut() {
                        let until = Instant::now() + Duration::from_secs(30);
                        while Instant::now() < until {
                            let Ok(Some(chunk)) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await else { break };
                            asr.feed(u, &chunk);
                            if matches!(vad.push(&chunk), VadDecision::SpeechEnd) {
                                break;
                            }
                        }
                    }
                }
                match asr.finish(u, &none).await {
                    Ok(t) => t,
                    Err(err) => {
                        tracing::warn!("duplex redirect: {err:#}");
                        return;
                    }
                }
            }
        };
        if text.trim().is_empty() {
            return;
        }
        // A short start ("actually…") is often followed by the request after a breath.
        let text = if text.split_whitespace().count() < 3 { self.continue_redirect(text).await } else { text };
        // The interrupted turn unwinds first (it records what was heard).
        let until = Instant::now() + Duration::from_secs(5);
        while self.turn_busy.load(Ordering::SeqCst) && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tracing::info!(text = %text, "duplex redirect: next request");
        self.start_text_turn(text).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classifier() -> Classifier {
        Classifier::new(&DuplexCfg::default(), "Echo")
    }

    #[test]
    fn classifies_interruptions() {
        let c = classifier();
        let speaking = "The forecast for Paris tomorrow is sunny with a high of twenty degrees";
        let f = |t: &str| c.classify(t, true, 900, speaking);
        let p = |t: &str| c.classify(t, false, 400, speaking);
        assert_eq!(f("Stop."), Verdict::Stop);
        assert_eq!(f("Echo, stop please"), Verdict::Stop);
        assert_eq!(f("wait wait"), Verdict::Stop);
        assert_eq!(f("Hold on a second."), Verdict::Stop);
        assert_eq!(f("Never mind."), Verdict::Stop);
        assert_eq!(f("Okay, stop."), Verdict::Stop);
        assert_eq!(f("Mm-hm."), Verdict::Backchannel);
        assert_eq!(f("Yeah, okay."), Verdict::Backchannel);
        assert_eq!(f("uh huh"), Verdict::Backchannel);
        assert_eq!(f("I see"), Verdict::Backchannel);
        assert_eq!(f(""), Verdict::Noise);
        assert_eq!(f("um"), Verdict::Noise);
        assert_eq!(f("sunny with a high"), Verdict::Echo);
        assert_eq!(f("twenty"), Verdict::Echo);
        assert_eq!(f("Actually, what's the weather in Berlin?"), Verdict::Redirect);
        assert_eq!(f("Yeah but what about Saturday?"), Verdict::Redirect);
        assert_eq!(f("Wait, what time is it?"), Verdict::Redirect);
        assert_eq!(f("Louder"), Verdict::Redirect);
        // Partials: a request is clear after three words; short ones wait for the end.
        assert_eq!(p("what about Saturday"), Verdict::Redirect);
        assert_eq!(p("stop"), Verdict::Pending);
        assert_eq!(p("yeah"), Verdict::Pending);
        assert_eq!(c.classify("", false, 400, speaking), Verdict::Pending);
        assert_eq!(c.classify("", false, 2000, speaking), Verdict::Noise);
        // A stop phrase the assistant happens to be saying is still a stop.
        assert_eq!(c.classify("stop", true, 900, "Please stop at the next exit"), Verdict::Stop);
        // Words that stopped changing: a stop, backchannel or echo is clear; a short request
        // or nothing at all waits for the final text.
        let s = |t: &str| c.classify_settled(t, 600, speaking);
        assert_eq!(s("stop"), Verdict::Stop);
        assert_eq!(s("Echo, wait"), Verdict::Stop);
        assert_eq!(s("mm-hm"), Verdict::Backchannel);
        assert_eq!(s("sunny"), Verdict::Echo);
        assert_eq!(s("Louder"), Verdict::Pending);
        assert_eq!(s("Wait, what"), Verdict::Pending);
        assert_eq!(s(""), Verdict::Pending);
    }

    fn track() -> ReplyTrack {
        ReplyTrack { generation: 4, phrases: vec!["The forecast is sunny.".into(), "Highs near twenty degrees.".into(), "Bring sunglasses.".into()] }
    }

    #[test]
    fn heard_text_stops_where_playback_stopped() {
        let t = track();
        let at = |phrase, samples, total| PlayedTo { generation: 4, phrase, samples, total };
        assert_eq!(heard_text(&t, at(0, 1000, 1000)), "The forecast is sunny.");
        // Halfway through the second phrase, cut back to a whole word.
        assert_eq!(heard_text(&t, at(1, 500, 1000)), "The forecast is sunny. Highs near");
        assert_eq!(heard_text(&t, at(1, 10, 1000)), "The forecast is sunny.");
        assert_eq!(heard_text(&t, at(2, 1000, 1000)), "The forecast is sunny. Highs near twenty degrees. Bring sunglasses.");
        // Another reply's audio: nothing of this one was heard.
        assert_eq!(heard_text(&t, PlayedTo { generation: 3, phrase: 2, samples: 5, total: 5 }), "");
    }

    #[test]
    fn speaking_now_spans_the_current_phrase() {
        let t = track();
        let s = speaking_now(&t, PlayedTo { generation: 4, phrase: 1, samples: 3, total: 9 });
        assert!(s.contains("sunny") && s.contains("twenty") && s.contains("sunglasses"));
        assert_eq!(speaking_now(&t, PlayedTo::default()), "Bring sunglasses.");
    }
}
