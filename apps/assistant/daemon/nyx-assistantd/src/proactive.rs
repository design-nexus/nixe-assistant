//! Small trusted event inbox. The host supplies source IDs; producers cannot claim another source.

use assistant_core::{
    config::{EventPreference, EventsCfg},
    paths,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub struct Notice {
    pub source: String,
    pub kind: String,
    pub id: String,
    pub title: String,
    pub text: String,
    pub speech: Option<String>,
    pub spool: Option<PathBuf>,
}

impl Notice {
    pub fn plugin(source: &str, value: &Value) -> Option<Self> {
        let id = value["id"].as_str()?;
        let title = value["title"].as_str()?;
        let text = value["text"].as_str()?;
        let kind = value["kind"].as_str().unwrap_or("generic");
        let notice = Self {
            source: format!("plugin.{source}"),
            kind: kind.into(),
            id: id.into(),
            title: title.into(),
            text: text.into(),
            speech: value["speech"].as_str().map(str::to_string),
            spool: None,
        };
        notice.valid().then_some(notice)
    }

    fn valid(&self) -> bool {
        !self.source.is_empty()
            && self.source.len() <= 160
            && !self.kind.is_empty()
            && self.kind.len() <= 80
            && self.kind.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            && !self.id.is_empty()
            && self.id.len() <= 128
            && !self.title.is_empty()
            && self.title.len() <= 120
            && !self.text.is_empty()
            && self.text.len() <= 2048
            && self.speech.as_ref().is_none_or(|s| s.len() <= 512)
    }

    fn key(&self) -> String {
        format!("{}:{}:{}", self.source, self.kind, self.id)
    }
    fn rate_key(&self) -> String {
        format!("{}:{}", self.source, self.kind)
    }
    pub fn preference(&self, cfg: &EventsCfg) -> EventPreference {
        cfg.preferences
            .get(&format!("{}.{}", self.source, self.kind))
            .or_else(|| cfg.preferences.get(&self.kind))
            .cloned()
            .unwrap_or_else(|| EventPreference { speak: self.kind == "reminder.due", ..Default::default() })
    }
}

#[derive(Default)]
struct State {
    queue: VecDeque<Notice>,
    recent: VecDeque<Value>,
    delivered: HashMap<String, u64>,
    last_kind: HashMap<String, u64>,
    retry_after: HashMap<String, u64>,
}

#[derive(Default)]
pub struct Hub {
    state: Mutex<State>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

impl Hub {
    pub fn submit(&self, notice: Notice) -> bool {
        if !notice.valid() {
            return false;
        }
        let mut state = self.state.lock().unwrap();
        if state.queue.iter().any(|n| n.key() == notice.key()) {
            return true;
        }
        if state.queue.len() >= 100 {
            return false;
        }
        state.queue.push_back(notice);
        true
    }

    pub fn next(&self, cfg: &EventsCfg, muted_or_dnd: bool) -> Option<(Notice, EventPreference, bool)> {
        let mut state = self.state.lock().unwrap();
        let now = now_ms();
        state.delivered.retain(|_, at| now.saturating_sub(*at) <= cfg.duplicate_window_secs.saturating_mul(1000));
        for _ in 0..state.queue.len() {
            let notice = state.queue.pop_front()?;
            let preference = notice.preference(cfg);
            let disabled = !cfg.enabled || (!preference.notify && !preference.speak);
            let duplicate = state.delivered.contains_key(&notice.key());
            if disabled || duplicate {
                return Some((notice, preference, true));
            }
            if state.retry_after.get(&notice.key()).is_some_and(|until| now < *until) {
                state.queue.push_back(notice);
                continue;
            }
            let too_soon = state
                .last_kind
                .get(&notice.rate_key())
                .is_some_and(|at| now.saturating_sub(*at) < cfg.rate_limit_secs.saturating_mul(1000));
            if muted_or_dnd || too_soon {
                state.queue.push_back(notice);
                continue;
            }
            return Some((notice, preference, false));
        }
        None
    }

    pub fn retry(&self, notice: Notice) {
        self.defer(notice, 30_000);
    }

    pub fn defer(&self, notice: Notice, delay_ms: u64) {
        let mut state = self.state.lock().unwrap();
        if state.queue.len() < 100 {
            state.retry_after.insert(notice.key(), now_ms() + delay_ms);
            state.queue.push_back(notice);
        }
    }

    pub fn finish(&self, notice: &Notice, outcome: &str) -> Value {
        let mut state = self.state.lock().unwrap();
        let now = now_ms();
        state.delivered.insert(notice.key(), now);
        state.retry_after.remove(&notice.key());
        if outcome == "delivered" {
            state.last_kind.insert(notice.rate_key(), now);
        }
        let record = json!({"source":notice.source,"kind":notice.kind,"id":notice.id,"title":notice.title,"text":notice.text,"outcome":outcome,"at_ms":now});
        state.recent.push_back(record.clone());
        while state.recent.len() > 100 {
            state.recent.pop_front();
        }
        if let Some(path) = &notice.spool {
            let _ = std::fs::remove_file(path);
        }
        record
    }

    pub fn recent(&self) -> Vec<Value> {
        self.state.lock().unwrap().recent.iter().cloned().collect()
    }

    pub fn has_pending(&self) -> bool { !self.state.lock().unwrap().queue.is_empty() }
}

const UNITS: &[&str] = &["nyx-assistantd.service", "nyx-assistant-stt.service"];

fn spool_path(unit: &str) -> PathBuf {
    paths::state_dir().join(format!("event-{unit}.json"))
}

/// Called only by the installed systemd failure units; the unit is an allowlisted argument.
pub fn spool_failure(unit: &str) -> anyhow::Result<()> {
    anyhow::ensure!(UNITS.contains(&unit), "unsupported service event");
    let data = json!({"unit":unit,"at_ms":now_ms()});
    assistant_core::config::atomic_write(&spool_path(unit), &serde_json::to_vec(&data)?)?;
    Ok(())
}

pub fn spooled() -> Vec<Notice> {
    UNITS
        .iter()
        .filter_map(|unit| {
            let path = spool_path(unit);
            let data: Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            if data["unit"] != *unit {
                return None;
            }
            let at = data["at_ms"].as_u64()?;
            let notice = Notice {
                source: "systemd".into(),
                kind: "service.failed".into(),
                id: format!("{unit}:{at}"),
                title: "Assistant service failed".into(),
                text: format!("{unit} stopped after repeated failures. Review its recent journal before changing it."),
                speech: None,
                spool: Some(path),
            };
            notice.valid().then_some(notice)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_and_suppresses_duplicates() {
        let hub = Hub::default();
        let notice =
            Notice::plugin("reminders", &json!({"id":"7","kind":"reminder.due","title":"Reminders","text":"Call Sam"})).unwrap();
        assert!(hub.submit(notice.clone()) && hub.submit(notice.clone()));
        let cfg = EventsCfg::default();
        let (first, pref, suppressed) = hub.next(&cfg, false).unwrap();
        assert!(!suppressed && pref.speak);
        hub.finish(&first, "delivered");
        assert!(hub.submit(notice));
        assert!(hub.next(&cfg, false).unwrap().2);
    }
}
