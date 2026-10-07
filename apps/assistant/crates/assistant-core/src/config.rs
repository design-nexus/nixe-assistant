use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub assistant: AssistantCfg,
    pub stt: SttCfg,
    pub llm: LlmCfg,
    pub tts: TtsCfg,
    pub vad: VadCfg,
    pub audio: AudioCfg,
    pub barge_in: BargeInCfg,
    pub wake_word: WakeWordCfg,
    pub clock: ClockCfg,
    pub ui: UiCfg,
    pub permissions: PermissionsCfg,
    pub agent: AgentCfg,
    pub mcp: McpCfg,
    pub http: HttpCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantCfg {
    pub name: String,
    pub mode: String,
    #[serde(default = "default_assistant_prompt")]
    pub prompt: String,
}

impl Default for AssistantCfg {
    fn default() -> Self {
        Self {
            name: "Nyx".into(),
            mode: "push-to-talk".into(),
            prompt: default_assistant_prompt(),
        }
    }
}

fn default_assistant_prompt() -> String {
    "You are {name}, a fast local voice assistant on Omarchy/Hyprland. \
     Answer questions out loud in short spoken sentences. \
     No markdown, no bullet lists, no code fences. \
     Never claim you changed the computer."
        .into()
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
    #[serde(default = "default_vad_confirm")]
    pub start_confirm_frames: u32,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_silero_url")]
    pub url: String,
}

fn default_vad_confirm() -> u32 {
    2
}

impl Default for VadCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: "silero".into(),
            threshold: 0.5,
            hangover_ms: 400,
            start_confirm_frames: 2,
            model: String::new(),
            url: default_silero_url(),
        }
    }
}

pub fn default_silero_url() -> String {
    "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx"
        .into()
}

impl VadCfg {
    pub fn wants_silero(&self) -> bool {
        self.enabled && self.backend.trim().eq_ignore_ascii_case("silero")
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
pub struct BargeInCfg {
    pub enabled: bool,
    pub grace_ms: u64,
    pub threshold: f32,
    pub hangover_ms: u64,
    pub aec: bool,
}

impl Default for BargeInCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            grace_ms: 500,
            threshold: 0.08,
            hangover_ms: 160,
            aec: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WakeWordCfg {
    pub enabled: bool,
    pub threshold: f32,
    pub hangover_ms: u64,
    pub start_confirm_frames: u32,
    pub preroll_ms: u64,
    pub min_ms: u64,
    pub max_ms: u64,
    pub hush_ms: u64,
    pub prefixes: Vec<String>,
    pub phrases: Vec<String>,
}

impl Default for WakeWordCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: 0.003,
            hangover_ms: 500,
            start_confirm_frames: 4,
            preroll_ms: 500,
            min_ms: 400,
            max_ms: 10_000,
            hush_ms: 1_200,
            prefixes: vec!["hey".into(), "ok".into(), "okay".into()],
            phrases: vec![
                "nyx".into(),
                "nix".into(),
                "nicks".into(),
                "nick".into(),
                "niks".into(),
                "nics".into(),
                "nyks".into(),
                "next".into(),
                "six".into(),
                "neks".into(),
                "некс".into(),
                "никс".into(),
            ],
        }
    }
}

impl WakeWordCfg {
    pub fn phrase_list(&self, name: &str) -> Vec<String> {
        let mut out = Vec::new();
        let n = name.trim().to_lowercase();
        if !n.is_empty() {
            out.push(n);
        }
        for p in &self.phrases {
            let p = p.trim().to_lowercase();
            if !p.is_empty() && !out.iter().any(|x| x == &p) {
                out.push(p);
            }
        }
        out
    }

    pub fn prefix_list(&self) -> Vec<String> {
        self.prefixes
            .iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClockCfg {
    pub enabled: bool,
    pub inject_prompt: bool,
    pub time_format: String,
    pub date_format: String,
    pub time_template: String,
    pub date_template: String,
    pub prompt: String,
    pub time_phrases: Vec<String>,
    pub date_phrases: Vec<String>,
    pub strip_tails: Vec<String>,
}

impl Default for ClockCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            inject_prompt: true,
            time_format: "%-I:%M %p".into(),
            date_format: "%A, %B %-d".into(),
            time_template: "It's {time}.".into(),
            date_template: "It's {date}.".into(),
            prompt: "The current local time is {time}. Today's date is {date}. Use these when asked the time or date.".into(),
            time_phrases: vec![
                "what time is it".into(),
                "whats the time".into(),
                "what is the time".into(),
                "tell me the time".into(),
                "tell me what time it is".into(),
                "current time".into(),
                "whats the current time".into(),
                "what is the current time".into(),
                "what time".into(),
                "got the time".into(),
                "do you have the time".into(),
                "time is it".into(),
            ],
            date_phrases: vec![
                "whats the date".into(),
                "what is the date".into(),
                "what day is it".into(),
                "whats today".into(),
                "what is today".into(),
                "whats todays date".into(),
                "what is todays date".into(),
                "tell me the date".into(),
                "what date is it".into(),
            ],
            strip_tails: vec![
                "please".into(),
                "now".into(),
                "right now".into(),
                "for me".into(),
            ],
        }
    }
}

