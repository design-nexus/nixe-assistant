//! Silero VAD v5 on ONNX Runtime CPU, sharing loqui's `ort` crate.

use anyhow::{Context, Result};
use assistant_core::config::VadCfg;
use assistant_core::paths;
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

const CHUNK: usize = 512;
const CONTEXT: usize = 64;
const WINDOW: usize = CONTEXT + CHUNK; // 576
const SR: i64 = 16_000;

#[derive(Clone, Debug)]
pub struct SileroState {
    pub h: Vec<f32>,
    pub context: Vec<f32>,
}

impl SileroState {
    pub fn new() -> Self {
        Self {
            h: vec![0.0; 256],
            context: vec![0.0; CONTEXT],
        }
    }

    pub fn reset(&mut self) {
        self.h.fill(0.0);
        self.context.fill(0.0);
    }
}

impl Default for SileroState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Silero {
    session: Mutex<Session>,
    input_audio: String,
    input_state: String,
    input_sr: Option<String>,
    output_prob: String,
    output_state: String,
    state_len: usize,
}

impl Silero {
    pub fn load(cfg: &VadCfg) -> Result<Self> {
        let path = model_path(cfg);
        ensure_model(&path, &cfg.url)?;
        let session = Session::builder()
            .map_err(|e| anyhow::anyhow!("silero session builder: {e}"))?
            .with_intra_threads(1)
            .map_err(|e| anyhow::anyhow!("silero threads: {e}"))?
            .commit_from_file(&path)
            .map_err(|e| anyhow::anyhow!("silero onnx {}: {e}", path.display()))?;

        let in_names: Vec<String> = session
            .inputs()
            .iter()
            .map(|o| o.name().to_string())
            .collect();
        let out_names: Vec<String> = session
            .outputs()
            .iter()
            .map(|o| o.name().to_string())
            .collect();
        tracing::info!(?in_names, ?out_names, path = %path.display(), "silero session");

        let input_audio = pick_name(&in_names, &["input", "audio", "x"])
            .cloned()
            .unwrap_or_else(|| in_names.first().cloned().unwrap_or_else(|| "input".into()));
        let input_state = pick_name(&in_names, &["state", "h", "hn"]).cloned();
        let input_sr = pick_name(&in_names, &["sr", "sampling_rate"]).cloned();
        let output_prob = pick_name(&out_names, &["output", "prob", "vad"])
            .cloned()
            .unwrap_or_else(|| {
                out_names
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "output".into())
            });
        let output_state = pick_name(&out_names, &["stateN", "state", "hn", "n"]).cloned();

        let Some(input_state) = input_state else {
            anyhow::bail!("silero model has no state input (unexpected graph)");
        };
        let Some(output_state) = output_state else {
            anyhow::bail!("silero model has no state output (unexpected graph)");
        };

        let state_len = 256;
        Ok(Self {
            session: Mutex::new(session),
            input_audio,
            input_state,
            input_sr,
            output_prob,
            output_state,
            state_len,
        })
    }

    pub fn chunk_len(&self) -> usize {
        CHUNK
    }

    pub fn new_state(&self) -> SileroState {
        SileroState::new()
    }

    pub fn infer(&self, frame: &[i16], state: &mut SileroState) -> Result<f32> {
        let mut audio = vec![0.0f32; WINDOW];
        // Prepend rolling context
        let ctx_len = state.context.len().min(CONTEXT);
        audio[..ctx_len].copy_from_slice(&state.context[..ctx_len]);

        // Copy incoming frame
        let n = frame.len().min(CHUNK);
        for i in 0..n {
            audio[CONTEXT + i] = frame[i] as f32 / 32768.0;
        }

        // Save tail for next frame
        state
            .context
            .copy_from_slice(&audio[WINDOW - CONTEXT..WINDOW]);

        if state.h.len() != self.state_len {
            state.h = vec![0.0; self.state_len];
        }

        let audio_t =
            Tensor::from_array(([1usize, WINDOW], audio)).context("silero audio tensor")?;
        let state_t = Tensor::from_array(([2usize, 1, 128], state.h.clone()))
            .context("silero state tensor")?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("silero session lock"))?;
        let outputs = if let Some(sr_name) = &self.input_sr {
            let sr = Tensor::from_array(([1usize], vec![SR])).context("silero sr tensor")?;
            session
                .run(ort::inputs![
                    self.input_audio.as_str() => audio_t,
                    self.input_state.as_str() => state_t,
                    sr_name.as_str() => sr
                ])
                .context("silero run")?
        } else {
            session
                .run(ort::inputs![
                    self.input_audio.as_str() => audio_t,
                    self.input_state.as_str() => state_t
                ])
                .context("silero run")?
        };
        let (_, prob) = outputs[self.output_prob.as_str()]
            .try_extract_tensor::<f32>()
            .context("silero prob")?;
        let p = prob.first().copied().unwrap_or(0.0);
        let (_, next) = outputs[self.output_state.as_str()]
            .try_extract_tensor::<f32>()
            .context("silero state out")?;
        if next.len() == self.state_len {
            state.h.copy_from_slice(next);
        }
        Ok(p)
    }
}

fn pick_name<'a>(names: &'a [String], want: &[&str]) -> Option<&'a String> {
    for w in want {
        if let Some(n) = names.iter().find(|n| n.eq_ignore_ascii_case(w)) {
            return Some(n);
        }
    }
    None
}

pub fn model_path(cfg: &VadCfg) -> PathBuf {
    let p = cfg.model.trim();
    if p.is_empty() {
        paths::cache_dir().join("silero").join("silero_vad.onnx")
    } else {
        PathBuf::from(p)
    }
}

fn ensure_model(path: &Path, url: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if url.trim().is_empty() {
        anyhow::bail!(
            "silero model missing at {} and [vad] url is empty",
            path.display()
        );
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("onnx.part");
    tracing::info!(path = %path.display(), url, "downloading silero vad");
    let status = Command::new("curl")
        .args(["-fsSL", "--connect-timeout", "10", "--max-time", "30", "-o"])
        .arg(&tmp)
        .arg(url)
        .status()
        .context("curl silero vad")?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("curl silero vad failed: {status}");
    }
    std::fs::rename(&tmp, path).context("install silero vad")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use assistant_core::config::VadCfg;

    #[test]
    fn silence_is_not_speech() {
        let cfg = VadCfg {
            model: model_path(&VadCfg::default()).display().to_string(),
            ..Default::default()
        };
        if !std::path::Path::new(&cfg.model).exists() {
            return;
        }
        let silero = Silero::load(&cfg).expect("load silero");
        let mut state = silero.new_state();
        let quiet = vec![0i16; 512];
        let p = silero.infer(&quiet, &mut state).expect("infer");
        assert!(p < 0.5, "silence prob {p}");
    }
}
