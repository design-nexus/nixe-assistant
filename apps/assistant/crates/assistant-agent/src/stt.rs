use anyhow::{Context, Result};
use assistant_core::config::Config;
use assistant_core::paths;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
    pub is_partial: bool,
}

pub struct VoxtypeProvider {
    bin: String,
    config_path: PathBuf,
    runtime_dir: PathBuf,
    stt_file: PathBuf,
}

impl VoxtypeProvider {
    pub fn new(cfg: &Config) -> Self {
        Self {
            bin: cfg.stt.voxtype_bin.clone(),
            config_path: paths::config_dir().join("voxtype-parakeet.toml"),
            runtime_dir: paths::assistant_voxtype_runtime(),
            stt_file: paths::runtime_dir().join("stt.txt"),
        }
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(&self.bin);
        c.arg("-c")
            .arg(&self.config_path)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("RUST_LOG", "error")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        c
    }

    async fn record(&self, args: &[&str]) -> Result<()> {
        let mut command = self.cmd();
        command.arg("record").args(args);
        let output = tokio::time::timeout(Duration::from_secs(5), command.output())
            .await
            .context("voxtype record timed out")?
            .with_context(|| format!("spawn {} record", self.bin))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("voxtype record {} failed: {err}", args.join(" "));
        }
        Ok(())
    }

    /// Start the warm Parakeet daemon recording for this PTT take.
    pub async fn begin(&self) -> Result<()> {
        tokio::fs::create_dir_all(paths::runtime_dir()).await?;
        let _ = tokio::fs::remove_file(&self.stt_file).await;
        let file = self.stt_file.to_string_lossy().into_owned();
        self.record(&["start", "--no-osd", "--file", &file]).await
    }

    pub async fn cancel_listen(&self) {
        let _ = self.record(&["cancel"]).await;
    }

    pub async fn finish(
        &self,
        pcm_16k_mono: &[i16],
        cancel: CancellationToken,
    ) -> Result<Transcript> {
        match self.finish_daemon(&cancel).await {
            Ok(tr) if !tr.text.trim().is_empty() || pcm_16k_mono.len() < 3_200 => Ok(tr),
            Ok(_) | Err(_) => self.transcribe(pcm_16k_mono, cancel).await,
        }
    }

    async fn finish_daemon(&self, cancel: &CancellationToken) -> Result<Transcript> {
        self.record(&["stop"]).await?;
        let state_path = self.runtime_dir.join("voxtype").join("state");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if cancel.is_cancelled() {
                anyhow::bail!("stt cancelled");
            }
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!("parakeet daemon timed out");
            }
            let state = tokio::fs::read_to_string(&state_path)
                .await
                .unwrap_or_default();
            if state.trim() == "idle" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let text = tokio::fs::read_to_string(&self.stt_file)
            .await
            .unwrap_or_default()
            .trim()
            .to_string();
        let _ = tokio::fs::remove_file(&self.stt_file).await;
        Ok(Transcript {
            text,
            is_partial: false,
        })
    }

    pub async fn transcribe(
        &self,
        pcm_16k_mono: &[i16],
        cancel: CancellationToken,
    ) -> Result<Transcript> {
        let dir = paths::runtime_dir();
        tokio::fs::create_dir_all(&dir).await?;
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let wav_path: PathBuf = dir.join(format!("utt-{}-{}.wav", std::process::id(), seq));
        write_wav(&wav_path, pcm_16k_mono)?;

        let child = Command::new(&self.bin)
            .arg("-c")
            .arg(&self.config_path)
            .arg("transcribe")
            .arg("--engine")
            .arg("parakeet")
            .arg(&wav_path)
            .env("RUST_LOG", "error")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawn {}", self.bin))?;

        let output = tokio::select! {
            _ = cancel.cancelled() => {
                anyhow::bail!("stt cancelled");
            }
            out = child.wait_with_output() => out.context("voxtype transcribe")?,
        };
        let _ = tokio::fs::remove_file(&wav_path).await;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("voxtype transcribe failed: {err}");
        }
        let text = parse_voxtype_stdout(&String::from_utf8_lossy(&output.stdout));
        Ok(Transcript {
            text,
            is_partial: false,
        })
    }
}

fn write_wav(path: &PathBuf, pcm: &[i16]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)
        .with_context(|| format!("create {}", path.display()))?;
    for s in pcm {
        w.write_sample(*s)?;
    }
    w.flush()?;
    Ok(())
}

fn parse_voxtype_stdout(stdout: &str) -> String {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("Loading ")
                && !line.starts_with("Audio format:")
                && !line.starts_with("Processing ")
                && !line.contains(" INFO ")
                && !line.contains(" WARN ")
                && !line.contains(" DEBUG ")
        })
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_last_transcript_line() {
        let raw = "\
Loading audio file: \"/tmp/x.wav\"
Audio format: 16000 Hz, 1 channel(s), Int
Processing 17274 samples (1.08s)...
2026-10-06T02:52:10Z  INFO Transcription completed in 0.75s: \"What is 2 plus 2?\"

What is 2 plus 2?
";
        assert_eq!(parse_voxtype_stdout(raw), "What is 2 plus 2?");
    }
}
