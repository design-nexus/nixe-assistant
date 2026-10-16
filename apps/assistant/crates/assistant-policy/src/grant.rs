//! Grants: permission to make exactly one call, bound to the tool and its arguments.
//!
//! A grant comes from a `Verdict` (allowed, or asked and approved), or from a host's
//! attestation when a plugin worker receives a call the host already checked. There is no
//! generic token: changed arguments, another tool, expiry or a second use all fail.

use crate::{Decision, Verdict};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

/// How long a grant stays valid. Calls dispatch right after the decision or the approval.
pub const GRANT_TTL_MS: u64 = 60_000;
/// The `_meta` key a host uses to pass a checked grant to its plugin worker.
pub const META_KEY: &str = "io.github.design_nexus/grant";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantKind {
    /// The policy allowed the call outright.
    Allowed,
    /// The policy asked and the user approved.
    Approved,
    /// A host checked a grant and forwarded the call to this process.
    Attested,
}

impl GrantKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Approved => "approved",
            Self::Attested => "attested",
        }
    }
}

#[derive(Debug)]
pub struct Grant {
    tool: String,
    digest: String,
    expires_at_ms: u64,
    kind: GrantKind,
    approval_id: Option<String>,
    turn: Option<String>,
    spent: AtomicBool,
}

impl Grant {
    /// For a verdict that allows the call.
    pub fn allowed(verdict: &Verdict, args: &Value) -> Result<Self> {
        ensure!(verdict.decision() == Decision::Allow, "{} is not allowed without approval", verdict.tool());
        Ok(Self::new(verdict.tool(), args, GrantKind::Allowed, None))
    }

    /// For a verdict that asked, once the user approved these exact arguments.
    pub fn approved(verdict: &Verdict, args: &Value, approval_id: &str) -> Result<Self> {
        ensure!(verdict.decision().needs_approval(), "{} does not take an approval", verdict.tool());
        Ok(Self::new(verdict.tool(), args, GrantKind::Approved, Some(approval_id.into())))
    }

    /// Rebuild a grant a host attested (see `attest`). The caller still `check`s it.
    pub fn from_attestation(meta: &Value) -> Result<Self> {
        let g = meta.get(META_KEY).context("call carries no grant")?;
        let text = |k: &str| g[k].as_str().map(String::from);
        Ok(Self {
            tool: text("tool").context("grant has no tool")?,
            digest: text("digest").context("grant has no digest")?,
            expires_at_ms: g["expires_at_ms"].as_u64().context("grant has no expiry")?,
            kind: GrantKind::Attested,
            approval_id: text("approval_id"),
            turn: text("turn"),
            spent: AtomicBool::new(false),
        })
    }

    fn new(tool: &str, args: &Value, kind: GrantKind, approval_id: Option<String>) -> Self {
        Self {
            tool: tool.into(),
            digest: digest(args),
            expires_at_ms: now_ms() + GRANT_TTL_MS,
            kind,
            approval_id,
            turn: None,
            spent: AtomicBool::new(false),
        }
    }

    /// Record the turn the grant belongs to, for the audit journal.
    pub fn for_turn(mut self, turn: impl Into<String>) -> Self {
        self.turn = Some(turn.into());
        self
    }

    pub fn kind(&self) -> GrantKind {
        self.kind
    }
    pub fn tool(&self) -> &str {
        &self.tool
    }
    pub fn approval_id(&self) -> Option<&str> {
        self.approval_id.as_deref()
    }
    pub fn turn(&self) -> Option<&str> {
        self.turn.as_deref()
    }

    /// The enforcement check: same tool, same arguments, unexpired, first use.
    pub fn check(&self, tool: &str, args: &Value) -> Result<()> {
        if tool != self.tool {
            bail!("grant is for {}, not {tool}", self.tool);
        }
        if digest(args) != self.digest {
            bail!("arguments changed after the grant");
        }
        if now_ms() >= self.expires_at_ms {
            bail!("grant expired");
        }
        if self.spent.swap(true, Ordering::SeqCst) {
            bail!("grant already used");
        }
        Ok(())
    }

