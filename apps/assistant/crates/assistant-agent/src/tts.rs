use anyhow::{Context, Result};
use assistant_core::config::Config;
use assistant_core::paths;
use loqui::{Engine, Format, KokoroVariant, SpeakRequest, TtsConfig, TtsDevice};
use std::io::Cursor;
use std::path::Path;
use std::sync::Mutex;

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
        Ok(Self {
            engine,
            voice: std::sync::RwLock::new(cfg.tts.voice.clone()),
            speed: std::sync::RwLock::new(cfg.tts.speed),
            device: active_device,
            variant: cfg.tts.variant.clone(),
            lock: Mutex::new(()),
        })
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

    pub fn set_voice(&self, voice: &str, speed: f32) {
        if let Ok(mut v) = self.voice.write() {
            *v = voice.to_string();
        }
        if let Ok(mut s) = self.speed.write() {
            *s = speed;
        }
    }

    pub fn speak(&self, text: &str) -> Result<Vec<f32>> {
        self.speak_with(
            text,
            &self.voice(),
            *self.speed.read().unwrap_or_else(|e| e.into_inner()),
        )
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
