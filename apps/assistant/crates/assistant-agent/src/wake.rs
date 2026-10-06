//! Hands-free wake: strip a configured name/phrase from the start of a transcript.

pub fn strip_wake(text: &str, name: &str, prefixes: &[String], phrases: &[String]) -> Option<String> {
    let words = tokenize(text);
    if words.is_empty() {
        return None;
    }
    let prefixes: Vec<&str> = prefixes.iter().map(String::as_str).collect();
    let mut names: Vec<String> = phrases.to_vec();
    let n = name.trim().to_lowercase();
    if !n.is_empty() && !names.iter().any(|p| p == &n) {
        names.insert(0, n);
    }
    let mut i = 0usize;
    let mut found = false;
    while i < words.len() {
        let w = words[i].as_str();
        if prefixes.contains(&w) {
            i += 1;
            continue;
        }
        if names.iter().any(|p| p == w) {
            found = true;
            i += 1;
            if i < words.len()
                && words[i].len() == 1
                && words[i].chars().all(|c| c.is_alphabetic())
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
        ["nyx", "nicks", "nick", "niks", "next", "six"]
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }

    fn prefixes() -> Vec<String> {
        ["hey", "ok", "okay"].iter().map(|s| (*s).to_string()).collect()
    }

    fn rest(text: &str) -> Option<String> {
        strip_wake(text, "Nyx", &prefixes(), &phrases())
    }

    #[test]
    fn strips_name_and_keeps_the_rest() {
        assert_eq!(rest("hey nyx, open files").as_deref(), Some("open files"));
        assert_eq!(rest("Nyx").as_deref(), Some(""));
        assert_eq!(rest("ok nyx lock").as_deref(), Some("lock"));
        assert_eq!(rest("Hey Nicks.").as_deref(), Some(""));
        assert_eq!(rest("Hey Nick's").as_deref(), Some(""));
        assert_eq!(rest("Next.").as_deref(), Some(""));
        assert_eq!(rest("Next what time is it").as_deref(), Some("what time is it"));
        assert_eq!(rest("Six open files").as_deref(), Some("open files"));
        assert_eq!(rest("Nyx tell me a joke").as_deref(), Some("tell me a joke"));
        assert_eq!(rest("hey okay nyx lock").as_deref(), Some("lock"));
        assert_eq!(rest("ok niks volume up").as_deref(), Some("volume up"));
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
