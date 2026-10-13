use crate::state::AssistantState;
use crate::timing::TurnTiming;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Plugins {
        state: serde_json::Value,
    },
    Session {
        session_id: u64,
        mode: String,
        remaining_secs: u64,
        target: String,
        draft: String,
    },
    Result {
        request_id: String,
        ok: bool,
        data: serde_json::Value,
        error: Option<String>,
    },
    Turn {
        turn_id: u64,
        event: serde_json::Value,
    },
    ToolApproval {
        approval_id: String,
        turn_id: u64,
        tool: String,
        arguments: serde_json::Value,
        expires_at_ms: u64,
    },
    Settings {
        running: serde_json::Value,
        restart_required: Vec<String>,
        hud_enabled: bool,
    },
    State {
        state: AssistantState,
        at_ms: u64,
    },
    PartialTranscript {
        text: String,
    },
    Transcript {
        text: String,
    },
    AssistantText {
        text: String,
        done: bool,
    },
    Level {
        rms: f32,
    },
    Timing {
        stages: Vec<TimingStage>,
        total_ms: u64,
    },
    Error {
        message: String,
    },
    Health {
        ok: bool,
        state: AssistantState,
        name: String,
        model: String,
        model_loaded: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingStage {
    pub name: String,
    pub ms: u64,
}

impl Event {
    pub fn from_timing(t: &TurnTiming) -> Self {
        let stages = t.stages();
        let total_ms = stages.last().map(|s| s.ms).unwrap_or(0);
        Self::Timing { stages, total_ms }
    }
}
