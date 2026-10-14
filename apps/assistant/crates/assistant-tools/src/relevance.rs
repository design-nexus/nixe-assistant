//! Narrow the planner's tool catalog to what a request is about.
//!
//! Every tool description costs context and every extra tool is another wrong choice for a
//! small model. Tools are scored by the words they share with the request; a hit brings in
//! its whole group (`notes.list` with `notes.delete`). Groups scoring at least half the best
//! are added best first up to a cap, and tools the task already used stay. If nothing
//! matches, the full catalog goes.

use serde_json::Value;
use std::collections::BTreeMap;

/// Most tools offered for one planning step when the request matches something.
pub const MAX_TOOLS: usize = 24;

const STOPWORDS: &[&str] = &[
    "use", "tools", "tool", "the", "and", "then", "please", "for", "with", "from", "into", "that", "this", "what", "are", "can",
    "you", "your", "my", "me", "all", "any", "set", "to", "it", "of", "in", "on", "is", "a", "an", "be", "do", "now", "want",
    "would", "like", "true", "false",
];

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !STOPWORDS.contains(w))
        .map(|w| w.strip_suffix('s').filter(|s| s.len() >= 3).unwrap_or(w).to_string())
        .collect()
}

fn matches(a: &str, b: &str) -> bool {
    a == b || (a.len().min(b.len()) >= 4 && (a.starts_with(b) || b.starts_with(a)))
}

fn group(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

/// The catalog entries (`{"name","description",…}`) worth offering for `request`, keeping
/// any named in `keep` (tools the task already used).
pub fn relevant(catalog: &[Value], request: &str, keep: &[&str]) -> Vec<Value> {
    let asked = words(request);
    let mut scores: BTreeMap<&str, usize> = BTreeMap::new();
    for tool in catalog {
        let name = tool["name"].as_str().unwrap_or("");
        // The group names the subject ("note", "calendar") and counts most; the action
        // ("list", "delete") is shared by many groups; descriptions are often boilerplate.
        let (subject, action) = name.split_once('.').unwrap_or((name, ""));
        let subject = words(&subject.replace('_', " "));
        let action = words(&action.replace('_', " "));
        let description = words(tool["description"].as_str().unwrap_or(""));
        let hits = |ws: &[String], a: &str| ws.iter().any(|w| matches(a, w));
        let score: usize = asked
            .iter()
            .map(|a| 4 * hits(&subject, a) as usize + 2 * hits(&action, a) as usize + hits(&description, a) as usize)
            .sum();
        let entry = scores.entry(group(name)).or_default();
        *entry = (*entry).max(score);
    }
    let top = scores.values().copied().max().unwrap_or(0);
    let floor = (top / 2).max(1);
    let mut groups: Vec<_> = scores.into_iter().filter(|(_, s)| *s >= floor).collect();
    if groups.is_empty() {
        return catalog.to_vec();
    }
    groups.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let size = |g: &str| catalog.iter().filter(|t| group(t["name"].as_str().unwrap_or("")) == g).count();
    let mut chosen: Vec<&str> = Vec::new();
    let mut count = 0;
    for (g, _) in groups {
        let n = size(g);
        if !chosen.is_empty() && count + n > MAX_TOOLS {
            continue;
        }
        chosen.push(g);
        count += n;
    }
    catalog
        .iter()
        .filter(|t| {
            let name = t["name"].as_str().unwrap_or("");
            chosen.contains(&group(name)) || keep.contains(&name)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog() -> Vec<Value> {
        let mut c = vec![];
        for (n, d) in [
            ("notes.create", "Create a note"),
            ("notes.list", "List saved notes"),
            ("notes.delete", "Delete a note by id"),
            ("calendar.create", "Create a calendar event"),
            ("calendar.list", "List upcoming events"),
            ("weather.lookup", "Current weather and forecast"),
            ("terminal.run", "Run an exact argument array as a managed terminal job"),
            ("terminal.status", "Read terminal job output"),
            ("files.read", "Read up to 64 KiB of a file inside HOME"),
        ] {
            c.push(json!({"name": n, "description": d}));
        }
        for i in 0..30 {
            c.push(json!({"name": format!("hypr.action{i}"), "description": "Execute a configured desktop voice phrase"}));
        }
        c
    }

    fn names(v: &[Value]) -> Vec<&str> {
        v.iter().map(|t| t["name"].as_str().unwrap()).collect()
    }

    #[test]
    fn whole_group_for_a_hit() {
        let out = relevant(&catalog(), "Use tools to delete note abc and then delete note def.", &[]);
        assert_eq!(names(&out), ["notes.create", "notes.list", "notes.delete"]);
    }

    #[test]
    fn several_groups_and_stems() {
        let out = relevant(&catalog(), "What's the weather, and list my calendar events?", &[]);
        let n = names(&out);
        assert!(n.contains(&"weather.lookup") && n.contains(&"calendar.list"), "{n:?}");
        assert!(!n.contains(&"notes.delete"));
    }

    #[test]
    fn cap_skips_big_groups_after_the_first() {
        let out = relevant(&catalog(), "read the file notes.txt and run a desktop action", &[]);
        assert!(out.len() <= MAX_TOOLS + 1, "{}", out.len());
    }

    #[test]
    fn keeps_used_tools_and_falls_back() {
        let out = relevant(&catalog(), "delete note abc", &["terminal.status"]);
        assert!(names(&out).contains(&"terminal.status"));
        assert_eq!(relevant(&catalog(), "xyzzy", &[]).len(), catalog().len());
    }
}
