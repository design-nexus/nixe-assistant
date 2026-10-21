//! Approvals waiting for a person: one entry per request, answered at most once.
//!
//! An answer for an unknown, expired or already answered ID fails, and "for the rest of the
//! task" only counts when the request offered it.

use super::now_ms;
use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::oneshot;

/// (approved, for the rest of the task)
pub(crate) type Answer = (bool, bool);

struct Entry {
    /// What `status` and Settings show: tool, arguments, summary, `expires_at_ms`,
    /// `task_scope_allowed`, `voice_allowed`.
    info: Value,
    tx: oneshot::Sender<Answer>,
}

#[derive(Default)]
pub(crate) struct Pending {
    map: Mutex<HashMap<String, Entry>>,
}

impl Pending {
    /// Register a request; the receiver gets the one answer.
    pub(crate) fn ask(&self, id: &str, info: Value) -> oneshot::Receiver<Answer> {
        let (tx, rx) = oneshot::channel();
        self.map.lock().unwrap().insert(id.into(), Entry { info, tx });
        rx
    }

    /// Answer a request from a key press (HUD, Settings, TUI, CLI). Returns whether the
    /// approval covers the rest of the task.
    pub(crate) fn answer(&self, id: &str, allow: bool, for_task: bool) -> Result<bool> {
        let Some(entry) = self.map.lock().unwrap().remove(id) else { bail!("approval expired or unknown") };
        if entry.info["expires_at_ms"].as_u64().is_none_or(|deadline| now_ms() >= deadline) {
            bail!("approval expired or unknown");
        }
        let for_task = allow && for_task && entry.info["task_scope_allowed"] == true;
        if entry.tx.send((allow, for_task)).is_err() {
            bail!("approval expired");
        }
        Ok(for_task)
    }

    pub(crate) fn remove(&self, id: &str) {
        self.map.lock().unwrap().remove(id);
    }

    /// Drop every request for one of these tools (their plugin is changing).
    pub(crate) fn remove_tools(&self, tools: &[Value]) {
        self.map.lock().unwrap().retain(|_, e| !tools.contains(&e.info["tool"]));
    }

    pub(crate) fn contains(&self, id: &str) -> bool {
        self.map.lock().unwrap().contains_key(id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.map.lock().unwrap().is_empty()
    }

    /// ID → info, for `status`.
    pub(crate) fn snapshot(&self) -> HashMap<String, Value> {
        self.map.lock().unwrap().iter().map(|(id, e)| (id.clone(), e.info.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info(expires: u64, task: bool) -> Value {
        json!({"tool": "notes.delete", "expires_at_ms": expires, "task_scope_allowed": task})
    }

    #[test]
    fn answered_once() {
        let p = Pending::default();
        let mut rx = p.ask("a", info(now_ms() + 60_000, true));
        assert!(p.answer("a", true, true).unwrap());
        assert_eq!(rx.try_recv().unwrap(), (true, true));
        assert!(p.answer("a", true, false).is_err(), "a second answer is refused");
        assert!(p.answer("nope", true, false).is_err(), "unknown ID");
        assert!(p.is_empty());
    }

    #[test]
    fn expired_requests_cant_be_approved() {
        let p = Pending::default();
        let mut rx = p.ask("old", info(now_ms() - 1, false));
        assert!(p.answer("old", true, false).is_err());
        assert!(rx.try_recv().is_err(), "nothing was sent");
        assert!(!p.contains("old"), "and it's gone");
    }

    #[test]
    fn task_scope_only_when_offered() {
        let p = Pending::default();
        let mut rx = p.ask("k", info(now_ms() + 60_000, false));
        assert!(!p.answer("k", true, true).unwrap());
        assert_eq!(rx.try_recv().unwrap(), (true, false));
        let mut rx = p.ask("d", info(now_ms() + 60_000, true));
        assert!(!p.answer("d", false, true).unwrap(), "a deny never covers the task");
        assert_eq!(rx.try_recv().unwrap(), (false, false));
    }

    #[test]
    fn a_request_that_stopped_waiting_cant_be_answered() {
        let p = Pending::default();
        drop(p.ask("gone", info(now_ms() + 60_000, false)));
        assert!(p.answer("gone", true, false).is_err());
        let _rx = p.ask("x", json!({"tool": "admin.install", "expires_at_ms": now_ms() + 60_000}));
        p.remove_tools(&[json!("admin.install")]);
        assert!(p.answer("x", true, false).is_err(), "dropped with its plugin");
    }
}
