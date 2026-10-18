//! The journal in `agent.db`: decisions, approvals and call outcomes (`audit`), and tasks
//! with their steps (`tasks`, `task_steps`).
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

use crate::task::{Step, Task, TaskState};

const SCHEMA_VERSION: i32 = 3;
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

enum Write {
    Audit(Record),
    Task(TaskRow),
    Step(String, StepRow),
}

/// A task as stored and listed.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TaskRow {
    pub id: String,
    pub turn: String,
    pub request: String,
    pub state: String,
    pub calls: usize,
    pub max_calls: usize,
    pub active_ms: u64,
    pub max_active_ms: u64,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub detail: Option<String>,
    pub plan: Vec<String>,
}

impl From<&Task> for TaskRow {
    fn from(t: &Task) -> Self {
        Self {
            id: t.id.clone(),
            turn: t.turn.clone(),
            request: truncate(&t.request, 1024),
            state: t.state.as_str().into(),
            calls: t.calls,
            max_calls: t.budget.max_calls,
            active_ms: t.active_ms,
            max_active_ms: t.budget.max_active_ms,
            created_ms: t.created_ms,
            updated_ms: t.updated_ms,
            detail: t.detail.clone(),
            plan: t.plan.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StepRow {
    pub index: usize,
    pub at_ms: u64,
    pub tool: String,
    pub verification: String,
    pub summary: String,
}

impl From<&Step> for StepRow {
    fn from(s: &Step) -> Self {
        Self {
            index: s.index,
            at_ms: crate::grant::now_ms(),
            tool: s.tool.clone(),
            verification: s.verification.as_str().into(),
            summary: truncate(&s.summary, 1024),
        }
    }
}

pub struct AuditLog {
    tx: Mutex<Option<mpsc::Sender<Write>>>,
}

impl AuditLog {
    /// Open (creating it owner-only), mark tasks a restart interrupted as failed, and start
    /// the writer thread.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = connect(path)?;
        let interrupted = interrupt_live(&conn)?;
        if interrupted > 0 {
            tracing::warn!(interrupted, "tasks interrupted by a restart marked failed");
        }
        let (tx, rx) = mpsc::channel::<Write>();
        std::thread::Builder::new().name("assistant-audit".into()).spawn(move || {
            for write in rx {
                let result = match &write {
                    Write::Audit(r) => insert(&conn, r),
                    Write::Task(t) => upsert_task(&conn, t),
                    Write::Step(task, s) => insert_step(&conn, task, s),
                };
                if let Err(err) = result {
                    tracing::warn!(%err, "journal write failed");
                }
            }
        })?;
        Ok(Self { tx: Mutex::new(Some(tx)) })
    }

    /// A journal that keeps nothing (plugin workers, tests, or when the file can't open).
    pub fn disabled() -> Self {
        Self { tx: Mutex::new(None) }
    }

    fn send(&self, write: Write) {
        if let Some(tx) = self.tx.lock().unwrap().as_ref() {
            let _ = tx.send(write);
        }
    }

    pub fn record(&self, record: Record) {
        self.send(Write::Audit(record));
    }

    /// Save the task's current state (insert or replace).
    pub fn task(&self, task: &Task) {
        self.send(Write::Task(task.into()));
    }

    pub fn step(&self, task: &Task, step: &Step) {
        self.send(Write::Step(task.id.clone(), step.into()));
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
    if version < 2 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                turn TEXT NOT NULL,
                request TEXT NOT NULL,
                state TEXT NOT NULL,
                calls INTEGER NOT NULL,
                max_calls INTEGER NOT NULL,
                active_ms INTEGER NOT NULL,
                max_active_ms INTEGER NOT NULL,
                created_ms INTEGER NOT NULL,
                updated_ms INTEGER NOT NULL,
                detail TEXT
            );
            CREATE INDEX IF NOT EXISTS tasks_created ON tasks(created_ms);
            CREATE TABLE IF NOT EXISTS task_steps (
                task TEXT NOT NULL,
                idx INTEGER NOT NULL,
                at_ms INTEGER NOT NULL,
                tool TEXT NOT NULL,
                verification TEXT NOT NULL,
                summary TEXT NOT NULL,
                PRIMARY KEY (task, idx)
            );",
        )?;
    }
    if version < 3 {
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN plan TEXT;")?;
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

fn upsert_task(conn: &Connection, t: &TaskRow) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT OR REPLACE INTO tasks (id, turn, request, state, calls, max_calls, active_ms,
            max_active_ms, created_ms, updated_ms, detail, plan)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            t.id,
            t.turn,
            t.request,
            t.state,
            t.calls as i64,
            t.max_calls as i64,
            t.active_ms as i64,
            t.max_active_ms as i64,
            t.created_ms as i64,
            t.updated_ms as i64,
            t.detail.as_deref().map(|d| truncate(d, 1024)),
            (!t.plan.is_empty()).then(|| serde_json::to_string(&t.plan).unwrap_or_default()),
        ],
    )
}