impl ClockCfg {
    pub fn format_time(&self) -> String {
        chrono::Local::now()
            .format(&self.time_format)
            .to_string()
    }

    pub fn format_date(&self) -> String {
        chrono::Local::now()
            .format(&self.date_format)
            .to_string()
    }

    pub fn spoken_reply(&self, text: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let key = self.key(text);
        if key.is_empty() {
            return None;
        }
        if self.time_phrases.iter().any(|p| self.key(p) == key) {
            return Some(self.time_template.replace("{time}", &self.format_time()));
        }
        if self.date_phrases.iter().any(|p| self.key(p) == key) {
            return Some(self.date_template.replace("{date}", &self.format_date()));
        }
        None
    }

    fn key(&self, text: &str) -> String {
        let mut s = normalize_ask(text);
        loop {
            let mut stripped = false;
            for tail in &self.strip_tails {
                let t = normalize_ask(tail);
                if t.is_empty() {
                    continue;
                }
                if let Some(rest) = s.strip_suffix(&format!(" {t}")) {
                    s = rest.trim().to_string();
                    stripped = true;
                } else if s == t {
                    s.clear();
                    stripped = true;
                }
            }
            if !stripped {
                break;
            }
        }
        s
    }
}

fn normalize_ask(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c == '\'' || c == '’' {
            continue;
        }
        if c.is_alphanumeric() {
            for x in c.to_lowercase() {
                out.push(x);
            }
        } else if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_string()
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentCfg {
    pub enabled: bool,
    pub default_agent: String,
    pub terminal: String,
}

impl Default for AgentCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            default_agent: "agy".into(),
            terminal: "omarchy-launch-terminal".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct McpCfg {
    pub enabled: bool,
    pub servers: std::collections::HashMap<String, McpServerCfg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerCfg {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpCfg {
    pub enabled: bool,
    pub bind: String,
    pub port: u16,
}

impl Default for HttpCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: "127.0.0.1".into(),
            port: 8765,
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
        let mut s = self.assistant.prompt.replace("{name}", &self.assistant.name);
        if s.trim().is_empty() {
            s = default_assistant_prompt().replace("{name}", &self.assistant.name);
        }
        if self.clock.enabled && self.clock.inject_prompt && !self.clock.prompt.is_empty() {
            let line = self
                .clock
                .prompt
                .replace("{time}", &self.clock.format_time())
                .replace("{date}", &self.clock.format_date());
            s.push(' ');
            s.push_str(&line);
        }
        s
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
        assert!(cfg.assistant.prompt.contains("short spoken sentences"));
        assert_eq!(cfg.stt.provider, "voxtype");
        assert_eq!(cfg.llm.backend, "llama.cpp");
        assert_eq!(cfg.tts.provider, "kokoro");
        assert_eq!(cfg.permissions.shell, "deny");
        assert!(cfg.barge_in.enabled);
        assert_eq!(cfg.barge_in.grace_ms, 500);
        assert!(cfg.wake_word.enabled);
        assert!(cfg.wake_word.phrases.iter().any(|p| p == "nyx"));
        assert!(cfg.clock.enabled);
        assert!(cfg.clock.inject_prompt);
        assert!(cfg.clock.time_phrases.iter().any(|p| p == "what time is it"));
        assert_eq!(cfg.vad.backend, "silero");
        assert!((cfg.vad.threshold - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn clock_speaks_local_time() {
        let clock = ClockCfg::default();
        let spoken = clock.spoken_reply("what time is it?").expect("time phrase");
        assert!(spoken.starts_with("It's "), "{spoken}");
        let spoken = clock
            .spoken_reply("what's the time please")
            .expect("apostrophe and tail");
        assert!(spoken.starts_with("It's "), "{spoken}");
        let spoken = clock.spoken_reply("what day is it").expect("date phrase");
        assert!(spoken.starts_with("It's "), "{spoken}");
        assert!(clock.spoken_reply("open terminal").is_none());
        assert!(clock.spoken_reply("what time does the store close").is_none());
    }

    #[test]
    fn system_prompt_includes_clock() {
        let cfg = Config::default();
        let prompt = cfg.system_prompt();
        assert!(prompt.contains("current local time"), "{prompt}");
        assert!(prompt.contains(&cfg.clock.format_time()), "{prompt}");
        assert!(
            !prompt.to_lowercase().contains("catch a command"),
            "{prompt}"
        );
    }
}
