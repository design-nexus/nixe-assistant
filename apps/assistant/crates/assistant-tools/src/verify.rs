//! After a call: did it take effect, as far as anything can confirm?
//!
//! A verdict per step, not on the whole goal. Errors fail; passive calls need nothing;
//! terminal jobs are confirmed by their exit status; tools that re-read what they changed
//! report `verified`; abilities that return an undo record have confirmed their own change;
//! anything else ran but stays unverified.

use crate::registry::Registry;
use assistant_core::Config;
use assistant_policy::{Capability, Verification};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    Done(Verification),
    /// Wait for this terminal job to finish.
    Job(String),
}

pub fn check(tool: &str, result: &Value) -> Check {
    if result["isError"] == true {
        return Check::Done(Verification::Failed);
    }
    if Capability::of(tool).passive(tool) {
        return Check::Done(Verification::NotNeeded);
    }
    let data = &result["structuredContent"];
    if matches!(tool, "terminal.run" | "terminal.shell" | "terminal.configured") {
        return match data["job_id"].as_str() {
            Some(id) => Check::Job(id.into()),
            None => Check::Done(Verification::Unverified),
        };
    }
    // Tools that re-read what they changed say so.
    if let Some(verified) = data["verified"].as_bool() {
        return Check::Done(if verified { Verification::Verified } else { Verification::Failed });
    }
    if data["undo"].is_object() {
        Check::Done(Verification::Verified)
    } else {
        Check::Done(Verification::Unverified)
    }
}

/// The verdict from a `terminal.status` result, or `None` while the job still runs.
pub fn job_verdict(status: &Value) -> Option<Verification> {
    match status["status"].as_str() {
        Some("running") => None,
        Some("finished") if status["exit_code"] == 0 => Some(Verification::Verified),
        Some(_) => Some(Verification::Failed),
        None => Some(Verification::Unverified),
    }
}

/// Check a step, waiting for a terminal job until `deadline`.
pub async fn verify(
    registry: &Registry,
    tool: &str,
    result: &Value,
    cfg: &Config,
    cancel: &CancellationToken,
    deadline: Instant,
) -> Verification {
    let id = match check(tool, result) {
        Check::Done(v) => return v,
        Check::Job(id) => id,
    };
    let mut wait = Duration::from_millis(250);
    loop {
        let status = match registry.call_allowed("terminal.status", json!({ "job_id": id }), cfg, cancel).await {
            Ok(v) => v,
            Err(_) => return Verification::Unverified,
        };
        if let Some(v) = job_verdict(&status["structuredContent"]) {
            return v;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Verification::Unverified;
        }
        tokio::select! {
            _ = cancel.cancelled() => return Verification::Unverified,
            _ = tokio::time::sleep(wait.min(left)) => {}
        }
        wait = (wait * 2).min(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Verification::*;

    #[test]
    fn checks() {
        assert_eq!(check("notes.create", &json!({"isError":true})), Check::Done(Failed));
        assert_eq!(check("notes.list", &json!({"isError":false})), Check::Done(NotNeeded));
        assert_eq!(check("notes.create", &json!({"structuredContent":{"undo":{"tool":"notes.undo"}}})), Check::Done(Verified));
        assert_eq!(check("hypr.workspace", &json!({"content":[]})), Check::Done(Unverified));
        assert_eq!(check("windows.focus", &json!({"structuredContent":{"verified":true}})), Check::Done(Verified));
        assert_eq!(check("windows.close", &json!({"structuredContent":{"verified":false}})), Check::Done(Failed));
        assert_eq!(
            check("files.write", &json!({"structuredContent":{"verified":false,"undo":{"tool":"files.undo"}}})),
            Check::Done(Failed),
            "a failed self-check beats the undo record"
        );
        assert_eq!(check("terminal.run", &json!({"structuredContent":{"job_id":"7-0"}})), Check::Job("7-0".into()));
        assert_eq!(check("terminal.run", &json!({})), Check::Done(Unverified));
    }

    #[test]
    fn job_verdicts() {
        assert_eq!(job_verdict(&json!({"status":"running"})), None);
        assert_eq!(job_verdict(&json!({"status":"finished","exit_code":0})), Some(Verified));
        assert_eq!(job_verdict(&json!({"status":"finished","exit_code":1})), Some(Failed));
        assert_eq!(job_verdict(&json!({"status":"timed_out","exit_code":null})), Some(Failed));
        assert_eq!(job_verdict(&json!({"status":"cancelled"})), Some(Failed));
        assert_eq!(job_verdict(&json!({})), Some(Unverified));
    }
}
