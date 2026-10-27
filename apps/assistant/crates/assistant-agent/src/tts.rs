use anyhow::{Context, Result};
use assistant_core::config::Config;
use assistant_core::paths;
use loqui::{Engine, Format, KokoroVariant, SpeakRequest, TtsConfig, TtsDevice};
use std::io::{BufRead, BufReader, Cursor, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

#[derive(Clone, PartialEq, Eq)]
struct CloneSpec { reference: PathBuf }

struct CloneWorker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Drop for CloneWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl CloneWorker {
    fn start() -> Result<Self> {
        let root = paths::data_dir().join("voice-engines/chatterbox");
        let python = root.join(".venv/bin/python");
        let script = root.join("worker.py");
        anyhow::ensure!(python.is_file() && script.is_file(), "Chatterbox runtime is not installed");
        let mut child = Command::new(python)
            .arg(script)
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .spawn().context("start local Chatterbox worker")?;
        let input = child.stdin.take().context("Chatterbox stdin")?;
        let output = BufReader::new(child.stdout.take().context("Chatterbox stdout")?);
        Ok(Self { child, input, output })
    }

    fn speak(&mut self, text: &str, spec: &CloneSpec) -> Result<Vec<f32>> {
        std::fs::create_dir_all(paths::runtime_dir())?;
        let output = tempfile::NamedTempFile::new_in(paths::runtime_dir())?;
        let request = serde_json::json!({"text":text,"reference":spec.reference,"output":output.path()});
        writeln!(self.input, "{request}")?;
        self.input.flush()?;
        let mut fd = libc::pollfd { fd: self.output.get_ref().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: fd points to the child stdout; poll only reads the descriptor state.
        anyhow::ensure!(unsafe { libc::poll(&mut fd, 1, 120_000) } > 0, "Chatterbox response timed out");
        let mut line = String::new();
        anyhow::ensure!(self.output.read_line(&mut line)? > 0, "Chatterbox worker exited");
        let response: serde_json::Value = serde_json::from_str(&line).context("Chatterbox response")?;
        anyhow::ensure!(response["ok"] == true, "Chatterbox: {}", response["error"].as_str().unwrap_or("unknown failure"));
        anyhow::ensure!(response["sample_rate"] == 24_000, "Chatterbox sample rate changed");
        let bytes = std::fs::read(output.path())?;
        anyhow::ensure!(bytes.len() % 4 == 0, "Chatterbox PCM is malformed");
        Ok(bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
}

fn cuda_skip_stamp() -> std::path::PathBuf {
    paths::cache_dir().join("kokoro-cuda-skip")
}

fn parse_variant(s: &str) -> KokoroVariant {
    match s.to_ascii_lowercase().as_str() {
        "quantized" | "int8" | "q8" => KokoroVariant::Quantized,
        "fp16" => KokoroVariant::Fp16,
        _ => KokoroVariant::Fp32,
    }
}

/// ORT CUDA EP has no Blackwell (sm_120) kernels. Skip the load+probe that always fails unless forced.
fn skip_cuda_tts(force: bool) -> bool {
    if force {
        return false;
    }
    if cuda_skip_stamp().exists() {
        return true;
    }
    let Ok(out) = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=compute_cap", "--format=csv,noheader"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.trim().starts_with("12."))
}

fn remember_cuda_tts_broken() {
    let _ = std::fs::create_dir_all(paths::cache_dir());
    let _ = std::fs::write(cuda_skip_stamp(), b"ort-cuda has no sm_120 kernels\n");
}

fn load_engine(
    cache: &Path,
    device: TtsDevice,
    variant: KokoroVariant,
    threads: Option<usize>,
) -> Result<Engine> {
    Engine::builder()
        .cache_dir(cache)
        .tts(Some(TtsConfig {
            device,
            variant,
            threads,
            ..TtsConfig::default()
        }))
        .speaker(None)
        .preload(true)
        .build()
        .context("loqui Kokoro engine")
}

/// ORT CUDA can load on Blackwell then fail at first infer (no sm_120 kernels).
fn probe_speak(engine: &Engine, voice: &str) -> Result<()> {
    let mut req = SpeakRequest::new("ok");
    req.voice = voice.to_string();
    req.speed = 1.0;
    req.format = Format::Wav;
    engine.speak(&req).map(|_| ()).context("kokoro cuda probe")
}

pub struct KokoroProvider {
    engine: Engine,
    voice: std::sync::RwLock<String>,
    speed: std::sync::RwLock<f32>,
    device: String,
    variant: String,
    lock: Mutex<()>,
    clone_spec: std::sync::RwLock<Option<CloneSpec>>,
    clone_worker: Mutex<Option<CloneWorker>>,
}

impl KokoroProvider {
    pub fn load(cfg: &Config) -> Result<Self> {
        let cache = paths::cache_dir().join("loqui");
        std::fs::create_dir_all(&cache)?;
        let variant = parse_variant(&cfg.tts.variant);
        let threads = cfg.tts.threads;
        tracing::info!(
            variant = ?variant,
            threads = ?threads,
            device = %cfg.tts.device,
            force_cuda = cfg.tts.force_cuda,
            "loading Kokoro (this downloads weights on first run)"
        );
        let t0 = std::time::Instant::now();
        let want_cuda = cfg.tts.device != "cpu" && !skip_cuda_tts(cfg.tts.force_cuda);
        let (engine, active_device) = if want_cuda {
            match load_engine(&cache, TtsDevice::Cuda(0), variant, threads) {
                Ok(engine) => match probe_speak(&engine, &cfg.tts.voice) {
                    Ok(()) => (engine, "cuda:0".to_string()),
                    Err(err) => {
                        tracing::warn!(
                            %err,
                            "Kokoro CUDA loaded but cannot infer on this GPU; using CPU"
                        );
                        remember_cuda_tts_broken();
                        drop(engine);
                        let eng = load_engine(&cache, TtsDevice::Cpu, variant, threads)
                            .context("loqui Kokoro CPU fallback")?;
                        (eng, "cpu (cuda probe failed)".to_string())
                    }
                },
                Err(err) => {
                    tracing::warn!(%err, "Kokoro CUDA failed; falling back to CPU (needs libcudnn for CUDA)");
                    let eng = load_engine(&cache, TtsDevice::Cpu, variant, threads)
                        .context("loqui Kokoro CPU fallback")?;
                    (eng, "cpu (cuda init failed)".to_string())
                }
            }
        } else {
            if cfg.tts.device != "cpu" && !cfg.tts.force_cuda {
                tracing::info!(
                    "Kokoro CUDA skipped on this GPU (no sm_120 ORT kernels); using CPU"
                );
            }
            let eng = load_engine(&cache, TtsDevice::Cpu, variant, threads)
                .context("loqui Kokoro CPU")?;
            (eng, "cpu".to_string())
        };
        tracing::info!(ms = t0.elapsed().as_millis(), active_device = %active_device, "Kokoro ready");
        let provider = Self {
            engine,
            voice: std::sync::RwLock::new(cfg.tts.voice.clone()),
            speed: std::sync::RwLock::new(cfg.tts.speed),
            device: active_device,
            variant: cfg.tts.variant.clone(),
            lock: Mutex::new(()),
            clone_spec: std::sync::RwLock::new(None),
            clone_worker: Mutex::new(None),
        };
        provider.configure(cfg)?;
        Ok(provider)
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    pub fn variant(&self) -> &str {
        &self.variant
    }

    pub fn voice(&self) -> String {
        self.voice
            .read()
            .map(|v| v.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    pub fn active_backend(&self) -> &'static str {
        if self.clone_spec.read().unwrap_or_else(|e| e.into_inner()).is_some() { "chatterbox" } else { "kokoro" }
    }

    pub fn set_voice(&self, voice: &str, speed: f32) {
        if let Ok(mut v) = self.voice.write() {
            *v = voice.to_string();
        }
        if let Ok(mut s) = self.speed.write() {
            *s = speed;
        }
    }

    /// A clone is selected only by an explicit persona profile. An installed voice override
    /// always selects Kokoro and unloads any cloned-voice worker.
    pub fn configure(&self, cfg: &Config) -> Result<()> {
        self.set_voice(&cfg.tts.voice, cfg.tts.speed);
        let next = if cfg.tts.voice_override.is_empty() && !cfg.assistant.persona_id.is_empty() {
            assistant_core::VoiceProfile::load(&cfg.assistant.persona_id)?
                .filter(|profile| profile.provider == "chatterbox")
                .and_then(|profile| profile.reference_audio.map(|reference| CloneSpec { reference }))
        } else { None };
        let mut spec = self.clone_spec.write().unwrap_or_else(|e| e.into_inner());
        if *spec != next {
            *self.clone_worker.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *spec = next;
        }
        Ok(())
    }

    pub fn speak(&self, text: &str) -> Result<Vec<f32>> {
        let clone = self.clone_spec.read().unwrap_or_else(|e| e.into_inner()).clone();
        let clone_requested = clone.is_some();
        if let Some(spec) = clone {
            let mut worker = self.clone_worker.lock().unwrap_or_else(|e| e.into_inner());
            let result = (|| {
                if worker.is_none() { *worker = Some(CloneWorker::start()?); }
                worker.as_mut().unwrap().speak(text, &spec)
            })();
            match result {
                Ok(audio) => return Ok(audio),
                Err(error) => {
                    tracing::warn!(%error, "cloned voice unavailable; using Kokoro fallback");
                    *worker = None;
                }
            }
        }
        let result = self.speak_with(
            text,
            &self.voice(),
            *self.speed.read().unwrap_or_else(|e| e.into_inner()),
        );
        if clone_requested && result.is_err() && self.voice() != "af_heart" {
            tracing::warn!("persona Kokoro fallback unavailable; using af_heart");
            return self.speak_with(text, "af_heart", 1.0);
        }
        result
    }
    pub fn speak_with(&self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>> {
        if !speed.is_finite() || !(0.5..=2.0).contains(&speed) {
            anyhow::bail!("invalid preview speed");
        }
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());

        let mut req = SpeakRequest::new(text);
        req.voice = voice.into();
        req.speed = speed;
        req.format = Format::Wav;
        let speech = self
            .engine
            .speak(&req)
            .with_context(|| format!("kokoro speak: {text}"))?;
        wav_to_f32(&speech.audio)
    }
}

fn wav_to_f32(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" {
        let cursor = Cursor::new(bytes.to_vec());
        let mut reader = hound::WavReader::new(cursor).context("kokoro wav")?;
        let spec = reader.spec();
        match spec.sample_format {
            hound::SampleFormat::Float => {
                let samples: Result<Vec<f32>, _> = reader.samples::<f32>().collect();
                samples.context("read f32 wav")
            }
            hound::SampleFormat::Int => {
                let samples: Result<Vec<i16>, _> = reader.samples::<i16>().collect();
                Ok(samples?.into_iter().map(|s| s as f32 / 32768.0).collect())
            }
        }
    } else {
        if !bytes.len().is_multiple_of(4) {
            anyhow::bail!("unexpected kokoro pcm size {}", bytes.len());
        }
        let (chunks, _) = bytes.as_chunks::<4>();
        let mut out = Vec::with_capacity(chunks.len());
        for &c in chunks {
            out.push(f32::from_le_bytes(c));
        }
        Ok(out)
    }
}
