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
    pub vision: VisionCfg,
    pub computer: ComputerCfg,
    pub tts: TtsCfg,
    pub vad: VadCfg,
    pub audio: AudioCfg,
    pub barge_in: BargeInCfg,
    pub duplex: DuplexCfg,
    pub wake_word: WakeWordCfg,
    pub clock: ClockCfg,
    pub ui: UiCfg,
    pub permissions: PermissionsCfg,
    pub agent: AgentCfg,
    pub mcp: McpCfg,
    pub http: HttpCfg,
    pub conversation: ConversationCfg,
    pub dictation: DictationCfg,
    pub browser: BrowserCfg,
    pub location: crate::location::LocationCfg,
    pub terminal: TerminalCfg,
    pub system: SystemCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SystemCfg {
    /// systemd user units the assistant may start, stop and restart without asking.
    pub user_units: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConversationCfg {
    pub enabled: bool,
    pub follow_up_secs: u64,
    pub idle_secs: u64,
}
impl Default for ConversationCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            follow_up_secs: 10,
            idle_secs: 60,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DictationCfg {
    pub enabled: bool,
    pub idle_secs: u64,
}
impl Default for DictationCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_secs: 120,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserCfg {
    pub enabled: bool,
    pub node: String,
    pub worker: String,
    pub weather_location: String,
    pub search_tool: String,
}
impl Default for BrowserCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            node: "nixe-assistant-node".into(),
            worker: "auto".into(),
            weather_location: String::new(),
            search_tool: String::new(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalCfg {
    pub timeout_secs: u64,
    pub commands: std::collections::BTreeMap<String, Vec<String>>,
    /// Network inside the sandbox for generated commands (package downloads, git fetch).
    pub sandbox_network: bool,
    /// Folders under HOME the sandbox may read (toolchains); missing ones are skipped.
    pub sandbox_read_only: Vec<String>,
}
impl Default for TerminalCfg {
    fn default() -> Self {
        Self {
            timeout_secs: 120,
            commands: Default::default(),
            sandbox_network: true,
            sandbox_read_only: ["~/.cargo", "~/.rustup", "~/.local/bin", "~/go", "~/.npm"]
                .map(String::from)
                .into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantCfg {
    pub name: String,
    pub persona_id: String,
    pub mode: String,
    #[serde(default = "default_assistant_prompt")]
    pub prompt: String,
}

impl Default for AssistantCfg {
    fn default() -> Self {
        Self {
            name: "Echo".into(),
            persona_id: String::new(),
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
    /// "streaming" (the Nemotron worker, which hears the echo-cancelled stream and gives
    /// partial words) or "voxtype" (warm Parakeet, whole utterances). Streaming falls back to
    /// Voxtype when its model or worker is missing.
    pub provider: String,
    pub voxtype_bin: String,
    /// The streaming model's directory, or "auto" for the one `models fetch asr` installs.
    pub model: String,
    pub threads: u32,
    /// "cpu" or "cuda". The int8 model is no faster on the GPU here (docs/BENCHMARKS.md).
    pub device: String,
}

impl SttCfg {
    pub fn streaming(&self) -> bool {
        self.provider == "streaming"
    }
}

impl Default for SttCfg {
    fn default() -> Self {
        Self {
            provider: "streaming".into(),
            voxtype_bin: "voxtype".into(),
            model: "auto".into(),
            threads: 2,
            device: "cpu".into(),
        }
    }
}

/// Computer use: operating app windows through their accessibility tree, the pointer and
/// the keyboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ComputerCfg {
    /// Window classes never touched, on top of the built-in list (password managers,
    /// terminals, authentication prompts, lock screens, the assistant's own windows).
    pub deny_classes: Vec<String>,
    /// A click on something labelled with one of these always asks, even inside an approved task.
    pub confirm_words: Vec<String>,
}

impl Default for ComputerCfg {
    fn default() -> Self {
        Self {
            deny_classes: Vec::new(),
            confirm_words: [
                "send", "submit", "buy", "pay", "purchase", "order", "checkout", "delete", "remove", "erase", "post",
                "publish", "confirm", "transfer", "unsubscribe", "uninstall", "format", "wipe",
            ]
            .iter()
            .map(|w| w.to_string())
            .collect(),
        }
    }
}

/// Looking at the screen with the LLM's image projector, loaded only while in use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VisionCfg {
    /// The projector (mmproj) GGUF, or "auto" for one next to the model file.
    pub mmproj: String,
    /// Unloaded after this long without a look, to give the VRAM back.
    pub idle_secs: u64,
    /// Screenshots are scaled down to this width before the model sees them.
    pub max_width: u32,
}

impl Default for VisionCfg {
    fn default() -> Self {
        Self { mmproj: "auto".into(), idle_secs: 120, max_width: 1280 }
    }
}

impl VisionCfg {
    /// The projector to use with `model`, if there is one.
    pub fn resolve(&self, model: &std::path::Path) -> Option<PathBuf> {
        if self.mmproj != "auto" && !self.mmproj.is_empty() {
            let p = PathBuf::from(&self.mmproj);
            return p.is_file().then_some(p);
        }
        let found: Vec<PathBuf> = std::fs::read_dir(model.parent()?)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("mmproj") && n.ends_with(".gguf")))
            .collect();
        // Several models may share a folder: the projector named most like the model.
        let stem = model.file_name()?.to_str()?.to_lowercase();
        let common = |p: &PathBuf| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
            let name = name.trim_start_matches("mmproj-").to_string();
            stem.chars().zip(name.chars()).take_while(|(a, b)| a == b).count()
        };
        match found.len() {
            0 => None,
            1 => found.into_iter().next(),
            _ => found.into_iter().filter(|p| common(p) >= 6).max_by_key(common),
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
    pub flash_attn: bool,
    pub prefix_cache: bool,
    pub n_batch: u32,
    pub n_ubatch: u32,
}

impl Default for LlmCfg {
    fn default() -> Self {
        Self {
            backend: "llama.cpp".into(),
            model: "auto".into(),
            n_gpu_layers: -1,
            ctx: 16384,
            n_predict: 768,
            temperature: 0.7,
            flash_attn: true,
            prefix_cache: true,
            n_batch: 2048,
            n_ubatch: 512,
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
    pub variant: String,
    pub threads: Option<usize>,
    pub force_cuda: bool,
}

impl Default for TtsCfg {
    fn default() -> Self {
        Self {
            provider: "kokoro".into(),
            voice: "af_heart".into(),
            speed: 1.0,
            device: "cuda".into(),
            variant: "fp32".into(),
            threads: None,
            force_cuda: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VadCfg {
    pub enabled: bool,
    pub backend: String,
    pub threshold: f32,
    pub energy_threshold: f32,
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
            energy_threshold: 0.012,
            hangover_ms: 400,
            start_confirm_frames: 2,
            model: String::new(),
            url: default_silero_url(),
        }
    }
}

pub fn default_silero_url() -> String {
    "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx".into()
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

/// Listening while speaking: what an interruption does (needs `[stt] provider = "streaming"`
/// and `[barge_in] aec`; otherwise any speech over a reply stops it, as before).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DuplexCfg {
    pub enabled: bool,
    /// What the voice does the moment someone speaks: "pause" (stop mid-word, nothing lost,
    /// so the words are heard without echo) or "duck" (carry on quieter, at `duck_gain`).
    /// Short words said over the voice rarely survive echo cancelling, so "pause" is default.
    pub on_speech: String,
    /// Playback volume while ducked (0–1).
    pub duck_gain: f32,
    /// Said alone, these end the reply and wait.
    pub stop_phrases: Vec<String>,
    /// Said alone, these let the reply carry on.
    pub backchannels: Vec<String>,
    /// Speech with no words after this long is noise; the voice comes back up.
    pub noise_ms: u64,
    /// The quietest echo-cancelled speech (RMS) that counts as someone talking.
    pub min_rms: f32,
}

impl Default for DuplexCfg {
    fn default() -> Self {
        let list = |s: &[&str]| s.iter().map(|w| w.to_string()).collect();
        Self {
            enabled: true,
            on_speech: "pause".into(),
            duck_gain: 0.25,
            stop_phrases: list(&[
                "stop", "wait", "hold on", "hang on", "never mind", "nevermind", "cancel", "enough", "that's enough",
                "be quiet", "quiet", "shut up", "stop talking", "pause",
            ]),
            backchannels: list(&[
                "mm hm", "mhm", "mm", "hmm", "uh huh", "yeah", "yep", "yes", "okay", "ok", "right", "sure", "i see",
                "got it", "alright", "all right", "cool", "nice", "great",
            ]),
            noise_ms: 1500,
            min_rms: 0.003,
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
            start_confirm_frames: 2,
            preroll_ms: 500,
            min_ms: 400,
            max_ms: 10_000,
            hush_ms: 1_200,
            prefixes: vec![
                "hey".into(),
                "ok".into(),
                "okay".into(),
                "hi".into(),
                "yo".into(),
                "hello".into(),
                "a".into(),
                "ay".into(),
                "aye".into(),
            ],
            phrases: vec![
                "echo".into(),
                "eco".into(),
                "ecko".into(),
                "ekko".into(),
                "eko".into(),
                "gecko".into(),
                "hecko".into(),
                "eggo".into(),
                "hecho".into(),
                "heco".into(),
                "acco".into(),
                "acko".into(),
                "a echo".into(),
                "ay echo".into(),
                "yo echo".into(),
                "hey echo".into(),
                "ok echo".into(),
                "okay echo".into(),
                "deco".into(),
                "deko".into(),
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
        chrono::Local::now().format(&self.time_format).to_string()
    }

    pub fn format_date(&self) -> String {
        chrono::Local::now().format(&self.date_format).to_string()
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
    pub admin: String,
}

impl PermissionsCfg {
    /// The capability rules for the policy. Values were checked by `Config::validate`;
    /// anything else asks.
    pub fn rules(&self) -> assistant_policy::Rules {
        let d = |s: &str| assistant_policy::Decision::parse(s).unwrap_or(assistant_policy::Decision::Ask);
        assistant_policy::Rules {
            launch_app: d(&self.launch_app),
            volume: d(&self.volume),
            files_read: d(&self.files_read),
            files_write: d(&self.files_write),
            shutdown: d(&self.shutdown),
            admin: d(&self.admin),
        }
    }
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
            admin: "confirm".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentCfg {
    pub enabled: bool,
    pub default_agent: String,
    pub terminal: String,
    /// How much runs without asking: autonomous, assisted or manual.
    pub mode: assistant_policy::AutonomyMode,
    /// Tool calls one task may make (1–32).
    pub max_calls: usize,
    /// Active seconds one task may spend, not counting approval waits (10–1800).
    pub max_active_secs: u64,
}

impl AgentCfg {
    pub fn budget(&self) -> assistant_policy::Budget {
        assistant_policy::Budget {
            max_calls: self.max_calls,
            max_active_ms: self.max_active_secs * 1000,
        }
    }
}

impl Default for AgentCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            default_agent: "agy".into(),
            terminal: "omarchy-launch-terminal".into(),
            mode: Default::default(),
            max_calls: 4,
            max_active_secs: 180,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct McpCfg {
    pub enabled: bool,
    pub servers: std::collections::HashMap<String, McpServerCfg>,
    pub permissions: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerCfg {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// What the server may reach. Without one it gets the network and nothing of HOME but
    /// the folders its command and script live in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxProfile>,
}

/// What a plugin may reach (a manifest's `[sandbox]`). The default is the strictest: no
/// network, nothing of HOME but its own package (read-only) and data folder, no desktop
/// sockets. Enforced with bubblewrap, a seccomp filter and a systemd scope's limits.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxProfile {
    #[serde(skip_serializing_if = "is_false")]
    pub network: bool,
    #[serde(skip_serializing_if = "HomeAccess::is_none")]
    pub home: HomeAccess,
    /// Folders or files it may read, beyond `home` ("~/..." or absolute).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub read: Vec<String>,
    /// Folders it may write, beyond `home`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub write: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sockets: Vec<SandboxSocket>,
    /// Session-bus names it may call (with `sockets = ["session-bus"]`), through a filtering
    /// proxy; "org.mpris.MediaPlayer2.*" covers a prefix.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub talk: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<SandboxDevice>,
    /// Sees the computer's processes (`/proc` of the host), not only its own.
    #[serde(skip_serializing_if = "is_false")]
    pub host_processes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<u64>,
    /// Runs without bubblewrap or seccomp (still under the limits), for the reason given.
    /// Honoured for first-party plugins only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unconfined: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HomeAccess {
    #[default]
    None,
    Read,
    Write,
}

impl HomeAccess {
    fn is_none(&self) -> bool {
        *self == HomeAccess::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxSocket {
    Wayland,
    Pipewire,
    /// Hyprland's control sockets. `hyprctl dispatch exec` runs programs outside the sandbox.
    Hyprland,
    SessionBus,
    SystemBus,
    /// The user service manager's own socket (what `systemctl --user` uses); it needs
    /// `host_processes`. Can start and stop the user's services.
    Systemd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxDevice {
    /// NVIDIA and DRI device nodes.
    Gpu,
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

/// A config key split into its path: `duplex.on_speech` is two steps; in
/// `mcp.permissions.files.write` everything after `mcp.permissions` is one key (tool names
/// contain dots).
fn key_path(key: &str) -> Vec<String> {
    let key = key.trim();
    if let Some(tool) = key.strip_prefix("mcp.permissions.") {
        return vec!["mcp".into(), "permissions".into(), tool.into()];
    }
    key.split('.').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// A value as typed on a command line: TOML (`true`, `0.25`, `["a", "b"]`, `"x"`), or else
/// the bare text as a string (`pause`).
pub fn parse_value(text: &str) -> serde_json::Value {
    match toml::from_str::<toml::Table>(&format!("v = {text}")) {
        Ok(mut t) => t.remove("v").and_then(|v| serde_json::to_value(v).ok()).unwrap_or(serde_json::Value::Null),
        Err(_) => serde_json::Value::String(text.to_string()),
    }
}

/// Floats in their single-precision shortest form (0.4, not 0.4000000059604645).
fn tidy_json(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Number(n) if n.is_f64() => {
            if let Some(short) = n.as_f64().and_then(|f| (f as f32).to_string().parse::<f64>().ok()) {
                if let Some(num) = serde_json::Number::from_f64(short) {
                    *n = num;
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(tidy_json),
        serde_json::Value::Object(map) => map.values_mut().for_each(tidy_json),
        _ => {}
    }
}

impl Config {
    /// The whole config as JSON, floats tidied.
    pub fn to_json(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        tidy_json(&mut v);
        v
    }

    /// One setting as JSON, by dotted key; `None` if there is no such key.
    pub fn get_key(&self, key: &str) -> Option<serde_json::Value> {
        let mut v = self.to_json();
        for step in key_path(key) {
            v = v.get(&step)?.clone();
        }
        Some(v)
    }

    /// A copy with one setting changed, checked: an unknown key, a wrong type or an invalid
    /// value is refused with the reason.
    pub fn with_key(&self, key: &str, value: serde_json::Value) -> crate::Result<Self> {
        let path = key_path(key);
        let (last, parents) = path.split_last().ok_or_else(|| crate::Error::msg("no key given"))?;
        let mut root = serde_json::to_value(self).map_err(|e| crate::Error::msg(e.to_string()))?;
        let mut node = &mut root;
        for step in parents {
            node = node.get_mut(step).ok_or_else(|| crate::Error::msg(format!("no setting “{key}”")))?;
        }
        let map = node.as_object_mut().ok_or_else(|| crate::Error::msg(format!("no setting “{key}”")))?;
        // A new key only where the section is a free-form map (per-tool rules).
        let free_form = path.len() == 3 && path[0] == "mcp" && path[1] == "permissions";
        if !free_form && !map.contains_key(last) {
            return Err(crate::Error::msg(format!("no setting “{key}”")));
        }
        map.insert(last.clone(), value);
        let next: Config = serde_json::from_value(root).map_err(|e| crate::Error::msg(format!("{key}: {e}")))?;
        next.validate()?;
        Ok(next)
    }

    pub fn load() -> crate::Result<Self> {
        Self::load_from(&paths::config_path())
    }

    pub fn load_from(path: &Path) -> crate::Result<Self> {
        if !path.exists() {
            let cfg: Self = toml::from_str(default_toml())?;
            cfg.validate()?;
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            atomic_write(path, default_toml().as_bytes())?;
            tracing::info!(path = %path.display(), "wrote default config");
            return Ok(cfg);
        }
        let text = fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn save(&self) -> crate::Result<()> {
        self.save_to(&paths::config_path())
    }

    pub fn save_to(&self, path: &Path) -> crate::Result<()> {
        self.validate()?;
        // Parse the existing file first: never replace malformed user configuration.
        let mut existing = if path.exists() {
            let text = fs::read_to_string(path)?;
            let _: Config = toml::from_str(&text)?;
            text.parse::<toml_edit::DocumentMut>()
                .map_err(|e| crate::Error::msg(e.to_string()))?
        } else {
            toml_edit::DocumentMut::new()
        };
        let mut updated = toml::to_string_pretty(self)?
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| crate::Error::msg(e.to_string()))?;
        tidy_floats(updated.as_table_mut());
        merge_table(existing.as_table_mut(), updated.as_table());
        if let Some(table) = existing
            .get_mut("location")
            .and_then(toml_edit::Item::as_table_mut)
        {
            if self.location.latitude.is_none() {
                table.remove("latitude");
            }
            if self.location.longitude.is_none() {
                table.remove("longitude");
            }
        }
        atomic_write(path, existing.to_string().as_bytes())
    }

    pub fn validate(&self) -> crate::Result<()> {
        let fail = |s: &str| Err(crate::Error::msg(s));
        if !(0.0..=1.0).contains(&self.duplex.duck_gain) {
            return fail("duplex.duck_gain must be between 0 and 1");
        }
        if self.duplex.noise_ms > 10_000 {
            return fail("duplex.noise_ms must be at most 10000");
        }
        if !(0.0..=0.1).contains(&self.duplex.min_rms) {
            return fail("duplex.min_rms must be between 0 and 0.1");
        }
        if !["pause", "duck"].contains(&self.duplex.on_speech.as_str()) {
            return fail("duplex.on_speech must be pause or duck");
        }
        self.location
            .validate()
            .map_err(|e| crate::Error::msg(e.to_string()))?;
        if self.conversation.follow_up_secs > 60
            || !(10..=600).contains(&self.conversation.idle_secs)
            || !(10..=600).contains(&self.dictation.idle_secs)
            || !(1..=600).contains(&self.terminal.timeout_secs)
        {
            return fail("invalid session or terminal timeout");
        }
        if self.browser.node.trim().is_empty()
            || self.browser.worker.trim().is_empty()
            || self
                .terminal
                .commands
                .values()
                .any(|args| args.is_empty() || args.iter().any(|a| a.contains('\0')))
        {
            return fail("invalid browser or configured command");
        }
        if self.assistant.name.trim().is_empty() {
            return fail("assistant name is empty");
        }
        if self.assistant.mode != "push-to-talk" {
            return fail("only push-to-talk mode is supported");
        }
        if self.llm.ctx < 256
            || self.llm.ctx > 1_048_576
            || self.llm.n_predict < 1
            || self.llm.n_predict as u32 >= self.llm.ctx
            || self.llm.n_batch == 0
            || self.llm.n_ubatch == 0
            || self.llm.n_ubatch > self.llm.n_batch
        {
            return fail("invalid LLM context, response allowance, or batch size");
        }
        for (name, value, min, max) in [
            ("temperature", self.llm.temperature, 0.0, 2.0),
            ("speed", self.tts.speed, 0.5, 2.0),
            ("Silero probability", self.vad.threshold, 0.0, 1.0),
            ("VAD RMS", self.vad.energy_threshold, 0.00001, 1.0),
            ("wake RMS", self.wake_word.threshold, 0.00001, 1.0),
            ("barge RMS", self.barge_in.threshold, 0.00001, 1.0),
        ] {
            if !value.is_finite() || value < min || value > max {
                return fail(&format!("invalid {name}"));
            }
        }
        if !["voxtype", "streaming"].contains(&self.stt.provider.as_str())
            || !["cpu", "cuda"].contains(&self.stt.device.as_str())
            || !(1..=16).contains(&self.stt.threads)
            || self.llm.backend != "llama.cpp"
            || self.tts.provider != "kokoro"
        {
            return fail("unsupported provider");
        }
        if !["fp32", "fp16", "quantized", "int8", "q8"].contains(&self.tts.variant.as_str())
            || !["cpu", "cuda", "cuda:0"].contains(&self.tts.device.as_str())
            || self.tts.voice.trim().is_empty()
            || self.tts.threads == Some(0)
        {
            return fail("invalid TTS settings");
        }
        if chrono::format::StrftimeItems::new(&self.clock.time_format)
            .chain(chrono::format::StrftimeItems::new(&self.clock.date_format))
            .any(|item| matches!(item, chrono::format::Item::Error))
        {
            return fail("invalid clock format");
        }
        if !["silero", "energy"].contains(&self.vad.backend.as_str()) {
            return fail("invalid VAD backend");
        }
        if self.wake_word.min_ms > self.wake_word.max_ms
            || self.wake_word.max_ms > 60_000
            || self.vad.start_confirm_frames == 0
            || self.wake_word.start_confirm_frames == 0
        {
            return fail("invalid VAD/wake timing");
        }
        if self.audio.capture_rate != 16_000 || self.audio.playback_rate != 24_000 {
            return fail("audio requires 16000 capture / 24000 playback");
        }
        if self
            .http
            .bind
            .parse::<std::net::IpAddr>()
            .map(|a| !a.is_loopback())
            .unwrap_or(true)
            || self.http.port == 0
        {
            return fail("HTTP must bind a loopback IP and nonzero port");
        }
        let p = &self.permissions;
        for policy in [
            &p.shell,
            &p.launch_app,
            &p.volume,
            &p.files_read,
            &p.files_write,
            &p.shutdown,
            &p.admin,
        ]
        .into_iter()
        .chain(self.mcp.permissions.values())
        {
            if assistant_policy::Decision::parse(policy).is_none() {
                return fail("permission must be allow, confirm (or ask), or deny");
            }
        }
        if !(1..=32).contains(&self.agent.max_calls)
            || !(10..=1800).contains(&self.agent.max_active_secs)
        {
            return fail("agent max_calls must be 1–32 and max_active_secs 10–1800");
        }
        for (name, server) in &self.mcp.servers {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
                || server.command.trim().is_empty()
            {
                return fail("invalid MCP server name or command");
            }
        }
        Ok(())
    }

    /// Keep fields owned by resident resources truthful until a service restart.
    pub fn apply_live(&self, running: &Config) -> (Config, Vec<String>) {
        let mut next = self.clone();
        let mut restart = Vec::new();
        macro_rules! deferred {
            ($section:ident, $field:ident) => {
                if serde_json::to_value(&self.$section.$field).ok()
                    != serde_json::to_value(&running.$section.$field).ok()
                {
                    restart.push(concat!(stringify!($section), ".", stringify!($field)).into());
                    next.$section.$field = running.$section.$field.clone();
                }
            };
        }
        deferred!(llm, model);
        deferred!(llm, backend);
        deferred!(llm, ctx);
        deferred!(llm, n_gpu_layers);
        deferred!(llm, flash_attn);
        deferred!(llm, prefix_cache);
        deferred!(llm, n_batch);
        deferred!(llm, n_ubatch);
        deferred!(tts, provider);
        deferred!(tts, device);
        deferred!(tts, variant);
        deferred!(tts, threads);
        deferred!(tts, force_cuda);
        deferred!(stt, provider);
        deferred!(stt, voxtype_bin);
        deferred!(stt, model);
        deferred!(stt, threads);
        deferred!(stt, device);
        deferred!(vision, mmproj);
        deferred!(audio, source);
        deferred!(audio, sink);
        deferred!(audio, capture_rate);
        deferred!(audio, playback_rate);
        deferred!(barge_in, aec);
        deferred!(http, enabled);
        deferred!(http, bind);
        deferred!(http, port);
        deferred!(mcp, enabled);
        deferred!(mcp, servers);
        deferred!(browser, enabled);
        deferred!(browser, node);
        deferred!(browser, worker);
        deferred!(browser, search_tool);
        (next, restart)
    }

    /// Activates a persona profile: updates name, prompt, TTS voice, wake prefixes, and aliases.
    pub fn activate_persona(&mut self, persona: &crate::Persona) {
        self.assistant.persona_id = persona.id.clone();
        self.assistant.name = persona.name.clone();
        if !persona.prompt.is_empty() {
            self.assistant.prompt = persona.prompt.clone();
        }
        if !persona.voice.is_empty() {
            self.tts.voice = persona.voice.clone();
        }
        if !persona.prefixes.is_empty() {
            self.wake_word.prefixes = persona.prefixes.clone();
        }
        self.wake_word.phrases = persona.aliases.clone();
    }

    pub fn scan_local_models() -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        let mut roots = Vec::new();
        roots.push(paths::models_dir());
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".lmstudio/models"));
            roots.push(home.join(".cache/huggingface/hub"));
            roots.push(home.join(".local/share/models"));
        }
        for root in roots {
            if root.is_dir() {
                scan_dir(&root, 0, 4, &mut candidates);
            }
        }
        candidates.retain(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    !name.to_lowercase().contains("mmproj") && !name.contains("-of-")
                })
        });
        candidates.sort();
        candidates.dedup();
        candidates
    }

    pub fn resolve_model_path(&self) -> PathBuf {
        if self.llm.model != "auto" {
            return PathBuf::from(&self.llm.model);
        }
        Self::auto_model(&Self::scan_local_models())
    }

    pub fn auto_model(models: &[PathBuf]) -> PathBuf {
        const PREFERRED: &[&str] = &[
            "gemma-4-E4B-it-Q4_K_M.gguf",
            "qwen2.5-3b-instruct-q4_k_m.gguf",
            "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
            "smollm2-1.7b-instruct-q4_k_m.gguf",
        ];
        for filename in PREFERRED {
            if let Some(path) = models.iter().find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(filename))
            }) {
                return path.clone();
            }
        }
        models
            .iter()
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.to_lowercase().contains("instruct") || n.contains("-it-"))
            })
            .or_else(|| models.first())
            .cloned()
            .unwrap_or_else(|| paths::models_dir().join("model.gguf"))
    }

    pub fn system_prompt(&self) -> String {
        let mut s = self
            .assistant
            .prompt
            .replace("{name}", &self.assistant.name);
        if s.trim().is_empty() {
            s = default_assistant_prompt().replace("{name}", &self.assistant.name);
        }
        // Personas ask for short answers; small models otherwise refuse a story as "too long".
        s.push_str(" Short answers are the default, not a limit: when asked for something longer, such as a story, an explanation or a list, give it in full and never refuse because of length.");
        s.push_str(" For a user in the United States, always use imperial units: Fahrenheit, miles, miles per hour, feet, and pounds. Do not default to metric for US locations.");
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

fn scan_dir(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, depth + 1, max_depth, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("gguf") {
            out.push(path);
        }
    }
}

pub fn default_toml() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/config.toml"
    ))
}

/// Settings are single precision: 0.4 serializes as 0.4000000059604645. Written back in
/// their shortest form so the file says what was meant.
fn tidy_floats(table: &mut toml_edit::Table) {
    for (_, item) in table.iter_mut() {
        if let Some(t) = item.as_table_mut() {
            tidy_floats(t);
        } else if let Some(f) = item.as_float() {
            if let Ok(short) = (f as f32).to_string().parse::<f64>() {
                *item = toml_edit::value(short);
            }
        }
    }
}

pub fn merge_table(dst: &mut toml_edit::Table, src: &toml_edit::Table) {
    for (key, value) in src.iter() {
        if let (Some(a), Some(b)) = (
            dst.get_mut(key).and_then(|i| i.as_table_mut()),
            value.as_table(),
        ) {
            merge_table(a, b);
        } else if dst.get(key).map(ToString::to_string) != Some(value.to_string()) {
            let mut replacement = value.clone();
            if let (Some(old), Some(new)) = (
                dst.get(key).and_then(|i| i.as_value()),
                replacement.as_value_mut(),
            ) {
                *new.decor_mut() = old.decor().clone();
            }
            dst.insert(key, replacement);
        }
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> crate::Result<()> {
    use std::io::Write;
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    if let Ok(meta) = fs::metadata(path) {
        temp.as_file().set_permissions(meta.permissions())?;
    }
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| crate::Error::Io(e.error))?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_saves_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = Config::default();
        cfg.assistant.name = "Astra".into();
        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.assistant.name, "Astra");
    }

    #[test]
    fn scans_local_models_finds_ggufs() {
        let models = Config::scan_local_models();
        // The test machine has models installed in LM Studio
        assert!(!models.is_empty(), "expected to find local GGUF models");
    }

    #[test]
    fn autonomy_mode_and_permission_spellings() {
        let cfg: Config = toml::from_str("[agent]\nmode = \"manual\"").unwrap();
        assert_eq!(cfg.agent.mode, assistant_policy::AutonomyMode::Manual);
        assert!(toml::from_str::<Config>("[agent]\nmode = \"yolo\"").is_err());
        let mut cfg = Config::default();
        cfg.permissions.volume = "ask".into();
        assert!(cfg.validate().is_ok());
        cfg.permissions.volume = "alow".into();
        assert!(cfg.validate().is_err());
        let mut cfg = Config::default();
        assert_eq!((cfg.agent.budget().max_calls, cfg.agent.budget().max_active_ms), (4, 180_000));
        cfg.agent.max_calls = 0;
        assert!(cfg.validate().is_err());
        cfg.agent.max_calls = 4;
        cfg.agent.max_active_secs = 5;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn projector_matches_its_model() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["gemma-4-E4B-it-Q4_K_M.gguf", "Qwen3.5-9B-Q4_K_M.gguf", "mmproj-gemma-4-E4B-it-BF16.gguf", "mmproj-Qwen3.5-9B-BF16.gguf"] {
            std::fs::write(dir.path().join(f), b"x").unwrap();
        }
        let v = VisionCfg::default();
        let pick = |m: &str| v.resolve(&dir.path().join(m)).and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));
        assert_eq!(pick("Qwen3.5-9B-Q4_K_M.gguf").as_deref(), Some("mmproj-Qwen3.5-9B-BF16.gguf"));
        assert_eq!(pick("gemma-4-E4B-it-Q4_K_M.gguf").as_deref(), Some("mmproj-gemma-4-E4B-it-BF16.gguf"));
    }

    #[test]
    fn keys_read_and_change_settings() {
        let cfg = Config::default();
        assert_eq!(cfg.get_key("duplex.on_speech"), Some(serde_json::json!("pause")));
        let next = cfg.with_key("duplex.on_speech", parse_value("duck")).unwrap();
        assert_eq!(next.duplex.on_speech, "duck");
        let next = next.with_key("duplex.duck_gain", parse_value("0.4")).unwrap();
        assert!((next.duplex.duck_gain - 0.4).abs() < 1e-6);
        let next = next.with_key("wake_word.enabled", parse_value("false")).unwrap();
        assert!(!next.wake_word.enabled);
        let next = next.with_key("computer.deny_classes", parse_value(r#"["obsidian"]"#)).unwrap();
        assert_eq!(next.computer.deny_classes, vec!["obsidian".to_string()]);
        // Per-tool rules: tool names contain dots, and new ones may be added.
        let next = next.with_key("mcp.permissions.files.write", parse_value("deny")).unwrap();
        assert_eq!(next.mcp.permissions.get("files.write").map(String::as_str), Some("deny"));
        assert_eq!(next.get_key("mcp.permissions.files.write"), Some(serde_json::json!("deny")));
        // Refused: unknown keys, wrong types, values the validator rejects.
        assert!(cfg.with_key("duplex.nonsense", parse_value("1")).is_err());
        assert!(cfg.with_key("duplex.duck_gain", parse_value("loud")).is_err());
        assert!(cfg.with_key("duplex.on_speech", parse_value("shout")).is_err());
        assert!(cfg.with_key("permissions.admin", parse_value("maybe")).is_err());
        assert!(cfg.get_key("nope.nothing").is_none());
    }

    #[test]
    fn shipped_config_parses() {
        let cfg: Config = toml::from_str(default_toml()).unwrap();
        assert_eq!(cfg.assistant.name, "Echo");
        assert_eq!(cfg.assistant.mode, "push-to-talk");
        assert!(cfg.assistant.prompt.contains("short spoken sentences"));
        assert!(cfg.system_prompt().contains("never refuse because of length"));
        assert!(cfg.stt.streaming() && cfg.stt.model == "auto" && cfg.stt.threads == 2 && cfg.stt.device == "cpu");
        assert_eq!(cfg.llm.backend, "llama.cpp");
        assert_eq!(cfg.tts.provider, "kokoro");
        assert_eq!(cfg.permissions.shell, "deny");
        assert_eq!(cfg.agent.mode, assistant_policy::AutonomyMode::Autonomous);
        assert_eq!(cfg.permissions.rules().shutdown, assistant_policy::Decision::Ask);
        assert!(cfg.barge_in.enabled);
        assert_eq!(cfg.barge_in.grace_ms, 500);
        assert!(cfg.wake_word.enabled);
        assert!(cfg.wake_word.phrases.iter().any(|p| p == "echo"));
        assert!(cfg.clock.enabled);
        assert!(cfg.clock.inject_prompt);
        assert!(cfg
            .clock
            .time_phrases
            .iter()
            .any(|p| p == "what time is it"));
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
        assert!(clock
            .spoken_reply("what time does the store close")
            .is_none());
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

#[cfg(test)]
mod safety_tests {
    use super::*;
    #[test]
    fn saves_preserve_user_data_and_refuse_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path,"# User comment\n[assistant]\nname = 'Old' # name comment\ncustom = 'keep me'\n[future]\nvalue = 42\n").unwrap();
        let mut cfg = Config::load_from(&path).unwrap();
        cfg.assistant.name = "New".into();
        cfg.save_to(&path).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains("# User comment")
                && raw.contains("# name comment")
                && raw.contains("keep me")
                && raw.contains("value = 42")
        );
        fs::write(&path, "[assistant\nname = 'broken'").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(cfg.save_to(&path).is_err());
        assert_eq!(before, fs::read(&path).unwrap());
        cfg.http.bind = "0.0.0.0".into();
        assert!(cfg.validate().is_err());
    }
}
