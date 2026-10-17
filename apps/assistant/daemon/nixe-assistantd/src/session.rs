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
    /// Operating an app window (computer use).
    Computer,
}

/// "In Notepad, type …", "click Save in the Settings window": a place in an app plus an
/// action someone would do with the mouse or keyboard.
fn computer_request(s: &str) -> bool {
    const ACTS: &[&str] = &[
        "click", "press", "type ", "tap ", "select", "toggle", "turn on", "turn off", "switch on", "switch off", "new tab",
        "close the tab", "save", "scroll", "fill in", "enter ", "check the", "uncheck", "open the menu", "go to the end",
        "search", "write ", "find ", "open ", "close ", "play", "pause", "add ", "go to ", "delete ", "rename ", "choose ",
    ];
    let place = s.contains(" window") || s.starts_with("in ") || s.contains(" in the app") || s.starts_with("on the screen");
    place && ACTS.iter().any(|a| s.contains(a))
}
/// The tools of the Nixe apps a request names that have their own ("notepad.", "write."):
/// an app request gets them next to the computer-use tools.
pub fn app_tool_prefixes(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    let window = window_in_request(text).map(|w| w.to_lowercase()).unwrap_or_default();
    let mut out = Vec::new();
    if lower.contains("notepad") {
        out.push("notepad.");
    }
    // "write" is also a verb: only the app named as the place ("In Write, …").
    if window == "write" || window.starts_with("write ") || window == "nixe write" {
        out.push("write.");
    }
    if window == "music" || window.starts_with("music ") || window == "nixe music" {
        out.push("music.");
    }
    if lower.contains("contacts") {
        out.push("contacts.");
    }
    out
}

/// The app window a request names: "In Write, …", "In the Notepad window, …",
/// "… in VS Code". Used when the planner leaves out the window.
pub fn window_in_request(text: &str) -> Option<String> {
    let s = text.trim();
    let lower = s.to_lowercase();
    let rest = if lower.starts_with("in the ") {
        &s[7..]
    } else if lower.starts_with("in ") {
        &s[3..]
    } else {
        let i = lower.rfind(" in the ").map(|i| i + 8).or_else(|| lower.rfind(" in ").map(|i| i + 4))?;
        &s[i..]
    };
    let end = rest.find([',', '.', ':', ';']).unwrap_or(rest.len());
    let mut name = rest[..end].trim();
    for cut in [" titled ", " window", " app"] {
        if let Some(i) = name.to_lowercase().find(cut) {
            // "the Firefox window titled Bench search": the title is the more exact name.
            name = if cut == " titled " { &name[i + cut.len()..] } else { &name[..i] };
        }
    }
    let name = name.trim();
    (!name.is_empty() && name.split_whitespace().count() <= 5).then(|| name.to_string())
}

pub fn classify(text: &str) -> RequestKind {
    let s = text.to_lowercase();
    if s.starts_with("use tools to ") {
        return RequestKind::Tools;
    }
    if computer_request(&s) {
        return RequestKind::Computer;
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
    fn windows_named_in_requests() {
        assert_eq!(window_in_request("In Write, type hello").as_deref(), Some("Write"));
        assert_eq!(window_in_request("In the Notepad window, open a new tab.").as_deref(), Some("Notepad"));
        assert_eq!(window_in_request("In VS Code, go to the end").as_deref(), Some("VS Code"));
        assert_eq!(window_in_request("In the Firefox window titled Bench search, type nixe").as_deref(), Some("Bench search"));
        assert_eq!(window_in_request("press play in Spotify").as_deref(), Some("Spotify"));
        assert_eq!(window_in_request("tell me a joke"), None);
    }

    #[test]
    fn computer_requests() {
        for t in [
            "In the Notepad window, open a new tab.",
            "In Notepad, type hello and save the file.",
            "In VS Code, go to the end of the file and type Line two.",
            "In the Firefox window titled Bench search, type nixe into the search box and press the Search button.",
            "In the Spotify window, press the play/pause button.",
            "Click Save in the Settings window",
            "In the Firefox window titled Bench search, search the page for nixe.",
            "In Spotify, play the next song",
        ] {
            assert_eq!(classify(t), RequestKind::Computer, "{t}");
        }
        for t in ["what's the weather", "in the morning remind me", "click the first result", "tell me a story"] {
            assert_ne!(classify(t), RequestKind::Computer, "{t}");
        }
    }
    #[test]
    fn app_tools_for_named_apps() {
        assert_eq!(app_tool_prefixes("In Notepad, write a shopping list and save it"), ["notepad."]);
        assert_eq!(app_tool_prefixes("In the Write window, type the sentence: Hello."), ["write."]);
        assert_eq!(app_tool_prefixes("In Write, add a heading"), ["write."]);
        assert!(app_tool_prefixes("In VS Code, write a new line").is_empty());
        assert!(app_tool_prefixes("write a poem").is_empty());
        assert_eq!(app_tool_prefixes("In Music, play the album Blue"), ["music."]);
        assert_eq!(app_tool_prefixes("In Contacts, add Sam Lee"), ["contacts."]);
        assert!(app_tool_prefixes("play some music").is_empty());
    }
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
