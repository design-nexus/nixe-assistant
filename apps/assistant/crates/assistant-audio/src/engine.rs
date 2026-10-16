//! PipeWire capture (16 kHz S16 mono) and playback (24 kHz F32 mono).

use crate::vad::VadGate;
use anyhow::{Context, Result};
use assistant_core::config::Config;
use pipewire as pw;
use pw::spa::pod::Pod;
use pw::{properties::properties, spa};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot, watch};

const CAPTURE_RATE: u32 = 16_000;
const PLAYBACK_RATE: u32 = 24_000;
const FRAME: usize = 512;

enum PwCmd {
    StartCapture,
    StopCapture,
    QueuePlay { epoch: u64, generation: u64, phrase: u32, pcm: Vec<f32> },
    CancelPlay { epoch: u64 },
    Shutdown,
}

pub struct AudioEngine {
    cmd: pw::channel::Sender<PwCmd>,
    samples_rx: Mutex<Option<mpsc::Receiver<Vec<i16>>>>,
    levels: watch::Receiver<f32>,
    aec_levels: Option<watch::Receiver<f32>>,
    aec_samples_rx: Mutex<Option<mpsc::Receiver<Vec<i16>>>>,
    queued: Arc<AtomicUsize>,
    play_epoch: Arc<AtomicU64>,
    play: Arc<Mutex<PlayState>>,
    echo_module: Option<u32>,
    echo_wire: Option<Arc<AtomicBool>>,
}

struct AecCapture {
    tx: mpsc::Sender<Vec<i16>>,
    pending: Vec<i16>,
}

struct CaptureState {
    tx: mpsc::Sender<Vec<i16>>,
    level: watch::Sender<f32>,
    active: bool,
    pending: Vec<i16>,
    armed: Option<Instant>,
}

struct PlayState {
    queue: crate::PlayQueue,
    epoch: u64,
    /// Replies up to this generation were cancelled: their phrases still on the way through
    /// the command channel are dropped when they arrive.
    cancelled_through: u64,
    queued: Arc<AtomicUsize>,
}

/// The generation of audio that isn't part of a reply (approval prompts, previews). Only a
/// full `cancel_playback` drops it.
pub const UNTAGGED: u64 = u64::MAX;

