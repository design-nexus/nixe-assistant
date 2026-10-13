use assistant_core::Event;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Request {
        request_id: String,
        action: Control,
    },
    PttStart,
    PttStop,
    Cancel,
    Status,
    Shutdown,
    /// Skip STT; run LLM → TTS → speakers on this text.
    Say {
        text: String,
    },
    /// Closed-loop probe: Kokoro → 16 kHz WAV → Parakeet → LLM → speakers.
    Demo,
    /// Reload `apps.toml`, `commands.toml`, and `config.toml` without restarting the models.
    Reload,
    /// Mute or unmute the assistant. If muted is None, toggle.
    Mute {
        #[serde(default)]
        muted: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Envelope {
    Command(Command),
    Event(Event),
}

pub fn encode_event(ev: &Event) -> serde_json::Result<String> {
    serde_json::to_string(ev)
}

pub fn parse_command(line: &str) -> Option<Command> {
    serde_json::from_str(line.trim()).ok()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    Plugins {
        action: String,
        id: Option<String>,
    },
    PluginApply {
        state: serde_json::Value,
    },
    ToolCall {
        name: String,
        arguments: serde_json::Value,
    },
    Status,
    Location {
        action: String,
        city: Option<String>,
    },
    Conversation {
        enabled: bool,
    },
    Dictation {
        action: String,
    },
    BrowserResume,
    BrowserTakeover,
    JobStatus {
        job_id: String,
    },
    JobCancel {
        job_id: String,
    },
    Reload,
    Diagnostics,
    Activity,
    ClearActivity,
    Preview {
        voice: String,
        speed: f32,
    },
    Calibrate,
    ToolMode {
        enabled: bool,
    },
    Approve {
        approval_id: String,
        allow: bool,
    },
}
