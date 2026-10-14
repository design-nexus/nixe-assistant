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

/// What each part of the assistant is doing, independently (plan §4.5): it can listen while
/// it speaks, think while a task runs, and so on. The single [`AssistantState`] above stays
/// for the HUD and TUI; each change to it also updates these, and duplex listening sets the
/// parts it can't express (listening and ducked while speaking).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub mic: Mic,
    pub recognition: Recognition,
    pub reasoning: Busy,
    pub synthesis: Busy,
    pub playback: Playback,
    pub agent: Busy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mic {
    #[default]
    Off,
    Listening,
    /// Listening for an interruption while the assistant speaks.
    Monitoring,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recognition {
    #[default]
    Idle,
    Streaming,
    Finalizing,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Busy {
    #[default]
    Idle,
    Busy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Playback {
    #[default]
    Idle,
    Playing,
    Ducked,
    /// Held in place while someone speaks.
    Paused,
}

impl Activity {
    /// Follow a change of the single state. Parts it says nothing about are kept: an agent
    /// task keeps running while the assistant goes back to idle.
    pub fn follow(&mut self, state: AssistantState) {
        let busy = |b| if b { Busy::Busy } else { Busy::Idle };
        match state {
            AssistantState::Idle | AssistantState::Error | AssistantState::Interrupted => {
                self.mic = Mic::Off;
                self.recognition = Recognition::Idle;
                self.reasoning = Busy::Idle;
                self.synthesis = Busy::Idle;
                self.playback = Playback::Idle;
            }
            AssistantState::Listening => {
                self.mic = Mic::Listening;
                self.recognition = Recognition::Streaming;
                self.playback = Playback::Idle;
            }
            AssistantState::Transcribing => {
                self.mic = Mic::Off;
                self.recognition = Recognition::Finalizing;
            }
            AssistantState::Thinking => {
                self.recognition = Recognition::Idle;
                self.reasoning = busy(true);
            }
            AssistantState::Executing => {
                self.recognition = Recognition::Idle;
                self.agent = busy(true);
            }
            AssistantState::Speaking => {
                self.recognition = Recognition::Idle;
                self.synthesis = busy(true);
                if self.playback == Playback::Idle {
                    self.playback = Playback::Playing;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_follows_the_single_state() {
        let mut a = Activity::default();
        a.follow(AssistantState::Listening);
        assert_eq!((a.mic, a.recognition), (Mic::Listening, Recognition::Streaming));
        a.follow(AssistantState::Transcribing);
        assert_eq!((a.mic, a.recognition), (Mic::Off, Recognition::Finalizing));
        a.follow(AssistantState::Executing);
        a.follow(AssistantState::Thinking);
        a.follow(AssistantState::Speaking);
        assert_eq!((a.reasoning, a.synthesis, a.playback, a.agent), (Busy::Busy, Busy::Busy, Playback::Playing, Busy::Busy));
        // Duplex marks it ducked; a repeated Speaking doesn't undo that.
        a.playback = Playback::Ducked;
        a.mic = Mic::Monitoring;
        a.follow(AssistantState::Speaking);
        assert_eq!((a.playback, a.mic), (Playback::Ducked, Mic::Monitoring));
        a.follow(AssistantState::Idle);
        assert_eq!(a.playback, Playback::Idle);
        assert_eq!(a.agent, Busy::Busy, "a task runs on past the reply");
        let json = serde_json::to_value(a).unwrap();
        assert_eq!(json["playback"], "idle");
        assert_eq!(json["agent"], "busy");
    }
}