impl AudioEngine {
    pub fn start(cfg: &Config) -> Result<Self> {
        let mut echo_wire = None;
        let echo_module = if cfg.barge_in.aec {
            match load_assistant_echo_cancel(&cfg.audio) {
                Ok((id, wire)) => {
                    tracing::info!(id, "echo-cancel loaded");
                    echo_wire = Some(wire);
                    Some(id)
                }
                Err(err) => {
                    tracing::warn!(%err, "echo-cancel not loaded; barge-in has no AEC");
                    None
                }
            }
        } else {
            None
        };
        let aec = echo_module.is_some();
        let (samples_tx, samples_rx) = mpsc::channel::<Vec<i16>>(32);
        let (level_tx, level_rx) = watch::channel(0.0f32);
        let (aec_level_tx, aec_level_rx) = if aec {
            let (tx, rx) = watch::channel(0.0f32);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let (aec_samples_tx, aec_samples_rx) = if aec {
            let (tx, rx) = mpsc::channel::<Vec<i16>>(32);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let queued = Arc::new(AtomicUsize::new(0));
        let play_epoch = Arc::new(AtomicU64::new(0));
        let play = Arc::new(Mutex::new(PlayState {
            queue: crate::PlayQueue::default(),
            epoch: 0,
            cancelled_through: 0,
            queued: queued.clone(),
        }));
        let play_thread = play.clone();
        let (ready_tx, ready_rx) = oneshot::channel::<Result<pw::channel::Sender<PwCmd>>>();
        let audio_cfg = cfg.audio.clone();

        std::thread::Builder::new()
            .name("assistant-pipewire".into())
            .spawn(move || {
                if let Err(err) = pw_thread(
                    audio_cfg,
                    samples_tx,
                    level_tx,
                    aec_level_tx,
                    aec_samples_tx,
                    play_thread,
                    ready_tx,
                ) {
                    tracing::error!(%err, "pipewire thread exited");
                }
            })
            .context("spawn pipewire thread")?;

        let cmd = ready_rx
            .blocking_recv()
            .map_err(|_| anyhow::anyhow!("pipewire thread dropped"))??;

        Ok(Self {
            cmd,
            samples_rx: Mutex::new(Some(samples_rx)),
            levels: level_rx,
            aec_levels: aec_level_rx,
            aec_samples_rx: Mutex::new(aec_samples_rx),
            queued,
            play_epoch,
            play,
            echo_module,
            echo_wire,
        })
    }

    pub fn take_samples(&self) -> Option<mpsc::Receiver<Vec<i16>>> {
        self.samples_rx.lock().ok().and_then(|mut g| g.take())
    }

    pub fn levels(&self) -> watch::Receiver<f32> {
        self.levels.clone()
    }

    pub fn aec_levels(&self) -> Option<watch::Receiver<f32>> {
        self.aec_levels.clone()
    }

    pub fn take_aec_samples(&self) -> Option<mpsc::Receiver<Vec<i16>>> {
        self.aec_samples_rx.lock().ok().and_then(|mut g| g.take())
    }

    pub fn start_capture(&self) -> Result<()> {
        self.cmd
            .send(PwCmd::StartCapture)
            .map_err(|_| anyhow::anyhow!("pipewire command channel closed"))
    }

    pub fn stop_capture(&self) -> Result<()> {
        self.cmd
            .send(PwCmd::StopCapture)
            .map_err(|_| anyhow::anyhow!("pipewire command channel closed"))
    }

    /// Queue audio that isn't part of a reply.
    pub fn play(&self, pcm: Vec<f32>) -> Result<()> {
        self.play_phrase(UNTAGGED, 0, pcm)
    }

    /// Queue one phrase of a reply.
    pub fn play_phrase(&self, generation: u64, phrase: u32, pcm: Vec<f32>) -> Result<()> {
        let epoch = self.play_epoch.load(Ordering::SeqCst);
        self.cmd
            .send(PwCmd::QueuePlay { epoch, generation, phrase, pcm })
            .map_err(|_| anyhow::anyhow!("pipewire command channel closed"))
    }

    fn play_state(&self) -> std::sync::MutexGuard<'_, PlayState> {
        self.play.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drop the queued audio of `generation` and older replies, now and on arrival.
    pub fn cancel_generation(&self, generation: u64) {
        let t0 = Instant::now();
        let mut st = self.play_state();
        st.cancelled_through = st.cancelled_through.max(generation);
        st.queue.cancel_through(generation);
        st.queued.store(st.queue.queued(), Ordering::Relaxed);
        tracing::info!(generation, us = t0.elapsed().as_micros(), "generation cancelled");
    }

    /// Ramp the playback volume (1.0 normal; lower ducks the voice while someone speaks).
    pub fn set_gain(&self, gain: f32) {
        self.play_state().queue.set_gain(gain);
    }

    /// Hold the reply in place (silence, nothing lost) or carry on from where it stopped.
    pub fn set_paused(&self, paused: bool) {
        self.play_state().queue.set_paused(paused);
    }

    /// How far playback got.
    pub fn played(&self) -> crate::PlayedTo {
        self.play_state().queue.played()
    }

    /// The gaps (ms) heard between phrases of `generation`'s reply.
    pub fn take_gaps(&self, generation: u64) -> Vec<u32> {
        self.play_state().queue.take_gaps(generation)
    }

    pub fn cancel_playback(&self) -> Result<()> {
        let t0 = Instant::now();
        let epoch = self.play_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.queued.store(0, Ordering::Relaxed);
        self.cmd
            .send(PwCmd::CancelPlay { epoch })
            .map_err(|_| anyhow::anyhow!("pipewire command channel closed"))?;
        tracing::info!(us = t0.elapsed().as_micros(), "playback cancel queued");
        Ok(())
    }

    pub async fn wait_playback(&self) {
        let start = tokio::time::Instant::now();
        while self.queued.load(Ordering::Relaxed) > 0 {
            if start.elapsed() > tokio::time::Duration::from_secs(30) {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.cmd.send(PwCmd::Shutdown);
        if let Some(run) = self.echo_wire.take() {
            run.store(false, Ordering::Relaxed);
        }
        if let Some(id) = self.echo_module.take() {
            unload_echo_module(id);
        }
    }
}

const EC_SOURCE: &str = "assistant-echo-cancel";

fn load_assistant_echo_cancel(
    audio: &assistant_core::config::AudioCfg,
) -> Result<(u32, Arc<AtomicBool>)> {
    unload_assistant_echo_cancel();
    // Keep the session sink format (typically 48 kHz stereo).
    let source_arg = format!("source_master={}", audio.source);
    let sink_arg = format!("sink_master={}", audio.sink);
    let mut targets = Vec::new();
    if audio.source != "default" {
        targets.push(source_arg.as_str());
    }
    if audio.sink != "default" {
        targets.push(sink_arg.as_str());
    }
    let id = match pactl_load_echo_cancel(
        &targets,
        &[
            "source_name=assistant-echo-cancel",
            "aec_method=webrtc",
            // webrtc NS ducks the near-end ~20x; the HUD bar and STT then look dead.
            "aec_args=noise_suppression=0 voice_detection=0 gain_control=0",
            "sink_properties=priority.session=0",
        ],
    ) {
        Ok(id) => id,
        Err(err) => {
            tracing::warn!(%err, "echo-cancel extra args failed; retrying base args");
            pactl_load_echo_cancel(
                &targets,
                &[
                    "source_name=assistant-echo-cancel",
                    "aec_method=webrtc",
                    "aec_args=noise_suppression=0 voice_detection=0",
                ],
            )?
        }
    };
    let run = Arc::new(AtomicBool::new(true));
    let run_thread = run.clone();
    let sink_target = audio.sink.clone();
    std::thread::Builder::new()
        .name("assistant-aec-wire".into())
        .spawn(move || {
            let mut pass = 0u32;
            while run_thread.load(Ordering::Relaxed) {
                let ms = if pass == 0 { 150 } else { 2_000 };
                std::thread::sleep(std::time::Duration::from_millis(ms));
                if !run_thread.load(Ordering::Relaxed) {
                    break;
                }
                match wire_echo_cancel_monitor(&sink_target) {
                    Ok(()) => {
                        if pass < 2 {
                            tracing::info!(pass, "echo-cancel using sink monitor");
                        }
                    }
                    Err(err) => tracing::warn!(pass, %err, "echo-cancel monitor wiring"),
                }
                pass = pass.saturating_add(1);
            }
        })
        .context("spawn aec wire thread")?;
    Ok((id, run))
}

/// Floor on the cancelled stream: above residual TTS leak, below speech.
pub fn aec_barge_threshold(residual: f32, configured: f32) -> f32 {
    let t = residual.mul_add(1.4, 0.005);
    t.clamp(0.016, configured.max(0.016))
}

fn pactl_load_echo_cancel(targets: &[&str], extra: &[&str]) -> Result<u32> {
    let mut args = vec!["load-module", "module-echo-cancel"];
    args.extend_from_slice(targets);
    args.extend_from_slice(extra);
    let out = std::process::Command::new("pactl")
        .args(&args)
        .output()
        .context("pactl load-module echo-cancel")?;
    if !out.status.success() {
        anyhow::bail!(
            "pactl load-module echo-cancel: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u32>()
        .context("echo-cancel module id")
}

fn default_sink_name() -> Result<String> {
    let out = std::process::Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .context("pactl get-default-sink")?;
    anyhow::ensure!(out.status.success(), "pactl get-default-sink failed");
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() {
        anyhow::bail!("pactl get-default-sink empty");
    }
    Ok(name)
}

fn default_source_name() -> Option<String> {
    let out = std::process::Command::new("pactl")
        .args(["get-default-source"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() || name == EC_SOURCE || name.contains("echo-cancel") {
        return None;
    }
    Some(name)
}

/// Pulse `monitor_mode` still creates an idle sink and plays it to the speakers,
/// so AEC has no reference. Disconnect that playback and feed the default sink
/// monitor into the canceller instead.
fn wire_echo_cancel_monitor(target: &str) -> Result<()> {
    let sink = if target == "default" {
        default_sink_name()?
    } else {
        target.to_string()
    };
    let outputs = pw_ports("-o");
    let inputs = pw_ports("-i");
    for out in outputs
        .iter()
        .filter(|p| p.starts_with("echo-cancel-playback:"))
    {
        for inp in inputs.iter().filter(|p| p.starts_with(&format!("{sink}:"))) {
            let _ = pw_link(&["-d", out, inp]);
        }
    }
    let monitors: Vec<String> = outputs
        .into_iter()
        .filter(|p| p.starts_with(&format!("{sink}:monitor_")))
        .collect();
    let playback: Vec<String> = inputs
        .into_iter()
        .filter(|p| p.starts_with("echo-cancel-sink:playback_"))
        .collect();
    if monitors.is_empty() || playback.is_empty() {
        anyhow::bail!("no monitor links to echo-cancel-sink from {sink}");
    }
    let mut linked = 0u32;
    for (out, inp) in pair_monitor_ports(&monitors, &playback) {
        if pw_link(&["-L", &out, &inp]) {
            linked += 1;
        }
    }
    if linked == 0 {
        anyhow::bail!("no monitor links to echo-cancel-sink from {sink}");
    }
    Ok(())
}

fn pw_ports(flag: &str) -> Vec<String> {
    let Ok(out) = std::process::Command::new("pw-link").arg(flag).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn pw_link(args: &[&str]) -> bool {
    let Ok(out) = std::process::Command::new("pw-link").args(args).output() else {
        return false;
    };
    if out.status.success() {
        return true;
    }
    String::from_utf8_lossy(&out.stderr).contains("File exists")
}

fn pair_monitor_ports(monitors: &[String], playback: &[String]) -> Vec<(String, String)> {
    if playback.len() == 1 {
        return monitors
            .iter()
            .cloned()
            .map(|m| (m, playback[0].clone()))
            .collect();
    }
    let mut pairs = Vec::new();
    for m in monitors {
        let ch = m.rsplit('_').next().unwrap_or("");
        if let Some(p) = playback.iter().find(|p| p.ends_with(&format!("_{ch}"))) {
            pairs.push((m.clone(), p.clone()));
        }
    }
    if pairs.is_empty() {
        for (m, p) in monitors.iter().zip(playback.iter()) {
            pairs.push((m.clone(), p.clone()));
        }
    }
    pairs
}

fn unload_assistant_echo_cancel() {
    let Ok(out) = std::process::Command::new("pactl")
        .args(["list", "modules", "short"])
        .output()
    else {
        return;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if !line.contains("assistant-echo-cancel") {
            continue;
        }
        if let Some(id) = line.split_whitespace().next() {
            if let Ok(n) = id.parse::<u32>() {
                unload_echo_module(n);
            }
        }
    }
}

fn unload_echo_module(id: u32) {
    let _ = std::process::Command::new("pactl")
        .args(["unload-module", &id.to_string()])
        .status();
}

fn pw_thread(
    audio: assistant_core::config::AudioCfg,
    samples_tx: mpsc::Sender<Vec<i16>>,
    level_tx: watch::Sender<f32>,
    aec_level_tx: Option<watch::Sender<f32>>,
    aec_samples_tx: Option<mpsc::Sender<Vec<i16>>>,
    play: Arc<Mutex<PlayState>>,
    ready: oneshot::Sender<Result<pw::channel::Sender<PwCmd>>>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).context("pipewire MainLoop")?;
    let context = pw::context::ContextRc::new(&mainloop, None).context("pipewire Context")?;
    let core = context.connect_rc(None).context("pipewire connect")?;

    let capture = Arc::new(Mutex::new(CaptureState {
        tx: samples_tx,
        level: level_tx,
        active: false,
        pending: Vec::new(),
        armed: None,
    }));

    // HUD / PTT PCM: hardware mic. AEC source is a second stream used only
    // for barge-in so TTS echo does not trip the energy detector.
    let source = if audio.source == "default" {
        default_source_name()
    } else {
        Some(audio.source)
    };
    let cap_props = match source {
        Some(src) => properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Communication",
            *pw::keys::NODE_NAME => "Assistant",
            *pw::keys::NODE_DESCRIPTION => "Assistant microphone",
            "target.object" => src.as_str(),
        },
        None => properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Communication",
            *pw::keys::NODE_NAME => "Assistant",
            *pw::keys::NODE_DESCRIPTION => "Assistant microphone",
        },
    };
    let mut play_props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Communication",
        *pw::keys::NODE_NAME => "Assistant",
        *pw::keys::NODE_DESCRIPTION => "Assistant speaker",
    };
    if audio.sink != "default" {
        play_props.insert("target.object", audio.sink.as_str());
    }
    let cap_stream = pw::stream::StreamBox::new(&core, "assistant-capture", cap_props)
        .context("capture stream")?;

    let play_stream = pw::stream::StreamBox::new(&core, "assistant-playback", play_props)
        .context("playback stream")?;

    let cap_data = capture.clone();
    let _cap_listener = cap_stream
        .add_local_listener_with_user_data(())
        .state_changed(|_, _, old, new| {
            tracing::debug!(?old, ?new, "capture state");
        })
        .process(move |stream, _| {
            let mut st = cap_data.lock().unwrap_or_else(|e| e.into_inner());
            if !st.active {
                if let Some(buf) = stream.dequeue_buffer() {
                    drop(buf);
                }
                return;
            }
            let Some(mut buf) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buf.datas_mut();
            if datas.is_empty() {
                return;
            }
            let data = &mut datas[0];
            let n = data.chunk().size() as usize;
            let Some(slice) = data.data() else {
                return;
            };
            let bytes = &slice[..n.min(slice.len())];
            let mut i = 0;
            while i + 1 < bytes.len() {
                let s = i16::from_le_bytes([bytes[i], bytes[i + 1]]);
                st.pending.push(s);
                i += 2;
            }
            while st.pending.len() >= FRAME {
                let frame: Vec<i16> = st.pending.drain(..FRAME).collect();
                let mut acc = 0.0f32;
                for s in &frame {
                    let x = *s as f32 / 32768.0;
                    acc += x * x;
                }
                let rms = (acc / frame.len() as f32).sqrt();
                let _ = st.level.send(rms);
                if let Some(t0) = st.armed.take() {
                    tracing::info!(ms = t0.elapsed().as_millis(), "capture first frame");
                }
                let _ = st.tx.try_send(frame);
            }
        })
        .register()
        .context("capture listener")?;

    let aec_stream = if aec_level_tx.is_some() {
        let props = properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Communication",
            *pw::keys::NODE_NAME => "Assistant-AEC",
            *pw::keys::NODE_DESCRIPTION => "Assistant echo-cancel",
            "target.object" => EC_SOURCE,
        };
        Some(pw::stream::StreamBox::new(&core, "assistant-aec", props).context("aec stream")?)
    } else {
        None
    };
    let aec_pcm = aec_samples_tx.map(|tx| {
        Arc::new(Mutex::new(AecCapture {
            tx,
            pending: Vec::new(),
        }))
    });
    let _aec_listener = if let (Some(stream), Some(tx)) = (aec_stream.as_ref(), aec_level_tx) {
        let pcm = aec_pcm.clone();
        Some(
            stream
                .add_local_listener_with_user_data(())
                .process(move |stream, _| {
                    let Some(mut buf) = stream.dequeue_buffer() else {
                        return;
                    };
                    let datas = buf.datas_mut();
                    if datas.is_empty() {
                        return;
                    }
                    let data = &mut datas[0];
                    let n = data.chunk().size() as usize;
                    let Some(slice) = data.data() else {
                        return;
                    };
                    let bytes = &slice[..n.min(slice.len())];
                    let mut acc = 0.0f32;
                    let mut count = 0u32;
                    let mut i = 0;
                    let mut samples = Vec::new();
                    while i + 1 < bytes.len() {
                        let s = i16::from_le_bytes([bytes[i], bytes[i + 1]]);
                        let x = s as f32 / 32768.0;
                        acc += x * x;
                        count += 1;
                        samples.push(s);
                        i += 2;
                    }
                    if count > 0 {
                        let _ = tx.send((acc / count as f32).sqrt());
                    }
                    if let Some(pcm) = &pcm {
                        if let Ok(mut st) = pcm.lock() {
                            st.pending.extend_from_slice(&samples);
                            while st.pending.len() >= FRAME {
                                let frame: Vec<i16> = st.pending.drain(..FRAME).collect();
                                let _ = st.tx.try_send(frame);
                            }
                        }
                    }
                })
                .register()
                .context("aec listener")?,
        )
    } else {
        None
    };

    let play_data = play.clone();
    let _play_listener = play_stream
        .add_local_listener_with_user_data(())
        .state_changed(|_, _, old, new| {
            tracing::debug!(?old, ?new, "playback state");
        })
        .process(move |stream, _| {
            let mut st = play_data.lock().unwrap_or_else(|e| e.into_inner());
            let Some(mut buf) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buf.datas_mut();
            if datas.is_empty() {
                return;
            }
            let data = &mut datas[0];
            let n = match data.data() {
                Some(bytes) => bytes.len() / 4,
                None => return,
            };
            let mut samples = vec![0.0f32; n];
            st.queue.pull(&mut samples);
            st.queued.store(st.queue.queued(), Ordering::Relaxed);
            if let Some(bytes) = data.data() {
                for (i, s) in samples.iter().enumerate() {
                    bytes[i * 4..i * 4 + 4].copy_from_slice(&s.to_le_bytes());
                }
            }
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = 4;
            *chunk.size_mut() = (n * 4) as u32;
        })
        .register()
        .context("playback listener")?;

    let cap_bytes = format_bytes(CAPTURE_RATE, spa::param::audio::AudioFormat::S16LE)?;
    let mut cap_params = [Pod::from_bytes(&cap_bytes).context("capture pod")?];
    cap_stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut cap_params,
        )
        .context("connect capture")?;

    if let Some(stream) = aec_stream.as_ref() {
        let aec_bytes = format_bytes(CAPTURE_RATE, spa::param::audio::AudioFormat::S16LE)?;
        let mut aec_params = [Pod::from_bytes(&aec_bytes).context("aec pod")?];
        stream
            .connect(
                spa::utils::Direction::Input,
                None,
                pw::stream::StreamFlags::AUTOCONNECT
                    | pw::stream::StreamFlags::MAP_BUFFERS
                    | pw::stream::StreamFlags::RT_PROCESS,
                &mut aec_params,
            )
            .context("connect aec")?;
    }

    let play_bytes = format_bytes(PLAYBACK_RATE, spa::param::audio::AudioFormat::F32LE)?;
    let mut play_params = [Pod::from_bytes(&play_bytes).context("playback pod")?];
    play_stream
        .connect(
            spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut play_params,
        )
        .context("connect playback")?;

    let (cmd_tx, cmd_rx) = pw::channel::channel::<PwCmd>();
    let cap_cmd = capture.clone();
    let play_cmd = play.clone();
    let loop_clone = mainloop.clone();
    let _attached = cmd_rx.attach(mainloop.loop_(), move |cmd| match cmd {
        PwCmd::StartCapture => {
            let mut st = cap_cmd.lock().unwrap_or_else(|e| e.into_inner());
            if !st.active {
                st.pending.clear();
                st.armed = Some(Instant::now());
                st.active = true;
            }
        }
        PwCmd::StopCapture => {
            cap_cmd.lock().unwrap_or_else(|e| e.into_inner()).active = false;
        }
        PwCmd::QueuePlay { epoch, generation, phrase, pcm } => {
            let mut st = play_cmd.lock().unwrap_or_else(|e| e.into_inner());
            if epoch != st.epoch || (generation != UNTAGGED && generation <= st.cancelled_through) {
                return;
            }
            st.queue.push(generation, phrase, pcm);
            let queued = st.queue.queued();
            st.queued.store(queued, Ordering::Relaxed);
        }
        PwCmd::CancelPlay { epoch } => {
            let t0 = Instant::now();
            let mut st = play_cmd.lock().unwrap_or_else(|e| e.into_inner());
            st.epoch = epoch;
            st.queue.clear();
            st.queue.set_gain(1.0);
            st.queue.set_paused(false);
            st.queued.store(0, Ordering::Relaxed);
            tracing::info!(us = t0.elapsed().as_micros(), "playback cancel flushed");
        }
        PwCmd::Shutdown => {
            loop_clone.quit();
        }
    });

    let _ = ready.send(Ok(cmd_tx));
    mainloop.run();
    Ok(())
}

fn format_bytes(rate: u32, format: spa::param::audio::AudioFormat) -> Result<Vec<u8>> {
    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(format);
    audio_info.set_rate(rate);
    audio_info.set_channels(1);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_MONO;
    audio_info.set_position(position);

    let values = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: audio_info.into(),
        }),
    )
    .map_err(|e| anyhow::anyhow!("serialize audio format: {e}"))?
    .0
    .into_inner();
    Ok(values)
}

