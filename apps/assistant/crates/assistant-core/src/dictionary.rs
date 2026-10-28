//! Small personal vocabulary ranking; guesses never become learned facts automatically.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Suggestion {
    pub word: String,
    pub score: u8,
}
fn key(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}
fn sound(text: &str) -> String {
    let mut out = String::new();
    for c in key(text).chars() {
        let c = match c {
            'c' | 'k' | 'g' | 'q' => 'k',
            's' | 'z' | 'x' => 's',
            'b' | 'f' | 'p' | 'v' => 'f',
            'd' | 't' => 't',
            'm' | 'n' => 'n',
            'l' => 'l',
            'r' => 'r',
            _ => continue,
        };
        if !out.ends_with(c) {
            out.push(c);
        }
    }
    out
}
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<_> = a.chars().collect();
    let b: Vec<_> = b.chars().collect();
    let mut rows = vec![vec![0; b.len() + 1]; a.len() + 1];
    for (i, row) in rows.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        rows[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            rows[i][j] = (rows[i - 1][j] + 1).min(rows[i][j - 1] + 1).min(rows[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]));
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                rows[i][j] = rows[i][j].min(rows[i - 2][j - 2] + 1);
            }
        }
    }
    rows[a.len()][b.len()]
}
pub fn similarity(heard: &str, word: &str) -> u8 {
    let a = key(heard);
    let b = key(word);
    if a == b {
        return 100;
    }
    if a.len() < 4 || b.len() < 4 || a.len() > 80 || b.len() > 80 {
        return 0;
    }
    let d = distance(&a, &b);
    if d > 3 {
        return 0;
    }
    let mut score = match d {
        1 if a.len().min(b.len()) >= 5 => 94,
        1 => 85,
        2 if a.len().min(b.len()) >= 6 => 83,
        2 => 72,
        3 => 65,
        _ => 0,
    };
    if !sound(&a).is_empty() && sound(&a) == sound(&b) {
        score = score.max(if d <= 2 { 92 } else { 78 });
    }
    score
}
/// Pairs are (known hearing variant, intended word); canonical words match themselves too.
pub fn suggest(heard: &str, entries: &[(&str, &str)]) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    for (alias, word) in entries {
        let score = if key(heard) == key(alias) { 100 } else { similarity(heard, word.split(',').next().unwrap_or(word)) };
        if score < 65 {
            continue;
        }
        if let Some(old) = out.iter_mut().find(|s| s.word.eq_ignore_ascii_case(word)) {
            old.score = old.score.max(score);
        } else {
            out.push(Suggestion { word: (*word).into(), score });
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.word.cmp(&b.word)));
    out.truncate(3);
    out
}
pub fn confident(scores: &[u8]) -> bool {
    scores.first().is_some_and(|score| *score >= 90 && scores.get(1).is_none_or(|next| score.saturating_sub(*next) >= 10))
}
/// Replace complete natural-language phrases only. Callers exclude commands, quotes and paths.
pub fn replace_phrase(text: &str, heard: &str, word: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let needle = heard.to_ascii_lowercase();
    if needle.is_empty() {
        return text.into();
    }
    let mut out = String::new();
    let mut start = 0;
    for (i, _) in lower.match_indices(&needle) {
        let end = i + needle.len();
        let before = text[..i].chars().next_back();
        let after = text[end..].chars().next();
        if before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphanumeric()) {
            out.push_str(&text[start..i]);
            out.push_str(word);
            start = end;
        }
    }
    out.push_str(&text[start..]);
    out
}
pub fn natural_text(text: &str) -> bool {
    !text.chars().any(|c| matches!(c, '"' | '“' | '”' | '`' | '/' | '\\' | '$' | '='))
}
pub fn valid_word(text: &str) -> bool {
    !text.trim().is_empty()
        && text.len() <= 80
        && text.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '\'' | '.' | '+' | '#' | '_' | ','))
}
pub fn add_command(text: &str) -> Option<(String, String)> {
    let text = text.trim().trim_end_matches(['.', '!', '?']);
    let lower = text.to_ascii_lowercase();
    let (start, context) = if lower.starts_with("add place ") {
        (10, "place")
    } else if lower.starts_with("add word ") {
        (9, "word")
    } else if lower.starts_with("add ") {
        (4, "word")
    } else {
        return None;
    };
    for suffix in [" to my dictionary", " to the dictionary", " to dictionary"] {
        if lower.ends_with(suffix) {
            let word = text[start..text.len() - suffix.len()].trim();
            if valid_word(word) {
                return Some((word.into(), context.into()));
            }
        }
    }
    None
}
pub fn confirmation(text: &str) -> Option<bool> {
    match crate::language::clean(text).as_str() {
        "yes" | "yes please" | "yeah" | "yep" | "correct" | "that's right" | "that is right" | "yes that's right" => Some(true),
        "no" | "nope" | "incorrect" | "that's wrong" | "that is wrong" => Some(false),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_variants_and_close_names() {
        let entries = [("Chalira", "Calera"), ("Calera", "Calera")];
        assert_eq!(suggest("Chalira", &entries)[0].score, 100);
        assert!(confident(&[similarity("Kalera", "Calera")]));
        assert!(!confident(&[94, 92]));
        assert_eq!(replace_phrase("Explain nycks and nycksville", "nycks", "Nyx"), "Explain Nyx and nycksville");
        assert!(!natural_text("run ~/nycks"));
        assert!(!valid_word("bad\nword"));
        assert_eq!(confirmation("Yes, please!"), Some(true));
        assert_eq!(confirmation("yes but delete it"), None);
        assert_eq!(add_command("Add place Calera to my dictionary."), Some(("Calera".into(), "place".into())));
    }
}
