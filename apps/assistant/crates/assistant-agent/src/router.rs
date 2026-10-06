/// Fast path for high-confidence desktop commands. Anything ambiguous stays Chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Chat,
    Forget,
    /// Spoken reply only; do not run a desktop command.
    Say(&'static str),
    Desktop {
        tool: &'static str,
        spoken: &'static str,
        omarchy: &'static [&'static str],
    },
}

const CAN_DO: &str = "I didn't catch a command I can run. I can lock, close a window, change volume or brightness, switch workspaces 1 to 5, cycle wallpaper, toggle the bar, gaps, or night light, take a screenshot, or open the browser, terminal, files, Spotify, or Signal.";

pub fn route(text: &str) -> Intent {
    let key = preprocess(text);
    if key.is_empty() {
        return Intent::Chat;
    }
    if FORGET.iter().any(|p| *p == key) {
        return Intent::Forget;
    }
    if MUTE_MIC.iter().any(|p| *p == key) {
        return Intent::Say("I need the microphone, so I can't mute it.");
    }
    if let Some(hit) = exact_command(&key) {
        return hit;
    }
    if let Some(n) = workspace_num(&key) {
        return workspace_intent(n);
    }
    if let Some(hit) = fuzzy_command(&key) {
        return hit;
    }
    if looks_like_command(&key) {
        return Intent::Say(CAN_DO);
    }
    Intent::Chat
}

fn exact_command(key: &str) -> Option<Intent> {
    for (phrases, tool, spoken, omarchy) in TABLE {
        if phrases.iter().any(|p| *p == key) {
            return Some(Intent::Desktop {
                tool,
                spoken,
                omarchy,
            });
        }
    }
    None
}

fn fuzzy_command(key: &str) -> Option<Intent> {
    let mut best: Option<Intent> = None;
    let mut best_d = usize::MAX;
    for (phrases, tool, spoken, omarchy) in TABLE {
        for phrase in *phrases {
            let d = phrase_distance(key, phrase);
            if d > 0 && d <= max_edits(phrase, key) && d < best_d {
                best_d = d;
                best = Some(Intent::Desktop {
                    tool,
                    spoken,
                    omarchy,
                });
            }
        }
    }
    best
}

fn max_edits(phrase: &str, key: &str) -> usize {
    let n = phrase.len().min(key.len());
    if n < 6 {
        0
    } else if n < 12 {
        1
    } else {
        2
    }
}

fn phrase_distance(a: &str, b: &str) -> usize {
    let aw: Vec<&str> = a.split_whitespace().collect();
    let bw: Vec<&str> = b.split_whitespace().collect();
    let whole = levenshtein(a, b);
    if aw.len() == bw.len() && !aw.is_empty() {
        let words: usize = aw.iter().zip(&bw).map(|(x, y)| levenshtein(x, y)).sum();
        return words.min(whole);
    }
    whole
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn workspace_intent(n: u8) -> Intent {
    let i = (n - 1) as usize;
    Intent::Desktop {
        tool: "hypr.workspace",
        spoken: WORKSPACE_SPOKEN[i],
        omarchy: WORKSPACES[i],
    }
}

fn workspace_num(key: &str) -> Option<u8> {
    let words: Vec<&str> = key.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        if !looks_like_workspace(w) {
            continue;
        }
        if let Some(next) = words.get(i + 1).copied() {
            if let Some(n) = parse_ws_num(next) {
                return Some(n);
            }
        }
        if i > 0 {
            if let Some(n) = parse_ws_num(words[i - 1]) {
                return Some(n);
            }
        }
        for tok in words.iter().rev() {
            if let Some(n) = parse_ws_num(tok) {
                return Some(n);
            }
        }
        return None;
    }
    let Some((last, rest)) = words.split_last() else {
        return None;
    };
    let n = parse_ws_num(last)?;
    if rest
        .iter()
        .any(|w| *w == "go" || *w == "going" || *w == "switch" || *w == "change")
    {
        return Some(n);
    }
    None
}

const NUM_WORDS: &[(&str, u8)] = &[
    ("one", 1),
    ("won", 1),
    ("wan", 1),
    ("wun", 1),
    ("first", 1),
    ("two", 2),
    ("too", 2),
    ("second", 2),
    ("three", 3),
    ("tree", 3),
    ("third", 3),
    ("four", 4),
    ("for", 4),
    ("fore", 4),
    ("fourth", 4),
    ("five", 5),
    ("fife", 5),
    ("fifth", 5),
];