    /// `_meta` for forwarding a checked call to a plugin worker. The host may forward the
    /// call under its wire name and with arguments it filled in (the weather location), so
    /// the attestation names what is actually sent. Only a grant that passed `check` attests.
    pub fn attest(&self, tool: &str, args: &Value) -> Result<Value> {
        ensure!(self.spent.load(Ordering::SeqCst), "only a checked grant can be forwarded");
        Ok(json!({ META_KEY: {
            "tool": tool,
            "digest": digest(args),
            "expires_at_ms": self.expires_at_ms,
            "kind": self.kind.as_str(),
            "approval_id": self.approval_id,
            "turn": self.turn,
        }}))
    }
}

/// SHA-256 over canonical JSON (object keys sorted at every level).
pub fn digest(args: &Value) -> String {
    let mut text = String::new();
    canonical(args, &mut text);
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decide, AutonomyMode, Context as Cx, Rules};

    fn verdict(tool: &str) -> Verdict {
        let rules = Rules::default();
        decide(
            tool,
            &Cx {
                available: true,
                trusted: true,
                tool_rule: None,
                rules: &rules,
                mode: AutonomyMode::Autonomous,
                resource_approved: false,
                runs_code: false,
            },
        )
    }

    #[test]
    fn digest_ignores_key_order() {
        let a: Value = serde_json::from_str(r#"{"b":1,"a":{"y":[1,2],"x":null}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":{"x":null,"y":[1,2]},"b":1}"#).unwrap();
        assert_eq!(digest(&a), digest(&b));
        assert_ne!(digest(&a), digest(&json!({"b":2})));
    }

    #[test]
    fn grants_follow_the_verdict() {
        assert!(Grant::allowed(&verdict("terminal.shell"), &json!({})).is_err());
        assert!(Grant::approved(&verdict("hypr.workspace"), &json!({}), "a").is_err());
        assert!(Grant::allowed(&verdict("hypr.workspace"), &json!({})).is_ok());
        assert!(Grant::approved(&verdict("terminal.shell"), &json!({}), "a").is_ok());
    }

    #[test]
    fn check_binds_tool_args_and_single_use() {
        let args = json!({"script":"true"});
        let g = Grant::approved(&verdict("terminal.shell"), &args, "a1").unwrap();
        assert!(g.attest("terminal.shell", &args).is_err(), "unchecked grants don't forward");
        assert!(g.check("terminal.run", &args).is_err());
        assert!(g.check("terminal.shell", &json!({"script":"rm -rf ~"})).is_err());
        g.check("terminal.shell", &args).unwrap();
        assert!(g.check("terminal.shell", &args).is_err(), "second use");
    }

    #[test]
    fn expired_grants_fail() {
        let args = json!({});
        let mut g = Grant::allowed(&verdict("hypr.workspace"), &args).unwrap();
        g.expires_at_ms = now_ms() - 1;
        assert!(g.check("hypr.workspace", &args).is_err());
    }

    #[test]
    fn attestation_round_trip() {
        let args = json!({"city":""});
        let g = Grant::allowed(&verdict("weather.lookup"), &args).unwrap().for_turn("t1");
        g.check("weather.lookup", &args).unwrap();
        let sent = json!({"city":"Austin"});
        let meta = g.attest("weather.lookup", &sent).unwrap();
        let w = Grant::from_attestation(&meta).unwrap();
        assert_eq!((w.kind(), w.turn()), (GrantKind::Attested, Some("t1")));
        assert!(w.check("weather.lookup", &args).is_err(), "attests what was sent");
        w.check("weather.lookup", &sent).unwrap();
        assert!(Grant::from_attestation(&json!({})).is_err());
    }
}
