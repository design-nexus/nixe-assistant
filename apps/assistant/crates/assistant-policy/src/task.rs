//! Tasks: one request carried out as a sequence of tool calls, with a state, a budget,
//! task-wide approvals and an outcome that says what could be confirmed.

use crate::{grant::now_ms, Capability};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// How long a task-wide approval lasts at most; it also ends with the task.
pub const TASK_APPROVAL_TTL_MS: u64 = 10 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Planning,
    Running,
    WaitingForApproval,
    Paused,
    Cancelling,
    Cancelled,
    Failed,
    PartiallyCompleted,
    Completed,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Planning => "planning",
            Self::Running => "running",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::Paused => "paused",
            Self::Cancelling => "cancelling",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::PartiallyCompleted => "partially_completed",
            Self::Completed => "completed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.into())).ok()
    }

    pub fn is_final(self) -> bool {
        matches!(self, Self::Cancelled | Self::Failed | Self::PartiallyCompleted | Self::Completed)
    }

    /// Live states a restart interrupts. Paused tasks wait for a person and stay paused.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Queued | Self::Planning | Self::Running | Self::WaitingForApproval | Self::Cancelling)
    }

    /// The one transition table.
    pub fn can_move_to(self, next: Self) -> bool {
        use TaskState::*;
        if self.is_final() {
            return false;
        }
        match (self, next) {
            (_, Cancelling) => self != Cancelling,
            (Cancelling, Cancelled) => true,
            (Cancelling, _) => false,
            (Queued, Planning) => true,
            (Planning, Running | Completed | PartiallyCompleted | Failed | Paused) => true,
            (Running, Planning | WaitingForApproval | Paused | Failed | PartiallyCompleted) => true,
            (WaitingForApproval, Running | Planning | Failed | PartiallyCompleted) => true,
            (Paused, Planning) => true,
            (Paused, Failed) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    /// The tool's own result or a follow-up check confirmed the change.
    Verified,
    /// The call failed or the check showed the change didn't happen.
    Failed,
    /// It ran, but nothing could confirm the effect.
    Unverified,
    /// A passive call; nothing to confirm.
    NotNeeded,
}

impl Verification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Failed => "failed",
            Self::Unverified => "unverified",
            Self::NotNeeded => "not_needed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Budget {
    pub max_calls: usize,
    pub max_active_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub index: usize,
    pub tool: String,
    pub digest: String,
    pub verification: Verification,
    /// A short, redacted summary of the result or the error.
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    pub turn: String,
    pub request: String,
    pub state: TaskState,
    pub budget: Budget,
    pub calls: usize,
    pub active_ms: u64,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub detail: Option<String>,
    pub steps: Vec<Step>,
    #[serde(skip)]
    approvals: Vec<(String, u64)>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);

impl Task {
    pub fn new(turn: impl Into<String>, request: impl Into<String>, budget: Budget) -> Self {
        let now = now_ms();
        Self {
            id: format!("t-{now}-{}", NEXT.fetch_add(1, Ordering::Relaxed)),
            turn: turn.into(),
            request: request.into(),
            state: TaskState::Queued,
            budget,
            calls: 0,
            active_ms: 0,
            created_ms: now,
            updated_ms: now,
            detail: None,
            steps: Vec::new(),
            approvals: Vec::new(),
        }
    }

