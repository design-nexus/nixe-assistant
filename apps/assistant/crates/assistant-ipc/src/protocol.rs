use assistant_core::Event;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    PttStart,
    PttStop,
    Cancel,
    Status,
    Shutdown,
    /// Skip STT; run LLM → TTS → speakers on this text.
    Say { text: String },
    /// Closed-loop probe: Kokoro → 16 kHz WAV → Voxtype → LLM → speakers.
    Demo,
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