fn parse_ws_num(rest: &str) -> Option<u8> {
    let rest = rest.strip_prefix("number ").unwrap_or(rest).trim();
    if rest.is_empty() {
        return None;
    }
    if let Ok(n) = rest.parse::<u8>() {
        return (1..=5).contains(&n).then_some(n);
    }
    for (word, n) in NUM_WORDS {
        if rest == *word {
            return Some(*n);
        }
    }
    if rest.len() < 3 {
        return None;
    }
    let mut best = None;
    let mut best_d = usize::MAX;
    for (word, n) in NUM_WORDS {
        let d = levenshtein(rest, word);
        if d == 1 && d < best_d {
            best_d = d;
            best = Some(*n);
        }
    }
    best
}

fn looks_like_workspace(tok: &str) -> bool {
    tok == "workspace"
        || tok == "workspaces"
        || tok == "space"
        || tok == "spaces"
        || tok.contains("space")
        || tok.contains("spice")
        || levenshtein(tok, "workspace") <= 4
}

fn looks_like_command(key: &str) -> bool {
    const STARTS: &[&str] = &[
        "go ",
        "going ",
        "switch ",
        "make ",
        "open ",
        "launch ",
        "turn ",
        "mute",
        "unmute",
        "lock",
        "take ",
        "toggle ",
        "change ",
        "dim",
        "brighten ",
        "raise ",
        "lower ",
        "increase ",
        "decrease ",
        "quieter",
        "louder",
        "screenshot",
        "workspace",
        "brighter",
        "dimmer",
        "stay ",
        "silence ",
        "hide ",
        "show ",
        "close ",
        "spotify",
        "signal",
        "fullscreen",
        "gaps",
        "keyboard",
        "touchpad",
        "discord",
        "screensaver",
    ];
    STARTS.iter().any(|p| key == p.trim() || key.starts_with(p))
        || key.split_whitespace().any(looks_like_workspace)
}

