/// Split a streaming token buffer into speakable phrases.
pub struct PhraseChunker {
    buf: String,
    max_words: usize,
    is_first: bool,
}

impl PhraseChunker {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            max_words: 12,
            is_first: true,
        }
    }

    pub fn push(&mut self, token: &str) -> Option<String> {
        self.buf.push_str(token);
        if let Some(idx) = find_boundary(&self.buf, self.is_first) {
            let (head, rest) = self.buf.split_at(idx + 1);
            let out = head.trim().to_string();
            self.buf = rest.to_string();
            if out.is_empty() {
                return None;
            }
            self.is_first = false;
            return Some(out);
        }
        if word_count(&self.buf) >= self.max_words {
            let out = self.buf.trim().to_string();
            self.buf.clear();
            if out.is_empty() {
                return None;
            }
            self.is_first = false;
            return Some(out);
        }
        None
    }

    pub fn flush(&mut self) -> Option<String> {
        let out = self.buf.trim().to_string();
        self.buf.clear();
        if out.is_empty() {
            None
        } else {
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

fn find_boundary(s: &str, is_first: bool) -> Option<usize> {
    let words = word_count(s);
    s.char_indices()
        .find(|(_, c)| {
            if matches!(c, '.' | '?' | '!' | ';' | ':') {
                true
            } else {
                is_first && words >= 3 && matches!(c, ',' | '—')
            }
        })
        .map(|(i, c)| i + c.len_utf8() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_period() {
        let mut c = PhraseChunker::new();
        assert!(c.push("Hello").is_none());
        let got = c.push(" there.");
        assert_eq!(got.as_deref(), Some("Hello there."));
    }

    #[test]
    fn splits_first_chunk_on_comma_after_three_words() {
        let mut c = PhraseChunker::new();
        assert!(c.push("Sure, I").is_none());
        assert_eq!(c.push(" can help").as_deref(), Some("Sure,"));
        assert_eq!(
            c.push(" you with that.").as_deref(),
            Some("I can help you with that.")
        );
        assert_eq!(c.flush().as_deref(), None);
    }
}