fn insert_step(conn: &Connection, task: &str, s: &StepRow) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT OR REPLACE INTO task_steps (task, idx, at_ms, tool, verification, summary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![task, s.index as i64, s.at_ms as i64, s.tool, s.verification, s.summary],
    )
}

/// Tasks a restart cut off can't be trusted to have finished: mark them failed.
fn interrupt_live(conn: &Connection) -> rusqlite::Result<usize> {
    let live: Vec<_> =
        [TaskState::Queued, TaskState::Planning, TaskState::Running, TaskState::WaitingForApproval, TaskState::Cancelling]
            .iter()
            .map(|s| format!("'{}'", s.as_str()))
            .collect();
    conn.execute(
        &format!(
            "UPDATE tasks SET state = 'failed', detail = 'interrupted by restart', updated_ms = ?1
             WHERE state IN ({})",
            live.join(",")
        ),
        [crate::grant::now_ms() as i64],
    )
}

const TASK_COLUMNS: &str = "id, turn, request, state, calls, max_calls, active_ms, max_active_ms,
    created_ms, updated_ms, detail, plan";

/// The columns to read: a journal from before plans (read-only, not migrated) has none.
fn task_columns(conn: &Connection) -> String {
    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap_or(0);
    if version >= 3 {
        TASK_COLUMNS.into()
    } else {
        TASK_COLUMNS.replace("plan", "NULL AS plan")
    }
}

fn task_row(r: &rusqlite::Row) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: r.get(0)?,
        turn: r.get(1)?,
        request: r.get(2)?,
        state: r.get(3)?,
        calls: r.get::<_, i64>(4)? as usize,
        max_calls: r.get::<_, i64>(5)? as usize,
        active_ms: r.get::<_, i64>(6)? as u64,
        max_active_ms: r.get::<_, i64>(7)? as u64,
        created_ms: r.get::<_, i64>(8)? as u64,
        updated_ms: r.get::<_, i64>(9)? as u64,
        detail: r.get(10)?,
        plan: r.get::<_, Option<String>>(11)?.and_then(|p| serde_json::from_str(&p).ok()).unwrap_or_default(),
    })
}

fn read_only(path: &Path) -> Result<Option<Connection>> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    Ok((version >= 2).then_some(conn))
}

