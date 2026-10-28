/// Split a streaming token buffer into speakable phrases.
///
/// Keep sentences together so synthesis has the context needed for natural prosody.
pub struct PhraseChunker {
    buf: String,
    /// Phrases emitted so far.
    count: usize,
}

const MAX_WORDS: usize = 40;

impl PhraseChunker {
    pub fn new() -> Self {
        Self { buf: String::new(), count: 0 }
    }

    fn max_words(&self) -> usize {
        MAX_WORDS
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
        if let Some(end) = find_boundary(&self.buf) {
            return self.emit(end);
        }
        // Too long without a boundary: cut after the last whole word. A word still
        // arriving (no space after it yet) waits for the next phrase.
        let complete = self.buf.rfind(char::is_whitespace)?;
        if word_count(&self.buf[..complete]) >= self.max_words() {
            if let Some((i, c)) = self.buf[..complete].char_indices().rev()
                .find(|(i, c)| matches!(c, ',' | ';' | ':' | '—') && word_count(&self.buf[..*i]) >= 10)
            {
                return self.emit(i + c.len_utf8());
            }
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
/// "10:30" don't split; one at the very end waits for the next token.
fn find_boundary(s: &str) -> Option<usize> {
    for (i, c) in s.char_indices() {
        if !matches!(c, '.' | '?' | '!') { continue; }
        if c == '.' {
            let word = s[..=i].split_whitespace().last().unwrap_or("").trim_start_matches(|c:char| !c.is_alphanumeric());
            let lower = word.to_ascii_lowercase();
            if ["mr.","mrs.","ms.","dr.","prof.","sr.","jr.","st.","vs.","etc.","e.g.","i.e."].contains(&lower.as_str()) { continue; }
            // Initials and dotted initialisms belong with their following name/noun.
            let parts:Vec<_> = word.trim_end_matches('.').split('.').collect();
            if !parts.is_empty() && parts.iter().all(|p| p.chars().count()==1 && p.chars().all(char::is_uppercase)) { continue; }
        }
        let mut end = i + c.len_utf8();
        for ch in s[end..].chars() {
            if matches!(ch, '\"' | '\'' | '”' | '’' | ')' | ']' | '}') { end += ch.len_utf8(); } else { break; }
        }
        if s[end..].chars().next().is_some_and(char::is_whitespace) { return Some(end); }
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
    fn keeps_clauses_in_the_same_sentence() {
        let phrases = all(&["Sure, I", " can", " help you", " with that."]);
        assert!(phrases[0].starts_with("Sure, I can"), "one word before the comma: no split there ({phrases:?})");
        assert_eq!(all(&["Of course,", " I can help", " you with that", "."]), ["Of course, I can help you with that."]);
    }

    #[test]
    fn limits_very_long_unpunctuated_text() {
        let words: Vec<String> = (1..=50).map(|i| format!(" w{i}")).collect();
        let tokens: Vec<&str> = words.iter().map(String::as_str).collect();
        let phrases = all(&tokens);
        let counts: Vec<usize> = phrases.iter().map(|p| word_count(p)).collect();
        assert_eq!(counts, [40, 10]);
    }

    #[test]
    fn never_cuts_inside_a_word() {
        // "unbelievable" arrives in two tokens right at the limit.
        let phrases = all(&["One", " two", " three", " four", " five", " unbeliev", "able", " six."]);
        assert_eq!(phrases[0], "One two three four five unbelievable six.");
        assert!(phrases.iter().any(|p| p.contains("unbelievable")), "{phrases:?}");
    }

    #[test]
    fn numbers_and_times_stay_whole() {
        assert_eq!(all(&["It", " uses", " 3", ".", "5 GB", " now."]), ["It uses 3.5 GB now."]);
        assert_eq!(all(&["Meet", " at", " 10", ":", "30 today."]), ["Meet at 10:30 today."]);
    }
}

#[cfg(test)]
mod sentence_regressions {
    use super::*;
    #[test]
    fn abbreviations_quotes_and_streaming() {
        for (input, expected) in [
            ("Ask Dr. Smith tomorrow. Then call me.", vec!["Ask Dr. Smith tomorrow.","Then call me."]),
            ("The U.S. office is open. Come in.", vec!["The U.S. office is open.","Come in."]),
            ("She said \"hello.\" Then left.", vec!["She said \"hello.\"","Then left."]),
        ] {
            let mut c=PhraseChunker::new(); let mut out=Vec::new();
            for ch in input.chars() { if let Some(p)=c.push(&ch.to_string()) {out.push(p);} }
            out.extend(c.flush());assert_eq!(out,expected);
        }
    }
}
