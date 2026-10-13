//! Hands-free wake: strip a configured name/phrase from the start of a transcript.

pub fn strip_wake(
    text: &str,
    name: &str,
    prefixes: &[String],
    phrases: &[String],
) -> Option<String> {
    let words = tokenize(text);
    if words.is_empty() {
        return None;
    }
    let prefixes: Vec<Vec<String>> = prefixes
        .iter()
        .map(|p| tokenize(p))
        .filter(|t| !t.is_empty())
        .collect();
    let mut names: Vec<Vec<String>> = phrases
        .iter()
        .map(|p| tokenize(p))
        .filter(|t| !t.is_empty())
        .collect();
    let n_tok = tokenize(name);
    if !n_tok.is_empty() && !names.iter().any(|p| p == &n_tok) {
        names.insert(0, n_tok);
    }

    let mut i = 0usize;
    let mut found = false;
    while i < words.len() {
        // First check prefixes (e.g. "hey", "ok", "a", "ay", "hey there")
        let mut matched_prefix_len = None;
        for p in &prefixes {
            if words[i..].starts_with(p) {
                matched_prefix_len = Some(p.len());
                break;
            }
        }
        if let Some(len) = matched_prefix_len {
            i += len;
            continue;
        }

        // Then check persona names/aliases (including multi-word phrases like "a echo", "site for")
        let mut matched_name_len = None;
        for n in &names {
            if words[i..].starts_with(n) {
                matched_name_len = Some(n.len());
                break;
            }
        }
        if let Some(len) = matched_name_len {
            found = true;
            i += len;
            // Skip single-letter trailing artifact (e.g. "echo a")
            if i < words.len() && words[i].len() == 1 && words[i].chars().all(|c| c.is_alphabetic())
            {
                i += 1;
            }
            continue;
        }
        break;
    }
    if !found {
        return None;
    }
    Some(words[i..].join(" "))
}

fn tokenize(raw: &str) -> Vec<String> {
    let alnum: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(' ')
            } else {
                ' '
            }
        })
        .collect();
    alnum.split_whitespace().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::strip_wake;

    fn phrases() -> Vec<String> {
        [
            "echo", "ekko", "ecko", "eco", "eko", "hecho", "a echo", "ay echo", "site for",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
    }

    fn prefixes() -> Vec<String> {
        ["hey", "ok", "okay", "a", "ay", "hi"]
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }

    fn rest(text: &str) -> Option<String> {
        strip_wake(text, "Echo", &prefixes(), &phrases())
    }

    #[test]
    fn strips_name_and_keeps_the_rest() {
        assert_eq!(rest("hey echo, open files").as_deref(), Some("open files"));
        assert_eq!(rest("Echo").as_deref(), Some(""));
        assert_eq!(rest("ok echo lock").as_deref(), Some("lock"));
        assert_eq!(rest("Hey Ekko.").as_deref(), Some(""));
        assert_eq!(
            rest("Eco what time is it").as_deref(),
            Some("what time is it")
        );
        assert_eq!(
            rest("Echo tell me a joke").as_deref(),
            Some("tell me a joke")
        );
        assert_eq!(rest("hey okay echo lock").as_deref(), Some("lock"));
        assert_eq!(rest("ok eko volume up").as_deref(), Some("volume up"));
        assert_eq!(rest("a echo lock screen").as_deref(), Some("lock screen"));
        assert_eq!(rest("ay echo lock").as_deref(), Some("lock"));
        assert_eq!(rest("site for what is up").as_deref(), Some("what is up"));
    }

    #[test]
    fn ignores_speech_without_the_name() {
        assert_eq!(rest("open files"), None);
        assert_eq!(rest("okay"), None);
        assert_eq!(rest("now"), None);
        assert_eq!(rest("hey"), None);
        assert_eq!(rest("hey ok"), None);
    }
}