fn preprocess(raw: &str) -> String {
    let mut s = normalize(raw);
    s = split_trailing_digits(&s);
    for (from, to) in [
        ("our space", "workspace"),
        ("work space", "workspace"),
        ("arc spice", "workspace"),
        ("scream", "screen"),
        ("screens", "screen"),
    ] {
        s = s.replace(from, to);
    }
    s.split_whitespace()
        .map(correct_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn correct_token(w: &str) -> String {
    if w == "mike" {
        return "make".into();
    }
    if looks_like_workspace(w) {
        return "workspace".into();
    }
    match w {
        "one" | "won" | "wan" | "wun" | "first" => return "1".into(),
        "two" | "second" => return "2".into(),
        "three" | "tree" | "third" => return "3".into(),
        "four" | "fourth" => return "4".into(),
        "five" | "fife" | "fifth" => return "5".into(),
        _ => {}
    }
    snap_vocab(w).to_string()
}

const VOCAB: &[&str] = &[
    "workspace",
    "screen",
    "brighter",
    "dimmer",
    "brightness",
    "volume",
    "lock",
    "browser",
    "chrome",
    "terminal",
    "wallpaper",
    "background",
    "files",
    "file",
    "manager",
    "nautilus",
    "bluetooth",
    "mute",
    "unmute",
    "open",
    "launch",
    "switch",
    "change",
    "make",
    "turn",
    "toggle",
    "night",
    "light",
    "nightlight",
    "screenshot",
    "capture",
    "silence",
    "notifications",
    "awake",
    "sleep",
    "stay",
    "quieter",
    "louder",
    "raise",
    "lower",
    "increase",
    "decrease",
    "brighten",
    "bar",
    "gaps",
    "animations",
    "transparency",
    "fullscreen",
    "layout",
    "spotify",
    "signal",
    "region",
    "about",
    "panel",
    "touchpad",
    "keyboard",
    "window",
    "close",
    "theme",
    "discord",
    "screensaver",
    "output",
    "backlight",
    "speakers",
    "headphones",
];

fn snap_vocab(w: &str) -> &str {
    if w.len() < 4 {
        return w;
    }
    if VOCAB.iter().any(|v| *v == w) {
        return w;
    }
    let mut best = w;
    let mut best_d = usize::MAX;
    for v in VOCAB {
        if v.len() < 4 {
            continue;
        }
        let d = levenshtein(w, v);
        let max = if w.len().min(v.len()) < 6 { 1 } else { 2 };
        if d > 0 && d <= max && d < best_d {
            best_d = d;
            best = v;
        }
    }
    best
}

fn split_trailing_digits(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let n = w.bytes().rev().take_while(u8::is_ascii_digit).count();
            if n > 0 && n < w.len() {
                let (alpha, num) = w.split_at(w.len() - n);
                format!("{alpha} {num}")
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

const WORKSPACE_SPOKEN: [&str; 5] = [
    "Workspace 1.",
    "Workspace 2.",
    "Workspace 3.",
    "Workspace 4.",
    "Workspace 5.",
];

const WORKSPACES: [&[&str]; 5] = [
    &["hyprctl", "eval", r#"hl.dispatch(hl.dsp.focus({ workspace = "1" }))"#],
    &["hyprctl", "eval", r#"hl.dispatch(hl.dsp.focus({ workspace = "2" }))"#],
    &["hyprctl", "eval", r#"hl.dispatch(hl.dsp.focus({ workspace = "3" }))"#],
    &["hyprctl", "eval", r#"hl.dispatch(hl.dsp.focus({ workspace = "4" }))"#],
    &["hyprctl", "eval", r#"hl.dispatch(hl.dsp.focus({ workspace = "5" }))"#],
];

const MUTE_MIC: &[&str] = &[
    "mute mic",
    "mute the mic",
    "mute microphone",
    "mute the microphone",
    "unmute mic",
    "unmute the mic",
    "unmute microphone",
    "unmute the microphone",
];

const FORGET: &[&str] = &[
    "forget that",
    "forget it",
    "forget everything",
    "start over",
    "clear the conversation",
    "new conversation",
];

const TABLE: &[(&[&str], &str, &str, &[&str])] = &[
    (
        &[
            "lock",
            "lock screen",
            "lock the screen",
            "lock session",
            "lock the session",
            "lock computer",
            "lock the computer",
            "lock machine",
            "lock the machine",
        ],
        "system.lock",
        "Locking.",
        &["system", "lock"],
    ),
    (
        &[
            "volume down",
            "turn down the volume",
            "turn the volume down",
            "lower the volume",
            "quieter",
            "turn it down",
        ],
        "audio.volume_down",
        "Volume down.",
        &["audio", "output", "volume", "lower"],
    ),
    (
        &[
            "volume up",
            "turn up the volume",
            "turn the volume up",
            "raise the volume",
            "louder",
            "turn it up",
        ],
        "audio.volume_up",
        "Volume up.",
        &["audio", "output", "volume", "raise"],
    ),
    (
        &[
            "mute",
            "mute audio",
            "mute the volume",
            "mute sound",
            "unmute",
            "unmute audio",
            "toggle mute",
        ],
        "audio.mute",
        "Toggling mute.",
        &["audio", "output", "volume", "mute-toggle"],
    ),
    (
        &[
            "open browser",
            "launch browser",
            "start browser",
            "open the browser",
            "launch the browser",
            "open chrome",
            "launch chrome",
        ],
        "app.launch",
        "Opening the browser.",
        &["launch", "browser"],
    ),
    (
        &[
            "open terminal",
            "launch terminal",
            "start terminal",
            "open a terminal",
            "open the terminal",
        ],
        "app.launch",
        "Opening a terminal.",
        &["launch", "terminal"],
    ),
    (
        &[
            "screenshot",
            "take a screenshot",
            "take screenshot",
            "capture screen",
            "capture the screen",
            "screenshot the screen",
        ],
        "capture.screenshot",
        "Capturing.",
        &["capture", "screenshot"],
    ),
    (
        &[
            "night light",
            "nightlight",
            "toggle night light",
            "toggle nightlight",
            "night mode",
        ],
        "toggle.nightlight",
        "Toggling night light.",
        &["toggle", "nightlight"],
    ),
    (
        &[
            "brighter",
            "brightness up",
            "increase brightness",
            "turn brightness up",
            "raise the brightness",
            "make it brighter",
            "make the screen brighter",
            "make screen brighter",
            "brighten the screen",
        ],
        "display.brightness_up",
        "Brightness up.",
        &["brightness", "display", "+10%"],
    ),
    (
        &[
            "dimmer",
            "brightness down",
            "decrease brightness",
            "turn brightness down",
            "lower the brightness",
            "make it dimmer",
            "make the screen dimmer",
            "make screen dimmer",
            "dim the screen",
        ],
        "display.brightness_down",
        "Brightness down.",
        &["brightness", "display", "10%-"],
    ),
    (
        &[
            "next wallpaper",
            "next background",
            "change wallpaper",
            "change the wallpaper",
            "change background",
        ],
        "theme.bg_next",
        "Next wallpaper.",
        &["theme", "bg", "next"],
    ),
    (
        &[
            "open files",
            "open file manager",
            "open the file manager",
            "open nautilus",
        ],
        "app.launch",
        "Opening files.",
        &["launch", "nautilus"],
    ),
    (
        &[
            "toggle bluetooth",
            "bluetooth",
            "turn bluetooth on",
            "turn bluetooth off",
        ],
        "bluetooth.toggle",
        "Toggling Bluetooth.",
        &["bluetooth", "power", "toggle"],
    ),
    (
        &[
            "do not disturb",
            "toggle do not disturb",
            "silence notifications",
        ],
        "toggle.dnd",
        "Toggling do not disturb.",
        &["toggle", "notification", "silencing"],
    ),
    (
        &["stay awake", "dont sleep", "do not sleep"],
        "toggle.idle",
        "Staying awake.",
        &["toggle", "idle", "stay-awake"],
    ),
    (
        &[
            "open spotify",
            "launch spotify",
            "play spotify",
            "spotify",
        ],
        "app.launch",
        "Opening Spotify.",
        &["launch", "spotify"],
    ),
    (
        &["open signal", "launch signal", "signal"],
        "app.launch",
        "Opening Signal.",
        &["launch", "signal"],
    ),
    (
        &[
            "toggle bar",
            "toggle the bar",
            "hide the bar",
            "hide bar",
            "show the bar",
            "show bar",
            "hide the panel",
            "show the panel",
        ],
        "toggle.bar",
        "Toggling the bar.",
        &["toggle", "bar"],
    ),
    (
        &[
            "toggle gaps",
            "toggle window gaps",
            "no gaps",
            "window gaps",
        ],
        "hypr.gaps",
        "Toggling gaps.",
        &["hyprland", "window", "gaps", "toggle"],
    ),
    (
        &[
            "toggle animations",
            "toggle effects",
            "reduce motion",
        ],
        "toggle.animations",
        "Toggling animations.",
        &["toggle", "animations"],
    ),
    (
        &[
            "toggle transparency",
            "toggle window transparency",
        ],
        "hypr.transparency",
        "Toggling transparency.",
        &["hyprland", "window", "transparency", "toggle"],
    ),
    (
        &[
            "toggle fullscreen",
            "fullscreen",
            "full screen",
        ],
        "hypr.fullscreen",
        "Toggling fullscreen.",
        &["hyprland", "window", "tiled", "fullscreen", "toggle"],
    ),
    (
        &[
            "toggle layout",
            "toggle tiling",
            "switch layout",
        ],
        "hypr.layout",
        "Toggling layout.",
        &["hyprland", "workspace", "layout", "toggle"],
    ),
    (
        &[
            "screenshot region",
            "region screenshot",
            "screenshot a region",
            "capture a region",
        ],
        "capture.screenshot_region",
        "Region screenshot.",
        &["capture", "screenshot", "region"],
    ),
    (
        &[
            "about this computer",
            "system info",
            "system information",
        ],
        "app.launch",
        "Opening system info.",
        &["launch", "about"],
    ),
    (
        &[
            "screenshot window",
            "window screenshot",
            "capture a window",
            "screenshot a window",
        ],
        "capture.screenshot_window",
        "Window screenshot.",
        &["capture", "screenshot", "windows"],
    ),
    (
        &[
            "fullscreen screenshot",
            "screenshot fullscreen",
            "full screen screenshot",
        ],
        "capture.screenshot_fullscreen",
        "Fullscreen screenshot.",
        &["capture", "screenshot", "fullscreen"],
    ),
    (
        &[
            "copy text from the screen",
            "copy text from screen",
            "capture text",
            "read the screen",
            "ocr",
        ],
        "capture.text",
        "Capturing text.",
        &["capture", "text"],
    ),
    (
        &[
            "close window",
            "close the window",
            "close this window",
            "close the focused window",
        ],
        "hypr.window_close",
        "Closing the window.",
        &["hyprctl", "eval", r#"hl.dispatch(hl.dsp.window.close())"#],
    ),
    (
        &[
            "next workspace",
            "go to the next workspace",
            "switch to the next workspace",
        ],
        "hypr.workspace",
        "Next workspace.",
        &[
            "hyprctl",
            "eval",
            r#"hl.dispatch(hl.dsp.focus({ workspace = "e+1" }))"#,
        ],
    ),
    (
        &[
            "previous workspace",
            "last workspace",
            "go to the previous workspace",
            "switch to the previous workspace",
        ],
        "hypr.workspace",
        "Previous workspace.",
        &[
            "hyprctl",
            "eval",
            r#"hl.dispatch(hl.dsp.focus({ workspace = "e-1" }))"#,
        ],
    ),
    (
        &[
            "toggle touchpad",
            "disable touchpad",
            "enable touchpad",
            "turn off the touchpad",
            "turn on the touchpad",
        ],
        "toggle.touchpad",
        "Toggling the touchpad.",
        &["toggle", "touchpad"],
    ),
    (
        &[
            "toggle screensaver",
            "screensaver",
        ],
        "toggle.screensaver",
        "Toggling the screensaver.",
        &["toggle", "screensaver"],
    ),
    (
        &[
            "fullscreen desktop",
            "toggle fullscreen desktop",
            "hide bar and gaps",
        ],
        "toggle.fullscreen_desktop",
        "Toggling fullscreen desktop.",
        &["toggle", "fullscreen", "desktop"],
    ),
    (
        &[
            "switch audio",
            "switch audio output",
            "switch speakers",
            "switch headphones",
            "change audio output",
        ],
        "audio.output_switch",
        "Switching audio output.",
        &["audio", "output", "switch"],
    ),
    (
        &[
            "keyboard brighter",
            "keyboard backlight up",
            "increase keyboard brightness",
            "keyboard up",
        ],
        "display.keyboard_up",
        "Keyboard brighter.",
        &["brightness", "keyboard", "up"],
    ),
    (
        &[
            "keyboard dimmer",
            "keyboard backlight down",
            "decrease keyboard brightness",
            "keyboard down",
            "dim the keyboard",
        ],
        "display.keyboard_down",
        "Keyboard dimmer.",
        &["brightness", "keyboard", "down"],
    ),
    (
        &[
            "change theme",
            "switch theme",
            "open theme switcher",
            "theme switcher",
        ],
        "theme.switcher",
        "Opening the theme switcher.",
        &["theme", "switcher"],
    ),
    (
        &[
            "open discord",
            "launch discord",
            "discord",
        ],
        "app.launch",
        "Opening Discord.",
        &["launch", "discord", "community"],
    ),
];

fn normalize(raw: &str) -> String {
    let alnum: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    let mut words: Vec<&str> = alnum.split_whitespace().collect();
    if words.first() == Some(&"hey") && words.get(1) == Some(&"nyx") {
        words.drain(..2);
    } else if words.first() == Some(&"nyx") {
        words.remove(0);
    }
    if words.first() == Some(&"please") {
        words.remove(0);
    }
    if words.last() == Some(&"please") {
        words.pop();
    }
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(text: &str) -> Option<&'static str> {
        match route(text) {
            Intent::Desktop { tool, .. } => Some(tool),
            Intent::Chat | Intent::Forget | Intent::Say(_) => None,
        }
    }

    #[test]
    fn locks_on_short_phrases() {
        assert_eq!(tool("Lock the screen."), Some("system.lock"));
        assert_eq!(tool("hey nyx, lock please"), Some("system.lock"));
        assert_eq!(tool("lock the door"), None);
    }

    #[test]
    fn volume_and_launch() {
        assert_eq!(tool("turn it down"), Some("audio.volume_down"));
        assert_eq!(tool("volume up"), Some("audio.volume_up"));
        assert_eq!(tool("mute"), Some("audio.mute"));
        assert_eq!(tool("open the browser"), Some("app.launch"));
        assert_eq!(tool("open a terminal"), Some("app.launch"));
        assert_eq!(tool("brighter"), Some("display.brightness_up"));
        assert_eq!(tool("next wallpaper"), Some("theme.bg_next"));
        assert_eq!(tool("go to workspace 3"), Some("hypr.workspace"));
        assert_eq!(tool("workspace 10"), None);
        assert_eq!(tool("go to workspace 8"), None);
        assert_eq!(tool("workspace 6"), None);
        assert_eq!(tool("open files"), Some("app.launch"));
        assert_eq!(tool("make the screen brighter"), Some("display.brightness_up"));
        assert_eq!(tool("switch workspace 2"), Some("hypr.workspace"));
        assert_eq!(tool("go to workspace one"), Some("hypr.workspace"));
        assert_eq!(tool("workspace won"), Some("hypr.workspace"));
        assert_eq!(tool("Workspace One."), Some("hypr.workspace"));
        assert_eq!(tool("Go to ArcSpice1."), Some("hypr.workspace"));
        assert_eq!(tool("make scream brighter"), Some("display.brightness_up"));
        assert_eq!(tool("Mike scream dimmer."), Some("display.brightness_down"));
        assert_eq!(tool("go to our space one"), Some("hypr.workspace"));
        assert!(matches!(route("mute the mic"), Intent::Say(_)));
    }

    #[test]
    fn leaves_questions_to_the_llm() {
        assert_eq!(tool("What is two plus two?"), None);
        assert_eq!(tool("open"), None);
        assert_eq!(tool("what's the volume of a sphere"), None);
        assert_eq!(tool(""), None);
        assert_eq!(route("forget everything"), Intent::Forget);
        assert_eq!(route("start over"), Intent::Forget);
    }

    #[test]
    fn refuses_unknown_commands_instead_of_pretending() {
        assert!(matches!(route("go to workspace eleven"), Intent::Say(_)));
        match route("go to arcspice one") {
            Intent::Desktop { spoken, .. } => assert_eq!(spoken, "Workspace 1."),
            other => panic!("{other:?}"),
        }
        assert!(matches!(route("make the screen wider"), Intent::Say(_)));
        match route("Going to workspace our space one.") {
            Intent::Desktop { spoken, .. } => assert_eq!(spoken, "Workspace 1."),
            other => panic!("{other:?}"),
        }
        match route("go to workspace one") {
            Intent::Desktop { spoken, .. } => assert_eq!(spoken, "Workspace 1."),
            other => panic!("{other:?}"),
        }
        assert!(matches!(route("go to workspace"), Intent::Say(_)));
        assert!(matches!(route("go to workspace 8"), Intent::Say(_)));
        assert!(matches!(route("go to workspace eleven"), Intent::Say(_)));
    }

    #[test]
    fn more_omarchy_tools() {
        assert_eq!(tool("open spotify"), Some("app.launch"));
        assert_eq!(tool("open signal"), Some("app.launch"));
        assert_eq!(tool("toggle bar"), Some("toggle.bar"));
        assert_eq!(tool("hide the bar"), Some("toggle.bar"));
        assert_eq!(tool("toggle gaps"), Some("hypr.gaps"));
        assert_eq!(tool("toggle animations"), Some("toggle.animations"));
        assert_eq!(tool("toggle transparency"), Some("hypr.transparency"));
        assert_eq!(tool("toggle fullscreen"), Some("hypr.fullscreen"));
        assert_eq!(tool("toggle layout"), Some("hypr.layout"));
        assert_eq!(tool("screenshot region"), Some("capture.screenshot_region"));
        assert_eq!(tool("screenshot window"), Some("capture.screenshot_window"));
        assert_eq!(tool("about this computer"), Some("app.launch"));
        assert_eq!(tool("close the window"), Some("hypr.window_close"));
        assert_eq!(tool("next workspace"), Some("hypr.workspace"));
        assert_eq!(tool("previous workspace"), Some("hypr.workspace"));
        assert_eq!(tool("toggle touchpad"), Some("toggle.touchpad"));
        assert_eq!(tool("switch audio output"), Some("audio.output_switch"));
        assert_eq!(tool("keyboard brighter"), Some("display.keyboard_up"));
        assert_eq!(tool("change theme"), Some("theme.switcher"));
        assert_eq!(tool("open discord"), Some("app.launch"));
        assert_eq!(tool("capture text"), Some("capture.text"));
        assert_eq!(tool("close"), None);
    }
}
