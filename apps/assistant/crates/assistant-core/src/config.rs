use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub assistant: AssistantCfg,
    pub stt: SttCfg,
    pub llm: LlmCfg,
    pub tts: TtsCfg,
    pub vad: VadCfg,
    pub audio: AudioCfg,
    pub wake_word: WakeWordCfg,
    pub ui: UiCfg,
    pub permissions: PermissionsCfg,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            assistant: AssistantCfg::default(),
            stt: SttCfg::default(),
            llm: LlmCfg::default(),
            tts: TtsCfg::default(),
            vad: VadCfg::default(),
            audio: AudioCfg::default(),
            wake_word: WakeWordCfg::default(),
            ui: UiCfg::default(),
            permissions: PermissionsCfg::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantCfg {
    pub name: String,
    pub mode: String,
}

impl Default for AssistantCfg {
    fn default() -> Self {
        Self {
            name: "Nyx".into(),
            mode: "push-to-talk".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SttCfg {
    pub provider: String,
    pub voxtype_bin: String,
}

impl Default for SttCfg {
    fn default() -> Self {
        Self {
            provider: "voxtype".into(),
            voxtype_bin: "voxtype".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmCfg {
    pub backend: String,
    /// Path to a GGUF file, or `auto` to pick a local instruct model.
    pub model: String,
    pub n_gpu_layers: i32,
    pub ctx: u32,
    pub n_predict: i32,
    pub temperature: f32,
}

impl Default for LlmCfg {
    fn default() -> Self {
        Self {
            backend: "llama.cpp".into(),
            model: "auto".into(),
            n_gpu_layers: -1,
            ctx: 4096,
            n_predict: 256,
            temperature: 0.7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsCfg {
    pub provider: String,
    pub voice: String,
    pub speed: f32,
    pub device: String,
}

impl Default for TtsCfg {
    fn default() -> Self {
        Self {
            provider: "kokoro".into(),
            voice: "af_heart".into(),
            speed: 1.0,
            device: "cuda".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VadCfg {
    pub enabled: bool,
    pub backend: String,
    pub threshold: f32,
    pub hangover_ms: u64,
}

impl Default for VadCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: "energy".into(),
            threshold: 0.012,
            hangover_ms: 400,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioCfg {
    pub capture_rate: u32,
    pub playback_rate: u32,
    pub source: String,
    pub sink: String,
}

impl Default for AudioCfg {
    fn default() -> Self {
        Self {
            capture_rate: 16_000,
            playback_rate: 24_000,
            source: "default".into(),
            sink: "default".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WakeWordCfg {
    pub enabled: bool,
}

impl Default for WakeWordCfg {
    fn default() -> Self {
        Self { enabled: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiCfg {
    pub enabled: bool,
}

impl Default for UiCfg {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionsCfg {
    pub shell: String,
    pub launch_app: String,
    pub volume: String,
    pub files_read: String,
    pub files_write: String,
    pub shutdown: String,
}

impl Default for PermissionsCfg {
    fn default() -> Self {
        Self {
            shell: "deny".into(),
            launch_app: "allow".into(),
            volume: "allow".into(),
            files_read: "allow".into(),
            files_write: "confirm".into(),
            shutdown: "confirm".into(),
        }
    }
}

impl Config {
    pub fn load() -> crate::Result<Self> {
        Self::load_from(&paths::config_path())
    }

    pub fn load_from(path: &Path) -> crate::Result<Self> {
        if !path.exists() {
            let cfg = Self::default();
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(path, default_toml())?;
            tracing::info!(path = %path.display(), "wrote default config");
            return Ok(cfg);
        }
        let text = fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&text)?;
        Ok(cfg)
    }

    pub fn resolve_model_path(&self) -> PathBuf {
        if self.llm.model != "auto" {
            return PathBuf::from(&self.llm.model);
        }
        const CANDIDATES: &[&str] = &[
            "/home/ken/.lmstudio/models/lmstudio-community/gemma-4-E4B-it-GGUF/gemma-4-E4B-it-Q4_K_M.gguf",
            "/home/ken/.lmstudio/models/lmstudio-community/Mistral-7B-Instruct-v0.3-GGUF/Mistral-7B-Instruct-v0.3-Q4_K_M.gguf",
            "/home/ken/.lmstudio/models/lmstudio-community/Qwen3.5-9B-GGUF/Qwen3.5-9B-Q4_K_M.gguf",
        ];
        for c in CANDIDATES {
            let p = PathBuf::from(c);
            if p.is_file() {
                return p;
            }
        }
        let dir = paths::models_dir();
        if let Ok(rd) = fs::read_dir(&dir) {
            let mut ggufs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("gguf"))
                .collect();
            ggufs.sort();
            if let Some(p) = ggufs.into_iter().next() {
                return p;
            }
        }
        dir.join("model.gguf")
    }

    pub fn system_prompt(&self) -> String {
        format!(
            "You are {}, a fast local voice assistant on Omarchy/Hyprland. \
             Answer questions out loud in short spoken sentences. \
             No markdown, no bullet lists, no code fences. \
             You cannot control the desktop, windows, brightness, volume, or workspaces. \
             If the user asked you to do something on the computer, say you did not catch a command you can run. \
             Never claim you did an action.",
            self.assistant.name
        )
    }
}

pub fn default_toml() -> &'static str {
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_config_parses() {
        let cfg: Config = toml::from_str(default_toml()).unwrap();
        assert_eq!(cfg.assistant.name, "Nyx");
        assert_eq!(cfg.assistant.mode, "push-to-talk");
        assert_eq!(cfg.stt.provider, "voxtype");
        assert_eq!(cfg.llm.backend, "llama.cpp");
        assert_eq!(cfg.tts.provider, "kokoro");
        assert_eq!(cfg.permissions.shell, "deny");
    }
}