    /// Move to `next`, or refuse an illegal transition (the state stays as it was).
    pub fn move_to(&mut self, next: TaskState) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.state.can_move_to(next),
            "task {} can't go from {} to {}",
            self.id,
            self.state.as_str(),
            next.as_str()
        );
        self.state = next;
        self.updated_ms = now_ms();
        if next.is_final() {
            self.approvals.clear();
        }
        Ok(())
    }

    /// The budget left, if any: `Err` names what ran out.
    pub fn within_budget(&self) -> Result<(), &'static str> {
        if self.calls >= self.budget.max_calls {
            return Err("call limit reached");
        }
        if self.active_ms >= self.budget.max_active_ms {
            return Err("time limit reached");
        }
        Ok(())
    }

    /// A state-changing call that already succeeded with these exact arguments.
    pub fn already_done(&self, tool: &str, digest: &str) -> bool {
        !Capability::of(tool).passive(tool)
            && self.steps.iter().any(|s| {
                s.tool == tool
                    && s.digest == digest
                    && matches!(s.verification, Verification::Verified | Verification::Unverified)
            })
    }

    pub fn push_step(&mut self, tool: &str, digest: String, verification: Verification, summary: String) -> &Step {
        self.steps.push(Step { index: self.steps.len(), tool: tool.into(), digest, verification, summary });
        self.updated_ms = now_ms();
        self.steps.last().unwrap()
    }

    /// The final state from the steps: everything confirmed, some of it, or nothing.
    pub fn outcome(&self) -> TaskState {
        let changes: Vec<_> = self.steps.iter().filter(|s| s.verification != Verification::NotNeeded).collect();
        let succeeded = self.steps.iter().any(|s| s.verification != Verification::Failed);
        if changes.iter().all(|s| s.verification == Verification::Verified) && (succeeded || self.steps.is_empty()) {
            TaskState::Completed
        } else if succeeded {
            TaskState::PartiallyCompleted
        } else {
            TaskState::Failed
        }
    }

    /// Remember a task-wide approval. Refused for capabilities that need a key press each time.
    pub fn approve_for_task(&mut self, tool: &str) -> bool {
        if !Capability::of(tool).task_approvable() || self.state.is_final() {
            return false;
        }
        self.approvals.retain(|(t, _)| t != tool);
        self.approvals.push((tool.into(), now_ms() + TASK_APPROVAL_TTL_MS));
        true
    }

    /// Whether an unexpired task-wide approval covers this tool.
    pub fn approved_for_task(&mut self, tool: &str) -> bool {
        let now = now_ms();
        self.approvals.retain(|(_, expires)| *expires > now);
        !self.state.is_final() && self.approvals.iter().any(|(t, _)| t == tool)
    }

    #[cfg(test)]
    fn expire_approvals(&mut self) {
        for (_, expires) in &mut self.approvals {
            *expires = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TaskState::*;

    fn task() -> Task {
        Task::new("1", "do things", Budget { max_calls: 2, max_active_ms: 1000 })
    }

    #[test]
    fn transitions() {
        let legal = [
            (Queued, Planning),
            (Planning, Running),
            (Running, WaitingForApproval),
            (WaitingForApproval, Running),
            (Running, Planning),
            (Running, Paused),
            (Paused, Planning),
            (Planning, Completed),
            (Running, Cancelling),
            (Cancelling, Cancelled),
            (Planning, Failed),
        ];
        for (a, b) in legal {
            assert!(a.can_move_to(b), "{a:?} -> {b:?}");
        }
        let illegal = [
            (Queued, Running),
            (Completed, Planning),
            (Cancelled, Cancelling),
            (Cancelling, Completed),
            (Paused, Running),
            (Failed, Planning),
            (Cancelling, Cancelling),
        ];
        for (a, b) in illegal {
            assert!(!a.can_move_to(b), "{a:?} -> {b:?}");
        }
        let mut t = task();
        assert!(t.move_to(Running).is_err());
        assert_eq!(t.state, Queued);
        t.move_to(Planning).unwrap();
        assert_eq!(TaskState::parse("waiting_for_approval"), Some(WaitingForApproval));
    }

    #[test]
    fn budget_and_outcomes() {
        let mut t = task();
        assert!(t.within_budget().is_ok());
        assert_eq!(t.outcome(), Completed, "nothing to do is done");
        t.push_step("notes.list", "a".into(), Verification::NotNeeded, String::new());
        t.push_step("notes.create", "b".into(), Verification::Verified, String::new());
        t.calls = 2;
        assert_eq!(t.within_budget(), Err("call limit reached"));
        assert_eq!(t.outcome(), Completed);
        t.push_step("hypr.workspace", "c".into(), Verification::Unverified, String::new());
        assert_eq!(t.outcome(), PartiallyCompleted);
        let mut t = task();
        t.push_step("terminal.run", "d".into(), Verification::Failed, String::new());
        assert_eq!(t.outcome(), Failed);
        t.calls = 0;
        t.active_ms = 1000;
        assert_eq!(t.within_budget(), Err("time limit reached"));
    }

    #[test]
    fn repeat_guard_skips_only_state_changes() {
        let mut t = task();
        t.push_step("notes.create", "x".into(), Verification::Verified, String::new());
        t.push_step("notes.list", "y".into(), Verification::NotNeeded, String::new());
        t.push_step("notes.delete", "z".into(), Verification::Failed, String::new());
        assert!(t.already_done("notes.create", "x"));
        assert!(!t.already_done("notes.create", "other"));
        assert!(!t.already_done("notes.list", "y"), "passive calls may repeat");
        assert!(!t.already_done("notes.delete", "z"), "failed calls may be retried");
    }

    #[test]
    fn task_approvals() {
        let mut t = task();
        t.move_to(Planning).unwrap();
        assert!(!t.approve_for_task("terminal.shell"));
        assert!(!t.approve_for_task("system.shutdown"));
        assert!(t.approve_for_task("notes.delete"));
        assert!(t.approved_for_task("notes.delete"));
        assert!(!t.approved_for_task("notes.edit"));
        t.expire_approvals();
        assert!(!t.approved_for_task("notes.delete"), "expired");
        assert!(t.approve_for_task("notes.delete"));
        t.move_to(Completed).unwrap();
        assert!(!t.approved_for_task("notes.delete"), "ends with the task");
    }
}
