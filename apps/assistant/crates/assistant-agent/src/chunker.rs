/// Split a streaming token buffer into speakable phrases.
///
/// Kokoro synthesizes a whole phrase before any of it plays, so the first phrase is kept
/// short (it decides when the first word is heard); later ones are longer, since they're
/// synthesized while earlier audio plays and longer phrases sound more natural.
pub struct PhraseChunker {
    buf: String,
    /// Phrases emitted so far.
    count: usize,
}

/// Words a phrase may reach before it's cut at a word boundary: the first, the second, the
/// rest.
const MAX_WORDS: [usize; 3] = [5, 10, 20];

impl PhraseChunker {
    pub fn new() -> Self {
        Self { buf: String::new(), count: 0 }
    }

    fn max_words(&self) -> usize {
        MAX_WORDS[self.count.min(MAX_WORDS.len() - 1)]
    }

    fn emit(&mut self, end: usize) -> Option<String> {
        let rest = self.buf.split_off(end);
        let out = std::mem::replace(&mut self.buf, rest).trim().to_string();
        self.buf = self.buf.trim_start().to_string();
        if out.is_empty() {
            return None;
        }
        self.count += 1;
        Some(out)
    }

    pub fn push(&mut self, token: &str) -> Option<String> {
        self.buf.push_str(token);
        if let Some(end) = find_boundary(&self.buf, self.count == 0) {
            return self.emit(end);
        }
        // Too long without a boundary: cut after the last whole word. A word still
        // arriving (no space after it yet) waits for the next phrase.
        let complete = self.buf.rfind(char::is_whitespace)?;
        if word_count(&self.buf[..complete]) >= self.max_words() {
            return self.emit(complete);
        }
        None
    }

    pub fn flush(&mut self) -> Option<String> {
        let out = self.buf.trim().to_string();
        self.buf.clear();
        if out.is_empty() {
            None
        } else {
            self.count += 1;
            Some(out)
        }
    }
}

impl Default for PhraseChunker {
    fn default() -> Self {
        Self::new()
    }
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Where the first phrase in `s` ends (the byte after its last character), if it's known
/// yet. Sentence marks count only when whitespace follows, so "3.5", "e.g." mid-word and
/// "10:30" don't split; one at the very end waits for the next token. The first phrase also
/// ends at a comma or dash after two words.
fn find_boundary(s: &str, is_first: bool) -> Option<usize> {
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|(_, n)| *n);
        let end = i + c.len_utf8();
        let followed_by_space = next.is_some_and(char::is_whitespace);
        match c {
            '.' | '?' | '!' | ';' | ':' if followed_by_space => return Some(end),
            ',' if is_first && followed_by_space && word_count(&s[..i]) >= 2 => return Some(end),
            '—' if is_first && word_count(&s[..i]) >= 2 => return Some(end),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(tokens: &[&str]) -> Vec<String> {
        let mut c = PhraseChunker::new();
        let mut out: Vec<String> = tokens.iter().filter_map(|t| c.push(t)).collect();
        out.extend(c.flush());
        out
    }

    #[test]
    fn splits_on_period() {
        let mut c = PhraseChunker::new();
        assert!(c.push("Hello").is_none());
        assert!(c.push(" there.").is_none(), "a period at the very end waits: it may be 3.5");
        assert_eq!(c.push(" How").as_deref(), Some("Hello there."));
        assert_eq!(c.flush().as_deref(), Some("How"));
    }

    #[test]
    fn the_first_phrase_ends_at_a_comma_after_two_words() {
        let phrases = all(&["Sure, I", " can", " help you", " with that."]);
        assert!(phrases[0].starts_with("Sure, I can"), "one word before the comma: no split there ({phrases:?})");
        assert_eq!(all(&["Of course,", " I can help", " you with that", "."]), ["Of course,", "I can help you with that."]);
    }

    #[test]
    fn the_first_phrase_is_short_and_later_ones_longer() {
        let words: Vec<String> = (1..=40).map(|i| format!(" w{i}")).collect();
        let tokens: Vec<&str> = words.iter().map(String::as_str).collect();
        let phrases = all(&tokens);
        let counts: Vec<usize> = phrases.iter().map(|p| word_count(p)).collect();
        assert_eq!(counts, [5, 10, 20, 5]);
    }

    #[test]
    fn never_cuts_inside_a_word() {
        // "unbelievable" arrives in two tokens right at the limit.
        let phrases = all(&["One", " two", " three", " four", " five", " unbeliev", "able", " six."]);
        assert_eq!(phrases[0], "One two three four five");
        assert!(phrases.iter().any(|p| p.contains("unbelievable")), "{phrases:?}");
    }

    #[test]
    fn numbers_and_times_stay_whole() {
        assert_eq!(all(&["It", " uses", " 3", ".", "5 GB", " now."]), ["It uses 3.5 GB now."]);
        assert_eq!(all(&["Meet", " at", " 10", ":", "30 today."]), ["Meet at 10:30 today."]);
    }
}
