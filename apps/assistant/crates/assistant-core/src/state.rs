use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssistantState {
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Executing,
    Speaking,
    Interrupted,
    Error,
}

impl AssistantState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Listening => "listening",
            Self::Transcribing => "transcribing",
            Self::Thinking => "thinking",
            Self::Executing => "executing",
            Self::Speaking => "speaking",
            Self::Interrupted => "interrupted",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for AssistantState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct StateMachine {
    current: AssistantState,
}

impl StateMachine {
    pub fn new() -> Self {
        Self {
            current: AssistantState::Idle,
        }
    }

    pub fn current(&self) -> AssistantState {
        self.current
    }

    pub fn set(&mut self, next: AssistantState) -> Option<(AssistantState, AssistantState)> {
        if self.current == next {
            return None;
        }
        let prev = self.current;
        self.current = next;
        Some((prev, next))
    }
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}