/// The newest `limit` tasks, newest first. Reads without the daemon running.
pub fn tasks(path: &Path, limit: usize) -> Result<Vec<TaskRow>> {
    let Some(conn) = read_only(path)? else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(&format!("SELECT {} FROM tasks ORDER BY created_ms DESC LIMIT ?1", task_columns(&conn)))?;
    let rows = stmt.query_map([limit as i64], task_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One task and its steps.
pub fn task(path: &Path, id: &str) -> Result<Option<(TaskRow, Vec<StepRow>)>> {
    let Some(conn) = read_only(path)? else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(&format!("SELECT {} FROM tasks WHERE id = ?1", task_columns(&conn)))?;
    let Some(task) = stmt.query_map([id], task_row)?.next().transpose()? else {
        return Ok(None);
    };
    let mut stmt = conn.prepare("SELECT idx, at_ms, tool, verification, summary FROM task_steps WHERE task = ?1 ORDER BY idx")?;
    let steps = stmt
        .query_map([id], |r| {
            Ok(StepRow {
                index: r.get::<_, i64>(0)? as usize,
                at_ms: r.get::<_, i64>(1)? as u64,
                tool: r.get(2)?,
                verification: r.get(3)?,
                summary: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some((task, steps)))
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

    fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> T {
        for _ in 0..100 {
            if let Some(v) = f() {
                return v;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("journal write never landed");
    }

    #[test]
    fn v1_journal_gains_task_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE audit (id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, turn TEXT, event TEXT NOT NULL, tool TEXT NOT NULL, capability TEXT NOT NULL, decision TEXT NOT NULL, rule TEXT NOT NULL, approval_id TEXT, outcome TEXT NOT NULL, duration_ms INTEGER, args TEXT NOT NULL, detail TEXT); PRAGMA user_version = 1;").unwrap();
            conn.execute("INSERT INTO audit (at_ms, event, tool, capability, decision, rule, outcome, args) VALUES (1, 'call', 'x', 'external', 'allow', 'default', 'ok', '{}')", []).unwrap();
        }
        assert!(tasks(&path, 5).unwrap().is_empty(), "v1 file reads as no tasks");
        drop(AuditLog::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, 3);
        assert_eq!(recent(&path, 5).unwrap().len(), 1, "audit rows kept");
    }

    fn state_of(path: &Path, id: &str) -> TaskRow {
        task(path, id).unwrap().unwrap().0
    }

    #[test]
    fn tasks_round_trip_and_restart_sweep() {
        use crate::task::{Budget, Verification};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let log = AuditLog::open(&path).unwrap();
        let budget = Budget { max_calls: 4, max_active_ms: 180_000 };
        let mut done = Task::new("1", "make a note", budget);
        done.move_to(TaskState::Planning).unwrap();
        done.set_plan(&["Find the meeting".into(), "  ".into(), "Write   the note".into()]);
        done.push_step("notes.create", "d".into(), Verification::Verified, "created".into());
        log.step(&done, &done.steps[0].clone());
        done.move_to(TaskState::Completed).unwrap();
        log.task(&done);
        let mut live = Task::new("2", "run tests", budget);
        live.move_to(TaskState::Planning).unwrap();
        live.move_to(TaskState::Running).unwrap();
        log.task(&live);
        let mut paused = Task::new("3", "search", budget);
        paused.move_to(TaskState::Planning).unwrap();
        paused.move_to(TaskState::Paused).unwrap();
        log.task(&paused);
        drop(log);
        wait_for(|| (tasks(&path, 10).unwrap().len() == 3).then_some(()));
        let (row, steps) = task(&path, &done.id).unwrap().unwrap();
        assert_eq!(row.state, "completed");
        assert_eq!(row.plan, ["Find the meeting", "Write the note"], "blank steps dropped, spaces tidied");
        assert!(state_of(&path, &live.id).plan.is_empty());
        assert_eq!((steps.len(), steps[0].verification.as_str()), (1, "verified"));
        assert!(task(&path, "t-missing").unwrap().is_none());
        // A restart: the running task can't be trusted to have finished; paused stays paused.
        drop(AuditLog::open(&path).unwrap());
        let state = |id: &str| task(&path, id).unwrap().unwrap().0;
        assert_eq!(state(&live.id).state, "failed");
        assert_eq!(state(&live.id).detail.as_deref(), Some("interrupted by restart"));
        assert_eq!(state(&paused.id).state, "paused");
        assert_eq!(state(&done.id).state, "completed");
    }
}