/// Collect PCM until PTT key-up (`stop` becomes true). VAD is updated
/// for levels/logging but does not end a hold-to-talk take. `tap` sees each chunk as it
/// arrives (the streaming recognizer decodes while the key is held).
pub async fn record_until(
    samples: &mut mpsc::Receiver<Vec<i16>>,
    mut stop: tokio::sync::watch::Receiver<bool>,
    vad: &mut impl VadGate,
    max_secs: u64,
    tap: &(dyn Fn(&[i16]) + Sync),
) -> Result<(Vec<i16>, bool)> {
    let mut pcm = Vec::new();
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(max_secs.max(1));
    if *stop.borrow() {
        while let Ok(chunk) = samples.try_recv() {
            tap(&chunk);
            pcm.extend_from_slice(&chunk);
        }
        return Ok((pcm, false));
    }
    // Drop frames captured while we were speaking (barge-in monitor).
    while samples.try_recv().is_ok() {}
    loop {
        tokio::select! {
            _ = stop.changed() => {
                if *stop.borrow() {
                    break;
                }
            }
            chunk = samples.recv() => {
                let Some(chunk) = chunk else { break };
                let _ = vad.push(&chunk);
                tap(&chunk);
                pcm.extend_from_slice(&chunk);
            }
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }
    while let Ok(chunk) = samples.try_recv() {
        tap(&chunk);
        pcm.extend_from_slice(&chunk);
    }
    Ok((pcm, false))
}

/// Collect PCM until VAD reports speech end, cancel, or timeout.
/// `keep_pending` keeps frames already in the channel (the barge-in utterance).
pub async fn record_until_vad(
    samples: &mut mpsc::Receiver<Vec<i16>>,
    mut stop: tokio::sync::watch::Receiver<bool>,
    vad: &mut impl VadGate,
    max_secs: u64,
    keep_pending: bool,
) -> Result<(Vec<i16>, bool)> {
    use crate::VadDecision;
    let mut pcm = Vec::new();
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(max_secs.max(1));
    if *stop.borrow() {
        if keep_pending {
            while let Ok(chunk) = samples.try_recv() {
                let d = vad.push(&chunk);
                pcm.extend_from_slice(&chunk);
                if matches!(d, VadDecision::SpeechEnd) {
                    return Ok((pcm, true));
                }
            }
        }
        return Ok((pcm, false));
    }
    if keep_pending {
        while let Ok(chunk) = samples.try_recv() {
            let d = vad.push(&chunk);
            pcm.extend_from_slice(&chunk);
            if matches!(d, VadDecision::SpeechEnd) {
                return Ok((pcm, true));
            }
        }
    } else {
        while samples.try_recv().is_ok() {}
    }
    loop {
        tokio::select! {
            _ = stop.changed() => {
                if *stop.borrow() {
                    break;
                }
            }
            chunk = samples.recv() => {
                let Some(chunk) = chunk else { break };
                let d = vad.push(&chunk);
                pcm.extend_from_slice(&chunk);
                if matches!(d, VadDecision::SpeechEnd) {
                    return Ok((pcm, true));
                }
            }
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }
    while let Ok(chunk) = samples.try_recv() {
        pcm.extend_from_slice(&chunk);
    }
    Ok((pcm, false))
}

#[cfg(test)]
mod tests {
    use super::{aec_barge_threshold, pair_monitor_ports};

    #[test]
    fn aec_threshold_sits_above_tts_residual() {
        assert!((aec_barge_threshold(0.012, 0.08) - 0.0218).abs() < 1e-4);
        assert_eq!(aec_barge_threshold(0.001, 0.08), 0.016);
        assert_eq!(aec_barge_threshold(0.06, 0.08), 0.08);
    }

    #[test]
    fn pairs_stereo_by_channel_suffix() {
        let monitors = vec!["analog:monitor_FL".into(), "analog:monitor_FR".into()];
        let playback = vec![
            "echo-cancel-sink:playback_FL".into(),
            "echo-cancel-sink:playback_FR".into(),
        ];
        assert_eq!(
            pair_monitor_ports(&monitors, &playback),
            vec![
                (
                    "analog:monitor_FL".into(),
                    "echo-cancel-sink:playback_FL".into()
                ),
                (
                    "analog:monitor_FR".into(),
                    "echo-cancel-sink:playback_FR".into()
                ),
            ]
        );
    }

    #[test]
    fn mixes_stereo_monitors_into_mono_playback() {
        let monitors = vec!["analog:monitor_FL".into(), "analog:monitor_FR".into()];
        let playback = vec!["echo-cancel-sink:playback_MONO".into()];
        assert_eq!(
            pair_monitor_ports(&monitors, &playback),
            vec![
                (
                    "analog:monitor_FL".into(),
                    "echo-cancel-sink:playback_MONO".into()
                ),
                (
                    "analog:monitor_FR".into(),
                    "echo-cancel-sink:playback_MONO".into()
                ),
            ]
        );
    }
}
