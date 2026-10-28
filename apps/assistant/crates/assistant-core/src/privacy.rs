//! Explicit, expiring content diagnostics and owner-only history/retention controls.
use crate::{
    config::{atomic_write, Config},
    paths,
};
use anyhow::Result;
use serde_json::{json, Value};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
pub fn diagnostics(enabled: bool) -> Result<()> {
    atomic_write(
        &paths::runtime_dir().join("content-log-until"),
        if enabled { (now() + 600_000).to_string() } else { "0".into() }.as_bytes(),
    )?;
    Ok(())
}
pub fn log_content(stage: &str, text: &str) {
    let until = std::fs::read_to_string(paths::runtime_dir().join("content-log-until"))
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    if now() < until && until <= now() + 600_000 {
        tracing::info!(stage, content=%text, "temporary content diagnostics");
    }
}
fn history_path() -> std::path::PathBuf {
    paths::state_dir().join("conversation-history.json")
}
fn history_lock() -> Result<std::fs::File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    std::fs::create_dir_all(paths::state_dir())?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(paths::state_dir().join("history.lock"))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(file)
}
pub fn history(query: &str) -> Vec<Value> {
    let Ok(_lock) = history_lock() else {
        return Vec::new();
    };
    read_history(query)
}
fn read_history(query: &str) -> Vec<Value> {
    let rows: Vec<Value> = std::fs::read(history_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let query = query.to_lowercase();
    rows.into_iter().filter(|r| query.is_empty() || r.to_string().to_lowercase().contains(&query)).collect()
}
pub fn remember(user: &str, assistant: &str, cfg: &Config) -> Result<()> {
    if !cfg.privacy.save_history {
        return Ok(());
    }
    let _lock = history_lock()?;
    let mut rows = read_history("");
    let cutoff = now().saturating_sub(cfg.privacy.retention_days as u64 * 86_400_000);
    rows.retain(|r| r["at_ms"].as_u64().unwrap_or(0) >= cutoff);
    rows.push(json!({"at_ms":now(),"user":user.chars().take(8192).collect::<String>(),"assistant":assistant.chars().take(16384).collect::<String>()}));
    let excess = rows.len().saturating_sub(500);
    rows.drain(..excess);
    atomic_write(&history_path(), &serde_json::to_vec(&rows)?)?;
    Ok(())
}
fn size(path: &Path) -> u64 {
    let Ok(meta) = path.symlink_metadata() else { return 0 };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    std::fs::read_dir(path).into_iter().flatten().flatten().map(|e| size(&e.path())).sum()
}
pub fn prune_backups(cfg: &Config, clear: bool) -> Result<Value> {
    prune_backups_reserved(cfg, clear, 0)
}
pub fn prune_backups_reserved(cfg: &Config, clear: bool, reserved: u64) -> Result<Value> {
    let root = paths::state_dir().join("file-backups");
    let mut entries: Vec<_> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let meta = p.symlink_metadata().ok()?;
            if !meta.is_dir() || meta.file_type().is_symlink() {
                return None;
            }
            Some((meta.modified().ok()?, size(&p), p))
        })
        .collect();
    entries.sort_by_key(|e| e.0);
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    let mut removed = 0;
    for (modified, bytes, path) in entries {
        let expired = modified.elapsed().unwrap_or_default().as_secs() > cfg.privacy.retention_days as u64 * 86400;
        if clear || expired || total > (cfg.privacy.backup_limit_mb as u64 * 1024 * 1024).saturating_sub(reserved) {
            std::fs::remove_dir_all(path)?;
            total = total.saturating_sub(bytes);
            removed += 1;
        }
    }
    Ok(json!({"backups_removed":removed,"backup_bytes":total}))
}
pub fn maintenance(cfg: &Config, clear: bool) -> Result<Value> {
    for name in ["task-checkpoint.json", "last-failure.json"] {
        let path = paths::state_dir().join(name);
        if clear
            || path
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|d| d.as_secs() > cfg.privacy.retention_days as u64 * 86400)
        {
            let _ = std::fs::remove_file(path);
        }
    }
    let db = assistant_policy::store::default_path(&paths::state_dir());
    let database = assistant_policy::store::prune(&db, cfg.privacy.retention_days, clear)?;
    let backup = prune_backups(cfg, clear)?;
    let _lock = history_lock()?;
    let mut rows = read_history("");
    let cutoff = now().saturating_sub(cfg.privacy.retention_days as u64 * 86_400_000);
    rows.retain(|r| !clear && r["at_ms"].as_u64().unwrap_or(0) >= cutoff);
    if history_path().exists() {
        atomic_write(&history_path(), &serde_json::to_vec(&rows)?)?;
    }
    Ok(
        json!({"database":database,"backups_removed":backup["backups_removed"],"backup_bytes":backup["backup_bytes"],"history_entries":rows.len()}),
    )
}
pub fn summary() -> Value {
    json!({"backup_bytes":size(&paths::state_dir().join("file-backups")),"history_entries":history("").len(),"content_logs":"off unless temporary diagnostics are explicitly enabled; old journal entries are not removed","dictionary":"managed separately; clearing history preserves learned words"})
}
