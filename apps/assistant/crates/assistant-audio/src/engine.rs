//! PipeWire capture (16 kHz S16 mono) and playback (24 kHz F32 mono).

use crate::vad::EnergyVad;
use anyhow::{Context, Result};
use assistant_core::config::Config;
use pipewire as pw;
use pw::spa::pod::Pod;
use pw::{properties::properties, spa};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot, watch};

const CAPTURE_RATE: u32 = 16_000;
const PLAYBACK_RATE: u32 = 24_000;
const FRAME: usize = 512;

enum PwCmd {
    StartCapture,
    StopCapture,
    QueuePlay(Vec<f32>),
    CancelPlay,
    Shutdown,
}

pub struct AudioEngine {
    cmd: pw::channel::Sender<PwCmd>,
    samples_rx: Mutex<Option<mpsc::Receiver<Vec<i16>>>>,
    levels: watch::Receiver<f32>,
    queued: Arc<AtomicUsize>,
}

struct CaptureState {
    tx: mpsc::Sender<Vec<i16>>,
    level: watch::Sender<f32>,
    active: bool,
    pending: Vec<i16>,
}

struct PlayState {
    queue: Vec<f32>,
    cancelled: bool,
    queued: Arc<AtomicUsize>,
}

impl AudioEngine {
    pub fn start(_cfg: &Config) -> Result<Self> {
        let (samples_tx, samples_rx) = mpsc::channel::<Vec<i16>>(32);
        let (level_tx, level_rx) = watch::channel(0.0f32);
        let queued = Arc::new(AtomicUsize::new(0));
        let (ready_tx, ready_rx) = oneshot::channel::<Result<pw::channel::Sender<PwCmd>>>();
        let queued_thread = queued.clone();

        std::thread::Builder::new()
            .name("nyx-pipewire".into())
            .spawn(move || {
                if let Err(err) = pw_thread(samples_tx, level_tx, queued_thread, ready_tx) {
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
            queued,
        })
    }

    pub fn take_samples(&self) -> Option<mpsc::Receiver<Vec<i16>>> {
        self.samples_rx.lock().ok().and_then(|mut g| g.take())
    }

    pub fn levels(&self) -> watch::Receiver<f32> {
        self.levels.clone()
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

    pub fn play(&self, pcm: Vec<f32>) -> Result<()> {
        self.queued.fetch_add(pcm.len(), Ordering::Relaxed);
        self.cmd
            .send(PwCmd::QueuePlay(pcm))
            .map_err(|_| anyhow::anyhow!("pipewire command channel closed"))
    }

    pub fn cancel_playback(&self) -> Result<()> {
        self.queued.store(0, Ordering::Relaxed);
        self.cmd
            .send(PwCmd::CancelPlay)
            .map_err(|_| anyhow::anyhow!("pipewire command channel closed"))
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
    }
}

fn pw_thread(
    samples_tx: mpsc::Sender<Vec<i16>>,
    level_tx: watch::Sender<f32>,
    queued: Arc<AtomicUsize>,
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
    }));
    let play = Arc::new(Mutex::new(PlayState {
        queue: Vec::new(),
        cancelled: false,
        queued: queued.clone(),
    }));

    let cap_stream = pw::stream::StreamBox::new(
        &core,
        "nyx-capture",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Communication",
            *pw::keys::NODE_NAME => "Nyx",
            *pw::keys::NODE_DESCRIPTION => "Nyx microphone",
        },
    )
    .context("capture stream")?;

    let play_stream = pw::stream::StreamBox::new(
        &core,
        "nyx-playback",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::MEDIA_ROLE => "Communication",
            *pw::keys::NODE_NAME => "Nyx",
            *pw::keys::NODE_DESCRIPTION => "Nyx speaker",
        },
    )
    .context("playback stream")?;

    let cap_data = capture.clone();
    let _cap_listener = cap_stream
        .add_local_listener_with_user_data(())
        .state_changed(|_, _, old, new| {
            tracing::debug!(?old, ?new, "capture state");
        })
        .process(move |stream, _| {
            let mut st = cap_data.lock().unwrap();
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
                let _ = st.tx.try_send(frame);
            }
        })
        .register()
        .context("capture listener")?;

    let play_data = play.clone();
    let _play_listener = play_stream
        .add_local_listener_with_user_data(())
        .state_changed(|_, _, old, new| {
            tracing::debug!(?old, ?new, "playback state");
        })
        .process(move |stream, _| {
            let mut st = play_data.lock().unwrap();
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
            let cancelled = st.cancelled;
            if cancelled {
                st.queue.clear();
                st.cancelled = false;
                st.queued.store(0, Ordering::Relaxed);
            }
            let take = if cancelled { 0 } else { n.min(st.queue.len()) };
            let samples: Vec<f32> = if take > 0 {
                st.queue.drain(..take).collect()
            } else {
                Vec::new()
            };
            st.queued.store(st.queue.len(), Ordering::Relaxed);
            if let Some(bytes) = data.data() {
                for i in 0..n {
                    let s = samples.get(i).copied().unwrap_or(0.0);
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
            let mut st = cap_cmd.lock().unwrap();
            st.pending.clear();
            st.active = true;
        }
        PwCmd::StopCapture => {
            cap_cmd.lock().unwrap().active = false;
        }
        PwCmd::QueuePlay(pcm) => {
            let mut st = play_cmd.lock().unwrap();
            st.cancelled = false;
            st.queue.extend(pcm);
            st.queued.store(st.queue.len(), Ordering::Relaxed);
        }
        PwCmd::CancelPlay => {
            let mut st = play_cmd.lock().unwrap();
            st.queue.clear();
            st.cancelled = true;
            st.queued.store(0, Ordering::Relaxed);
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

/// Collect PCM until PTT key-up (`stop` becomes true). Energy VAD is updated
/// for levels/logging but does not end a hold-to-talk take.
pub async fn record_until(
    samples: &mut mpsc::Receiver<Vec<i16>>,
    mut stop: tokio::sync::watch::Receiver<bool>,
    vad: &mut EnergyVad,
    max_secs: u64,
) -> Result<(Vec<i16>, bool)> {
    let mut pcm = Vec::new();
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(max_secs.max(1));
    if *stop.borrow() {
        while let Ok(chunk) = samples.try_recv() {
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
                pcm.extend_from_slice(&chunk);
            }
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }
    while let Ok(chunk) = samples.try_recv() {
        pcm.extend_from_slice(&chunk);
    }
    Ok((pcm, false))
}

/// Collect PCM until energy VAD reports speech end, cancel, or timeout.
/// `keep_pending` keeps frames already in the channel (the barge-in utterance).
pub async fn record_until_vad(
    samples: &mut mpsc::Receiver<Vec<i16>>,
    mut stop: tokio::sync::watch::Receiver<bool>,
    vad: &mut crate::EnergyVad,
    max_secs: u64,
    keep_pending: bool,
) -> Result<(Vec<i16>, bool)> {
    use crate::VadDecision;
    let mut pcm = Vec::new();
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(max_secs.max(1));
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
