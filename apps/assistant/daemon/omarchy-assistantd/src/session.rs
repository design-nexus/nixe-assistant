use serde_json::{json, Value};
use std::time::{Duration, Instant};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Wake,
    FollowUp,
    Conversation,
    Dictation,
    Paused,
}
impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Wake => "wake",
            Self::FollowUp => "follow_up",
            Self::Conversation => "conversation",
            Self::Dictation => "dictation",
            Self::Paused => "dictation_paused",
        }
    }
}
pub struct Session {
    pub id: u64,
    pub mode: Mode,
    pub deadline: Instant,
    pub target: Value,
    pub draft: String,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            id: 0,
            mode: Mode::Wake,
            deadline: Instant::now(),
            target: Value::Null,
            draft: String::new(),
        }
    }
}
impl Session {
    pub fn begin(&mut self, mode: Mode, secs: u64) {
        self.id += 1;
        self.mode = mode;
        self.deadline = Instant::now() + Duration::from_secs(secs);
    }
    pub fn stop(&mut self) {
        self.id += 1;
        self.mode = Mode::Wake;
        self.deadline = Instant::now();
        if self.draft.is_empty() {
            self.target = Value::Null;
        }
    }
    pub fn after_reply(&mut self, enabled: bool, follow: u64, idle: u64) {
        match self.mode {
            Mode::Conversation => self.deadline = Instant::now() + Duration::from_secs(idle),
            Mode::Wake | Mode::FollowUp if enabled && follow > 0 => {
                self.begin(Mode::FollowUp, follow)
            }
            _ => {}
        }
    }
    pub fn remaining(&self) -> u64 {
        self.deadline
            .saturating_duration_since(Instant::now())
            .as_secs()
    }
    pub fn json(&self) -> Value {
        json!({"session_id":self.id,"mode":self.mode.name(),"remaining_secs":self.remaining(),"target":self.target,"draft":self.draft})
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum RequestKind {
    Chat,
    Lookup,
    Browse,
    Terminal,
    Tools,
}
pub fn classify(text: &str) -> RequestKind {
    let s = text.to_lowercase();
    if s.starts_with("use tools to ") {
        return RequestKind::Tools;
    }
    if ["open ", "go to ", "navigate to "]
        .iter()
        .any(|p| s.starts_with(p))
        && [".com", ".org", ".net", ".io", ".dev", ".gov", ".edu"]
            .iter()
            .any(|d| s.contains(d))
    {
        return RequestKind::Browse;
    }
    if [
        "read the file ",
        "read file ",
        "git status",
        "check git status",
    ]
    .iter()
    .any(|p| s.starts_with(p))
    {
        return RequestKind::Tools;
    }
    if [
        "google ",
        "google this",
        "find websites",
        "find web pages",
        "show me pages",
        "show me websites",
        "open that source",
        "open the source",
        "open the first result",
        "browse ",
        "open http",
        "go to http",
        "open a web",
        "open the browser",
        "search google",
        "on this page",
        "on that page",
        "fill out",
        "click ",
    ]
    .iter()
    .any(|x| s.contains(x))
    {
        return RequestKind::Browse;
    }
    if [
        "run command",
        "run the command",
        "run this command",
        "run in terminal",
        "in the terminal",
        "open terminal",
        "start terminal",
    ]
    .iter()
    .any(|x| s.contains(x))
    {
        return RequestKind::Terminal;
    }
    if [
        "near me",
        "nearby",
        "around here",
        "local news",
        "weather",
        "forecast",
        "news",
        "latest",
        "up to date",
        "current price",
        "stock price",
        "exchange rate",
        "today's",
        "todays",
        "this week's",
        "who is the current",
        "search the web",
        "look up",
        "look it up",
        "search online",
    ]
    .iter()
    .any(|x| s.contains(x))
    {
        return RequestKind::Lookup;
    }
    RequestKind::Chat
}
pub fn dictation_control(text: &str) -> Option<&'static str> {
    match text
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .to_lowercase()
        .as_str()
    {
        "stop dictating" | "end dictation" => Some("stop"),
        "pause dictation" => Some("pause"),
        "resume dictation" => Some("resume"),
        _ => None,
    }
}
pub fn punctuation(text: &str) -> String {
    match text.trim().trim_end_matches('.').to_lowercase().as_str() {
        "new line" => "\n".into(),
        "new paragraph" => "\n\n".into(),
        "comma" => ",".into(),
        "period" | "full stop" => ".".into(),
        "question mark" => "?".into(),
        "exclamation mark" => "!".into(),
        _ => format!("{} ", text.trim()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_expiry_and_controls() {
        let mut s = Session::default();
        s.after_reply(true, 10, 60);
        assert_eq!(s.mode, Mode::FollowUp);
        s.begin(Mode::Conversation, 60);
        s.after_reply(true, 10, 60);
        assert_eq!(s.mode, Mode::Conversation);
        s.stop();
        assert_eq!(s.mode, Mode::Wake);
        assert_eq!(dictation_control("Stop dictating."), Some("stop"));
        assert_eq!(
            dictation_control("write stop dictating in the document"),
            None
        );
        assert_eq!(punctuation("new paragraph"), "\n\n");
    }
    #[test]
    fn factual_vs_browser() {
        assert_eq!(
            classify("What's the weather tomorrow?"),
            RequestKind::Lookup
        );
        assert_eq!(classify("Google weather pages"), RequestKind::Browse);
        assert_eq!(classify("show me websites about Rust"), RequestKind::Browse);
        assert_eq!(classify("What is two plus two?"), RequestKind::Chat);
    }
}
