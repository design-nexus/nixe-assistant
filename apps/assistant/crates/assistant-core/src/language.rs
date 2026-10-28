//! Deterministic speech controls and explicit spelling, never blanket transcript rewriting.
pub fn clean(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn silent_stop(text: &str) -> bool {
    if text.trim().starts_with(['"', '“']) {
        return false;
    }
    let text = clean(text);
    let text = ["would you ", "could you ", "can you ", "will you "]
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix))
        .unwrap_or(&text);
    let words: Vec<_> =
        text.split_whitespace().filter(|w| !["please", "just", "now", "hey", "nyx", "thanks"].contains(w)).collect();
    matches!(
        words.join(" ").as_str(),
        "stop"
            | "stop talking"
            | "stop speaking"
            | "shut up"
            | "be quiet"
            | "quiet"
            | "enough"
            | "that's enough"
            | "cancel"
            | "never mind"
            | "nevermind"
    )
}
/// Accept actual separated letters, not a model's guess about a word.
pub fn letters(text: &str) -> Option<String> {
    let text = text.trim().trim_end_matches(['.', '!', '?']);
    let words: Vec<_> =
        text.split(|c: char| c.is_whitespace() || matches!(c, '-' | ',' | '.')).filter(|w| !w.is_empty()).collect();
    (words.len() >= 2 && words.len() <= 40 && words.iter().all(|w| w.len() == 1 && w.as_bytes()[0].is_ascii_alphabetic()))
        .then(|| words.concat().to_uppercase())
}
pub fn spelling_correction(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for marker in ["it's spelled ", "it is spelled ", "it's spelt ", "spelled ", "spelling is ", "no, it's ", "no it's ", "it's ", "it is "] {
        if let Some(i) = lower.find(marker) {
            return letters(&text[i + marker.len()..]);
        }
    }
    for prefix in ["no, it's ", "no it's ", "it's ", "it is "] {
        if let Some(tail)=lower.strip_prefix(prefix) {
            if let Some(word)=letters(tail) { return Some(word); }
        }
    }
    // An explicitly supplied trailing letter sequence: "No, Calera — C A L E R A".
    for (index, c) in text.char_indices().rev() {
        if matches!(c, ':' | '—') {
            if let Some(word) = letters(&text[index + c.len_utf8()..]) {
                return Some(word);
            }
        }
    }
    letters(text)
}
/// Strict explicit X -> Y correction; no learning from arbitrary conversation.
pub fn word_correction(text: &str) -> Option<(String, String)> {
    let lower = text.to_ascii_lowercase();
    let valid = |s: &str| !s.is_empty() && s.len() <= 80 && s.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\''));
    for prefix in ["i said ", "it's ", "it is ", "no, i meant ", "i meant "] {
        if let Some(tail) = lower.strip_prefix(prefix) {
            if let Some((right, wrong)) = tail.split_once(", not ") {
                let wrong = wrong.trim_end_matches(['.', '!', '?']).trim();
                if valid(wrong) && valid(right) && wrong != right {
                    return Some((wrong.into(), right.into()));
                }
            }
        }
    }
    let tail = lower.strip_prefix("not ").or_else(|| lower.strip_prefix("no, not "))?;
    let (wrong, right) = tail.split_once(", it's ").or_else(|| tail.split_once(", it is "))?;
    let valid = |s: &str| !s.is_empty() && s.len() <= 80 && s.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\''));
    let right = right.trim_end_matches(['.', '!', '?']).trim();
    (valid(wrong) && valid(right) && wrong != right).then(|| (wrong.into(), right.into()))
}
pub fn map_place(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if !["show", "display", "open", "map", "can you show", "could you show", "would you show", "please show"]
        .iter()
        .any(|w| lower.starts_with(w))
    {
        return None;
    }
    for marker in ["map of ", "map for ", "map in "] {
        if let Some(i) = lower.find(marker) {
            let place = text[i + marker.len()..].trim().trim_end_matches(['.', '!', '?']).trim_end_matches(" please");
            if !place.is_empty() && place.len() <= 256 {
                return Some(place.into());
            }
        }
    }
    None
}
/// A named-place correction has a known subject supplied by the previous request.
pub fn corrected_place(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for prefix in ["no, it's ", "no it's ", "no, it is ", "i meant ", "i said ", "it's ", "it is "] {
        if let Some(tail) = lower.strip_prefix(prefix) {
            let value = tail.trim_end_matches(['.', '!', '?']);
            if value.len() <= 128
                && !value.is_empty()
                && value.split_whitespace().count() <= 5
                && value.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | ',' | '\''))
            {
                return Some(value.into());
            }
        }
    }
    None
}
pub fn spelling_request(text: &str, corrected: Option<&str>) -> Option<String> {
    let lower = clean(text);
    let tail = lower
        .strip_prefix("spell ")
        .or_else(|| lower.strip_prefix("can you spell "))
        .or_else(|| lower.strip_prefix("please spell "))?;
    let tail = tail
        .strip_prefix("the word ")
        .unwrap_or(tail)
        .trim_end_matches(" back to me")
        .trim_end_matches(" for me")
        .trim_end_matches(" please");
    if matches!(tail, "it" | "that" | "that word" | "the word") {
        return corrected.map(str::to_string);
    }
    (tail.len() <= 40 && !tail.is_empty() && tail.chars().all(|c| c.is_ascii_alphabetic())).then(|| tail.to_uppercase())
}
pub fn spelled_display(word: &str) -> String {
    word.to_uppercase().chars().map(|c| c.to_string()).collect::<Vec<_>>().join("-")
}
/// Only isolated spelling runs are converted; ordinary prose and acronyms stay intact.
pub fn letter_name(letter: u8) -> &'static str {
    const NAMES: [&str; 26] = [
        "ay",
        "bee",
        "see",
        "dee",
        "ee",
        "eff",
        "jee",
        "aitch",
        "eye",
        "jay",
        "kay",
        "ell",
        "em",
        "en",
        "oh",
        "pee",
        "cue",
        "ar",
        "ess",
        "tee",
        "you",
        "vee",
        "double you",
        "ex",
        "why",
        "zee",
    ];
    NAMES[(letter.to_ascii_uppercase() - b'A') as usize]
}
pub fn for_speech(text: &str) -> String {
    text.split_whitespace()
        .map(|token| {
            let bare = token.trim_matches(|c: char| matches!(c, '.' | ',' | ':' | ';' | '!' | '?' | '"'));
            if bare.contains('-') {
                if let Some(word) = letters(bare) {
                    return word.bytes().map(|c| letter_name(c)).collect::<Vec<_>>().join(". ") + ".";
                }
            }
            token.into()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_and_spelling() {
        for s in ["shut up", "Please stop talking!", "Nyx, just be quiet now."] {
            assert!(silent_stop(s));
        }
        for s in ["stop the service", "say stop talking", "don't stop", "the word stop"] {
            assert!(!silent_stop(s));
        }
        assert_eq!(spelling_correction("No, it's spelled C A L E R A."), Some("CALERA".into()));
        assert_eq!(letters("regular words"), None);
        assert_eq!(spoken_letters("see a ell ee are ay"),Some("CALERA".into()));
        assert_eq!(spoken_letters("show me a map"),None);
        assert_eq!(place_answer("Calera, Alabama."),Some("Calera, Alabama".into()));
        assert_eq!(place_answer("open terminal"),None);
        assert_eq!(map_place("Can you show me a map of Calera, AL?"),Some("Calera, AL".into()));
        assert!(!silent_stop("\"stop talking\""));
        assert!(silent_stop("Could you please stop talking?"));
        assert_eq!(spelling_correction("No, Calera — C A L E R A"), Some("CALERA".into()));
        assert_eq!(word_correction("I said Calera, not camera."), Some(("camera".into(), "calera".into())));
        assert_eq!(for_speech("C-A-L-E-R-A"), "see. ay. ell. ee. ar. ay.");
        assert_eq!(for_speech("NASA is well-known."), "NASA is well-known.");
        assert_eq!(word_correction("Not camera, it's Calera."), Some(("camera".into(), "calera".into())));
    }
}

/// Spoken letter names are accepted only while a spelling answer is pending.
pub fn spoken_letters(text: &str) -> Option<String> {
    let cleaned = clean(text);
    let mut out = String::new();
    for word in cleaned.split_whitespace() {
        let c = match word {
            "ay" | "aye" => 'A',
            "bee" | "be" => 'B',
            "see" | "sea" => 'C',
            "dee" => 'D',
            "ee" => 'E',
            "eff" => 'F',
            "gee" | "jee" => 'G',
            "aitch" | "h" => 'H',
            "eye" => 'I',
            "jay" => 'J',
            "kay" => 'K',
            "ell" => 'L',
            "em" => 'M',
            "en" => 'N',
            "oh" => 'O',
            "pee" => 'P',
            "cue" | "queue" => 'Q',
            "ar" | "are" => 'R',
            "ess" => 'S',
            "tee" => 'T',
            "you" => 'U',
            "vee" => 'V',
            "ex" => 'X',
            "why" => 'Y',
            "zee" | "zed" => 'Z',
            _ if word.len() == 1 && word.as_bytes()[0].is_ascii_alphabetic() => word.as_bytes()[0].to_ascii_uppercase() as char,
            _ => return None,
        };
        out.push(c);
    }
    (out.len() >= 2 && out.len() <= 40).then_some(out)
}
pub fn place_answer(text: &str) -> Option<String> {
    let lower = clean(text);
    if lower.split_whitespace().any(|w| {
        ["stop", "cancel", "no", "yes", "you", "what", "why", "how", "show", "open", "run", "tell", "forget", "call", "spell"]
            .contains(&w)
    }) {
        return None;
    }
    let value = text.trim().trim_end_matches(['.', '!', '?']);
    (!value.is_empty()
        && value.len() <= 128
        && value.split_whitespace().count() <= 4
        && value.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | ',' | '-' | '\'')))
    .then(|| value.into())
}

pub fn map_question(text: &str) -> bool {
    if map_place(text).is_some() { return false; }
    let text=clean(text);
    let map=text.split_whitespace().any(|w| matches!(w,"map"|"maps"));
    map && ["can you ","could you ","are you able ","why can't you ","why can t you ","show me a map","show me maps","display a map","open a map"].iter().any(|p| text.starts_with(p))
}
