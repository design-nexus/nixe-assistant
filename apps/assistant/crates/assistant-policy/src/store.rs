//! The audit journal: one SQLite table of decisions, approvals and call outcomes.
//!
//! Writes go through a channel to one writer thread, so a slow disk never holds up a voice
//! turn; a failed write is logged and dropped. Arguments are stored redacted.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use serde_json::{Map, Value};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};

const SCHEMA_VERSION: i32 = 1;
const MAX_VALUE: usize = 256;
/// Keys whose values are always secret, anywhere in the name.
const SECRET_WORDS: &[&str] = &["password", "passwd", "secret", "token", "cookie", "credential"];
/// Short words that are secret only as the whole key or its last part (not `keyword`).
const SECRET_ENDINGS: &[&str] = &["key", "auth", "authorization", "apikey", "session"];

#[derive(Debug, Clone, Default, Serialize)]
pub struct Record {
    pub at_ms: u64,
    pub turn: Option<String>,
    /// `decision`, `approval` or `call`.
    pub event: String,
    pub tool: String,
    pub capability: String,
    pub decision: String,
    pub rule: String,
    pub approval_id: Option<String>,
    /// `ok`, `error`, `denied`, `approved`, `expired`, `cancelled`.
    pub outcome: String,
    pub duration_ms: Option<u64>,
    /// Redacted JSON.
    pub args: String,
    pub detail: Option<String>,
}

impl Record {
    pub fn new(event: &str, tool: &str, args: &Value) -> Self {
        Self {
            at_ms: crate::grant::now_ms(),
            event: event.into(),
            tool: tool.into(),
            capability: serde_json::to_value(crate::Capability::of(tool))
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            args: redact(args).to_string(),
            ..Self::default()
        }
    }
}

pub struct AuditLog {
    tx: Mutex<Option<mpsc::Sender<Record>>>,
}

impl AuditLog {
    /// Open (creating it owner-only) and start the writer thread.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = connect(path)?;
        let (tx, rx) = mpsc::channel::<Record>();
        std::thread::Builder::new().name("assistant-audit".into()).spawn(move || {
            for record in rx {
                if let Err(err) = insert(&conn, &record) {
                    tracing::warn!(%err, "audit write failed");
                }
            }
        })?;
        Ok(Self { tx: Mutex::new(Some(tx)) })
    }

    /// A journal that keeps nothing (plugin workers, tests, or when the file can't open).
    pub fn disabled() -> Self {
        Self { tx: Mutex::new(None) }
    }

    pub fn record(&self, record: Record) {
        if let Some(tx) = self.tx.lock().unwrap().as_ref() {
            let _ = tx.send(record);
        }
    }
}

fn connect(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY,
                at_ms INTEGER NOT NULL,
                turn TEXT,
                event TEXT NOT NULL,
                tool TEXT NOT NULL,
                capability TEXT NOT NULL,
                decision TEXT NOT NULL,
                rule TEXT NOT NULL,
                approval_id TEXT,
                outcome TEXT NOT NULL,
                duration_ms INTEGER,
                args TEXT NOT NULL,
                detail TEXT
            );
            CREATE INDEX IF NOT EXISTS audit_at ON audit(at_ms);",
        )?;
    }
    anyhow::ensure!(version <= SCHEMA_VERSION, "audit journal is from a newer assistant (schema {version})");
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

fn insert(conn: &Connection, r: &Record) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO audit (at_ms, turn, event, tool, capability, decision, rule, approval_id,
            outcome, duration_ms, args, detail)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            r.at_ms as i64,
            r.turn,
            r.event,
            r.tool,
            r.capability,
            r.decision,
            r.rule,
            r.approval_id,
            r.outcome,
            r.duration_ms.map(|d| d as i64),
            r.args,
            r.detail.as_deref().map(|d| truncate(d, 1024)),
        ],
    )
}

/// The newest `limit` records, oldest first. Reads without the daemon running.
pub fn recent(path: &Path, limit: usize) -> Result<Vec<Record>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT at_ms, turn, event, tool, capability, decision, rule, approval_id, outcome,
            duration_ms, args, detail FROM audit ORDER BY id DESC LIMIT ?1",
    )?;
    let mut rows = stmt
        .query_map([limit as i64], |r| {
            Ok(Record {
                at_ms: r.get::<_, i64>(0)? as u64,
                turn: r.get(1)?,
                event: r.get(2)?,
                tool: r.get(3)?,
                capability: r.get(4)?,
                decision: r.get(5)?,
                rule: r.get(6)?,
                approval_id: r.get(7)?,
                outcome: r.get(8)?,
                duration_ms: r.get::<_, Option<i64>>(9)?.map(|d| d as u64),
                args: r.get(10)?,
                detail: r.get(11)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.reverse();
    Ok(rows)
}

/// Keys stay; secret-looking keys and long strings lose their values.
pub fn redact(v: &Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let lower = k.to_ascii_lowercase();
                    let last = lower.rsplit(['_', '-', '.']).next().unwrap_or(&lower);
                    let secret = SECRET_WORDS.iter().any(|s| lower.contains(s)) || SECRET_ENDINGS.contains(&last);
                    let value = if secret && !v.is_null() {
                        Value::String(format!("<redacted len={}>", v.to_string().len()))
                    } else {
                        redact(v)
                    };
                    (k.clone(), value)
                })
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        Value::String(s) if s.chars().count() > MAX_VALUE => Value::String(format!("<redacted len={}>", s.len())),
        other => other.clone(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub fn default_path(state_dir: &Path) -> PathBuf {
    state_dir.join("agent.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redaction_keeps_keys() {
        let long = "x".repeat(300);
        let out = redact(
            &json!({"api_key":"abc","nested":{"Password":"p"},"script":long,"argv":["ls","-la"],"token":null,"keyword":"rust"}),
        );
        assert_eq!(out["api_key"], "<redacted len=5>");
        assert_eq!(out["nested"]["Password"], "<redacted len=3>");
        assert_eq!(out["script"], "<redacted len=300>");
        assert_eq!(out["argv"], json!(["ls", "-la"]));
        assert_eq!(out["token"], Value::Null);
        assert_eq!(out["keyword"], "rust");
    }

    #[test]
    fn journal_round_trip_and_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/agent.db");
        let log = AuditLog::open(&path).unwrap();
        let mut r = Record::new("call", "terminal.shell", &json!({"script":"true","secret":"s"}));
        r.decision = "confirm".into();
        r.outcome = "ok".into();
        log.record(r);
        let mut r = Record::new("approval", "notes.delete", &json!({}));
        r.outcome = "denied".into();
        log.record(r);
        drop(log);
        let mut rows = Vec::new();
        for _ in 0..50 {
            rows = recent(&path, 10).unwrap();
            if rows.len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tool, "terminal.shell");
        assert_eq!(rows[0].capability, "terminal_generated");
        assert!(rows[0].args.contains("<redacted len=3>"));
        assert_eq!(rows[1].outcome, "denied");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
    }
}
