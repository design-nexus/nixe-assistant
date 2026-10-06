use anyhow::{Context, Result};
use assistant_core::config::Config;
use assistant_core::paths;
use loqui::{Engine, Format, SpeakRequest, TtsConfig, TtsDevice};
use std::io::Cursor;
use std::path::Path;
use std::sync::Mutex;

fn cuda_skip_stamp() -> std::path::PathBuf {
    paths::cache_dir().join("kokoro-cuda-skip")
}

/// ORT CUDA EP has no Blackwell (sm_120) kernels. Skip the load+probe that always fails.
fn skip_cuda_tts() -> bool {
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

fn load_engine(cache: &Path, device: TtsDevice) -> Result<Engine> {
    Engine::builder()
        .cache_dir(cache)
        .tts(Some(TtsConfig {
            device,
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
    voice: String,
    speed: f32,
    lock: Mutex<()>,
}

impl KokoroProvider {
    pub fn load(cfg: &Config) -> Result<Self> {
        let cache = paths::cache_dir().join("loqui");
        std::fs::create_dir_all(&cache)?;
        tracing::info!("loading Kokoro (this downloads weights on first run)");
        let t0 = std::time::Instant::now();
        let want_cuda = cfg.tts.device != "cpu" && !skip_cuda_tts();
        let engine = if want_cuda {
            match load_engine(&cache, TtsDevice::Cuda(0)) {
                Ok(engine) => match probe_speak(&engine, &cfg.tts.voice) {
                    Ok(()) => engine,
                    Err(err) => {
                        tracing::warn!(
                            %err,
                            "Kokoro CUDA loaded but cannot infer on this GPU; using CPU"
                        );
                        remember_cuda_tts_broken();
                        drop(engine);
                        load_engine(&cache, TtsDevice::Cpu)
                            .context("loqui Kokoro CPU fallback")?
                    }
                },
                Err(err) => {
                    tracing::warn!(%err, "Kokoro CUDA failed; falling back to CPU (needs libcudnn for CUDA)");
                    load_engine(&cache, TtsDevice::Cpu).context("loqui Kokoro CPU fallback")?
                }
            }
        } else {
            if cfg.tts.device != "cpu" {
                tracing::info!("Kokoro CUDA skipped on this GPU (no sm_120 ORT kernels); using CPU");
            }
            load_engine(&cache, TtsDevice::Cpu).context("loqui Kokoro CPU")?
        };
        tracing::info!(ms = t0.elapsed().as_millis(), "Kokoro ready");
        Ok(Self {
            engine,
            voice: cfg.tts.voice.clone(),
            speed: cfg.tts.speed,
            lock: Mutex::new(()),
        })
    }

    pub fn speak(&self, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let _g = self.lock.lock().unwrap();
        let mut req = SpeakRequest::new(text);
        req.voice = self.voice.clone();
        req.speed = self.speed;
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
                Ok(samples?
                    .into_iter()
                    .map(|s| s as f32 / 32768.0)
                    .collect())
            }
        }
    } else {
        if bytes.len() % 4 != 0 {
            anyhow::bail!("unexpected kokoro pcm size {}", bytes.len());
        }
        let mut out = Vec::with_capacity(bytes.len() / 4);
        for c in bytes.chunks_exact(4) {
            out.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
        Ok(out)
    }
}
