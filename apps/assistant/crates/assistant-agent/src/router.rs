/// Fast path for high-confidence desktop commands. Anything ambiguous stays Chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Chat,
    Forget,
    /// Spoken reply only; do not run a desktop command.
    Say(String),
    Desktop {
        tool: String,
        spoken: String,
        omarchy: Vec<String>,
    },
    AgentHandoff {
        agent: String,
        prompt: String,
        spoken: String,
    },
}

pub fn route(text: &str) -> Intent {
    route_with(text, &[], &assistant_core::CommandsFile::shipped())
}

pub fn route_with_apps(text: &str, apps: &[assistant_core::AppLaunch]) -> Intent {
    route_with(text, apps, &assistant_core::CommandsFile::shipped())
}

pub fn route_with(
    text: &str,
    apps: &[assistant_core::AppLaunch],
    cmds: &assistant_core::CommandsFile,
) -> Intent {
    if let Some(hit) = match_agent_handoff(text) {
        return hit;
    }
    let extra = app_vocab(apps);
    let key = preprocess(text, &extra, &cmds.vocab);
    if key.is_empty() {
        return Intent::Chat;
    }
    if cmds.forget.iter().any(|p| p == &key) {
        return Intent::Forget;
    }
    for say in &cmds.say {
        if say.phrases.iter().any(|p| p == &key) {
            return Intent::Say(say.reply.clone());
        }
    }
    if let Some(hit) = exact_command(&key, cmds) {
        return hit;
    }
    if let Some(hit) = exact_app(&key, apps) {
        return hit;
    }
    if let Some(n) = workspace_num(&key, cmds.workspace.max) {
        if is_workspace_move(&key) {
            return workspace_move_intent(n, &cmds.workspace);
        }
        return workspace_intent(n, &cmds.workspace);
    }
    if let Some(hit) = fuzzy_command(&key, cmds) {
        return hit;
    }
    if let Some(hit) = fuzzy_app(&key, apps) {
        return hit;
    }
    Intent::Chat
}

pub fn match_agent_handoff(text: &str) -> Option<Intent> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();

    // 1. Explicit agent phrases: ask/have/tell <agent> [to] <prompt>. Longer names first, so
    // "cursor agent" isn't taken for "cursor".
    let mut words: Vec<&str> = assistant_tools::agent::AGENTS.iter().flat_map(|a| a.words.iter().copied()).collect();
    words.sort_by_key(|w| std::cmp::Reverse(w.len()));
    for agent in &words {
        let prefixes = [
            format!("ask {agent} to "),
            format!("ask {agent} "),
            format!("have {agent} to "),
            format!("have {agent} "),
            format!("tell {agent} to "),
            format!("tell {agent} "),
        ];
        for prefix in &prefixes {
            if lower.starts_with(prefix) {
                let prompt = trimmed[prefix.len()..].trim().to_string();
                let agent_name = assistant_tools::agent::normalize_agent_name(agent).to_string();
                let spoken = format!(
                    "Handing off to {}.",
                    assistant_tools::agent::spoken_agent_name(agent)
                );
                return Some(Intent::AgentHandoff {
                    agent: agent_name,
                    prompt,
                    spoken,
                });
            }
        }

        // Open/launch specific agent: "open claude", "launch agy"
        for verb in &["open", "launch", "start"] {
            if lower == format!("{verb} {agent}") {
                let agent_name = assistant_tools::agent::normalize_agent_name(agent).to_string();
                let spoken = format!(
                    "Opening {}.",
                    assistant_tools::agent::spoken_agent_name(agent)
                );
                return Some(Intent::AgentHandoff {
                    agent: agent_name,
                    prompt: String::new(),
                    spoken,
                });
            }
        }
    }

    // 2. Generic coding agent launch: "open coding agent", "start coding agent"
    for verb in &["open", "launch", "start"] {
        if lower == format!("{verb} coding agent") || lower == format!("{verb} the coding agent") {
            // An empty agent means the configured default.
            return Some(Intent::AgentHandoff {
                agent: String::new(),
                prompt: String::new(),
                spoken: "Opening your coding agent.".into(),
            });
        }
    }

    // 3. Generic handoff with prompt: "handoff to coding agent: <prompt>", "code <prompt>"
    for prefix in &[
        "handoff to coding agent ",
        "hand off to coding agent ",
        "handoff to coding agent: ",
        "hand off to coding agent: ",
        "handoff to agent ",
        "hand off to agent ",
        "handoff to the coding agent ",
        "hand off to the coding agent ",
    ] {
        if lower.starts_with(prefix) {
            let prompt = trimmed[prefix.len()..].trim().to_string();
            return Some(Intent::AgentHandoff {
                agent: String::new(),
                prompt,
                spoken: "Handing off to your coding agent.".into(),
            });
        }
    }

    // 4. "code <prompt>", "start coding <prompt>", "write code for/to <prompt>"
    for prefix in &[
        "code ",
        "start coding ",
        "write code for ",
        "write code to ",
    ] {
        if lower.starts_with(prefix) {
            let prompt = trimmed[prefix.len()..].trim().to_string();
            if !prompt.is_empty() {
                return Some(Intent::AgentHandoff {
                    agent: String::new(),
                    prompt,
                    spoken: "Handing off to your coding agent.".into(),
                });
            }
        }
    }

    None
}

fn app_vocab(apps: &[assistant_core::AppLaunch]) -> Vec<String> {
    let mut words = Vec::new();
    for app in apps {
        for phrase in &app.phrases {
            for w in phrase.split_whitespace() {
                if w.len() >= 4 && !words.iter().any(|x| x == w) {
                    words.push(w.to_string());
                }
            }
        }
    }
    words
}

fn exact_app(key: &str, apps: &[assistant_core::AppLaunch]) -> Option<Intent> {
    for app in apps {
        if app.phrases.iter().any(|p| p == key) {
            return Some(Intent::Desktop {
                tool: "app.launch".into(),
                spoken: app.reply.clone(),
                omarchy: app.args(),
            });
        }
    }
    None
}

fn fuzzy_app(key: &str, apps: &[assistant_core::AppLaunch]) -> Option<Intent> {
    let mut best: Option<Intent> = None;
    let mut best_d = usize::MAX;
    for app in apps {
        for phrase in &app.phrases {
            let d = phrase_distance(key, phrase);
            if d > 0 && d <= max_edits(phrase, key) && d < best_d {
                best_d = d;
                best = Some(Intent::Desktop {
                    tool: "app.launch".into(),
                    spoken: app.reply.clone(),
                    omarchy: app.args(),
                });
            }
        }
    }
    best
}

const FILLER: &[&str] = &[
    "yeah", "yes", "yep", "yup", "ok", "okay", "um", "uh", "hmm", "huh", "ah", "oh", "mhm",
];

/// Backchannel that should not start a new chat when there is no conversation yet.
pub fn is_filler(text: &str) -> bool {
    let key = preprocess(text, &[], &[]);
    FILLER.iter().any(|p| *p == key)
}

fn exact_command(key: &str, cmds: &assistant_core::CommandsFile) -> Option<Intent> {
    for cmd in &cmds.command {
        if cmd.phrases.iter().any(|p| p == key) {
            return Some(Intent::Desktop {
                tool: cmd.tool.clone(),
                spoken: cmd.reply.clone(),
                omarchy: cmd.omarchy.clone(),
            });
        }
    }
    None
}

fn fuzzy_command(key: &str, cmds: &assistant_core::CommandsFile) -> Option<Intent> {
    let mut best: Option<Intent> = None;
    let mut best_d = usize::MAX;
    for cmd in &cmds.command {
        for phrase in &cmd.phrases {
            let d = phrase_distance(key, phrase);
            if d > 0 && d <= max_edits(phrase, key) && d < best_d {
                best_d = d;
                best = Some(Intent::Desktop {
                    tool: cmd.tool.clone(),
                    spoken: cmd.reply.clone(),
                    omarchy: cmd.omarchy.clone(),
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

fn workspace_intent(n: u8, ws: &assistant_core::WorkspaceCommands) -> Intent {
    let n = n.to_string();
    Intent::Desktop {
        tool: ws.tool.clone(),
        spoken: ws.spoken.replace("{n}", &n),
        omarchy: vec!["hyprctl".into(), "eval".into(), ws.focus.replace("{n}", &n)],
    }
}

fn workspace_move_intent(n: u8, ws: &assistant_core::WorkspaceCommands) -> Intent {
    let n = n.to_string();
    Intent::Desktop {
        tool: ws.move_tool.clone(),
        spoken: ws.move_spoken.replace("{n}", &n),
        omarchy: vec![
            "hyprctl".into(),
            "eval".into(),
            ws.r#move.replace("{n}", &n),
        ],
    }
}

fn is_workspace_move(key: &str) -> bool {
    key.split_whitespace()
        .any(|w| w == "move" || w == "send" || w == "put")
}

fn workspace_num(key: &str, max: u8) -> Option<u8> {
    let words: Vec<&str> = key.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        if !looks_like_workspace(w) {
            continue;
        }
        if let Some(next) = words.get(i + 1).copied() {
            if let Some(n) = parse_ws_num(next, max) {
                return Some(n);
            }
        }
        if i > 0 {
            if let Some(n) = parse_ws_num(words[i - 1], max) {
                return Some(n);
            }
        }
        for tok in words.iter().rev() {
            if let Some(n) = parse_ws_num(tok, max) {
                return Some(n);
            }
        }
        return None;
    }
    let (last, rest) = words.split_last()?;
    let n = parse_ws_num(last, max)?;
    if rest.iter().any(|w| {
        *w == "go"
            || *w == "going"
            || *w == "switch"
            || *w == "change"
            || *w == "move"
            || *w == "send"
            || *w == "put"
    }) {
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

fn parse_ws_num(rest: &str, max: u8) -> Option<u8> {
    let rest = rest.strip_prefix("number ").unwrap_or(rest).trim();
    if rest.is_empty() || max == 0 {
        return None;
    }
    if let Ok(n) = rest.parse::<u8>() {
        return (1..=max).contains(&n).then_some(n);
    }
    for (word, n) in NUM_WORDS {
        if rest == *word {
            return (*n <= max).then_some(*n);
        }
    }
    if rest.len() < 3 {
        return None;
    }
    let mut best = None;
    let mut best_d = usize::MAX;
    for (word, n) in NUM_WORDS {
        if *n > max {
            continue;
        }
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

fn preprocess(raw: &str, extra: &[String], vocab: &[String]) -> String {
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
        .map(|w| correct_token(w, extra, vocab))
        .collect::<Vec<_>>()
        .join(" ")
}

fn correct_token(w: &str, extra: &[String], vocab: &[String]) -> String {
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
    snap_vocab(w, vocab, extra).to_string()
}

fn snap_vocab<'a>(w: &'a str, vocab: &'a [String], extra: &'a [String]) -> &'a str {
    if w.len() < 4 {
        return w;
    }
    if vocab.iter().any(|v| v == w) || extra.iter().any(|v| v == w) {
        return w;
    }
    let mut best = w;
    let mut best_d = usize::MAX;
    let consider = |v: &'a str, best: &mut &'a str, best_d: &mut usize| {
        if v.len() < 4 {
            return;
        }
        let d = levenshtein(w, v);
        let max = if w.len().min(v.len()) < 6 { 1 } else { 2 };
        if d > 0 && d <= max && d < *best_d {
            *best_d = d;
            *best = v;
        }
    };
    for v in vocab {
        consider(v.as_str(), &mut best, &mut best_d);
    }
    for v in extra {
        consider(v.as_str(), &mut best, &mut best_d);
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

    fn route(text: &str) -> Intent {
        route_with_apps(text, &assistant_core::AppsFile::shipped().app)
    }

    fn tool(text: &str) -> Option<String> {
        match route(text) {
            Intent::Desktop { tool, .. } => Some(tool),
            Intent::Chat | Intent::Forget | Intent::Say(_) | Intent::AgentHandoff { .. } => None,
        }
    }

    #[test]
    fn locks_on_short_phrases() {
        assert_eq!(tool("Lock the screen.").as_deref(), Some("system.lock"));
        assert_eq!(tool("lock please").as_deref(), Some("system.lock"));
        assert_eq!(tool("lock the door"), None);
    }

    #[test]
    fn volume_and_launch() {
        assert_eq!(tool("turn it down").as_deref(), Some("audio.volume_down"));
        assert_eq!(tool("volume up").as_deref(), Some("audio.volume_up"));
        assert_eq!(tool("mute").as_deref(), Some("audio.mute"));
        assert_eq!(tool("open the browser").as_deref(), Some("app.launch"));
        assert_eq!(tool("open a terminal").as_deref(), Some("app.launch"));
        match route("open terminal") {
            Intent::Desktop { omarchy, .. } => {
                assert_eq!(omarchy, ["launch", "terminal"]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(tool("brighter").as_deref(), Some("display.brightness_up"));
        assert_eq!(tool("next wallpaper").as_deref(), Some("theme.bg_next"));
        assert_eq!(tool("go to workspace 3").as_deref(), Some("hypr.workspace"));
        match route("move to workspace 3") {
            Intent::Desktop { tool, spoken, .. } => {
                assert_eq!(tool, "hypr.workspace_move");
                assert_eq!(spoken, "Moving to workspace 3.");
            }
            other => panic!("{other:?}"),
        }
        match route("send this window to workspace one") {
            Intent::Desktop { tool, spoken, .. } => {
                assert_eq!(tool, "hypr.workspace_move");
                assert_eq!(spoken, "Moving to workspace 1.");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(tool("workspace 10"), None);
        assert_eq!(tool("go to workspace 8"), None);
        assert_eq!(tool("workspace 6"), None);
        assert_eq!(tool("open files").as_deref(), Some("app.launch"));
        match route("open files") {
            Intent::Desktop { omarchy, .. } => {
                assert_eq!(omarchy, ["launch", "nautilus"]);
            }
            other => panic!("{other:?}"),
        }
        match route("open nautilus") {
            Intent::Desktop { omarchy, .. } => assert_eq!(omarchy, ["launch", "nautilus"]),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            tool("make the screen brighter").as_deref(),
            Some("display.brightness_up")
        );
        assert_eq!(
            tool("switch workspace 2").as_deref(),
            Some("hypr.workspace")
        );
        assert_eq!(
            tool("go to workspace one").as_deref(),
            Some("hypr.workspace")
        );
        assert_eq!(tool("workspace won").as_deref(), Some("hypr.workspace"));
        assert_eq!(tool("Workspace One.").as_deref(), Some("hypr.workspace"));
        assert_eq!(tool("Go to ArcSpice1.").as_deref(), Some("hypr.workspace"));
        assert_eq!(
            tool("make scream brighter").as_deref(),
            Some("display.brightness_up")
        );
        assert_eq!(
            tool("Mike scream dimmer.").as_deref(),
            Some("display.brightness_down")
        );
        assert_eq!(
            tool("go to our space one").as_deref(),
            Some("hypr.workspace")
        );
        assert!(matches!(route("mute the mic"), Intent::Say(_)));
        match route("shut down") {
            Intent::Say(s) => assert!(s.contains("won't"), "{s}"),
            other => panic!("{other:?}"),
        }
        match route("reboot") {
            Intent::Say(s) => assert!(s.contains("won't"), "{s}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(tool("what is a shutdown"), None);
    }

    #[test]
    fn leaves_questions_to_the_llm() {
        assert_eq!(tool("What is two plus two?"), None);
        assert_eq!(tool("open"), None);
        assert_eq!(tool("what's the volume of a sphere"), None);
        assert_eq!(tool(""), None);
        assert_eq!(route("forget everything"), Intent::Forget);
        assert_eq!(route("start over"), Intent::Forget);
        assert!(matches!(route("make a joke"), Intent::Chat));
        assert!(matches!(route("show me how gravity works"), Intent::Chat));
        assert!(matches!(route("go to workspace eleven"), Intent::Chat));
        assert!(matches!(route("make the screen wider"), Intent::Chat));
    }

    #[test]
    fn refuses_unknown_commands_instead_of_pretending() {
        match route("go to arcspice one") {
            Intent::Desktop { spoken, .. } => assert_eq!(spoken, "Workspace 1."),
            other => panic!("{other:?}"),
        }
        match route("Going to workspace our space one.") {
            Intent::Desktop { spoken, .. } => assert_eq!(spoken, "Workspace 1."),
            other => panic!("{other:?}"),
        }
        match route("go to workspace one") {
            Intent::Desktop { spoken, .. } => assert_eq!(spoken, "Workspace 1."),
            other => panic!("{other:?}"),
        }
        assert!(matches!(route("go to workspace"), Intent::Chat));
        assert!(matches!(route("go to workspace 8"), Intent::Chat));
        assert!(matches!(route("go to workspace eleven"), Intent::Chat));
    }

    #[test]
    fn more_omarchy_tools() {
        assert_eq!(tool("open spotify").as_deref(), Some("app.launch"));
        assert_eq!(tool("spotify").as_deref(), Some("app.launch"));
        assert!(matches!(route_with_apps("spotify", &[]), Intent::Chat));
        assert_eq!(tool("open signal").as_deref(), Some("app.launch"));
        assert_eq!(tool("toggle bar").as_deref(), Some("toggle.bar"));
        assert_eq!(tool("hide the bar").as_deref(), Some("toggle.bar"));
        assert_eq!(tool("toggle gaps").as_deref(), Some("hypr.gaps"));
        assert_eq!(
            tool("toggle animations").as_deref(),
            Some("toggle.animations")
        );
        assert_eq!(
            tool("toggle transparency").as_deref(),
            Some("hypr.transparency")
        );
        assert_eq!(
            tool("toggle fullscreen").as_deref(),
            Some("hypr.fullscreen")
        );
        assert_eq!(tool("toggle layout").as_deref(), Some("hypr.layout"));
        assert_eq!(
            tool("screenshot region").as_deref(),
            Some("capture.screenshot_region")
        );
        assert_eq!(
            tool("screenshot window").as_deref(),
            Some("capture.screenshot_window")
        );
        assert_eq!(tool("about this computer").as_deref(), Some("app.launch"));
        assert_eq!(
            tool("close the window").as_deref(),
            Some("hypr.window_close")
        );
        assert_eq!(tool("next workspace").as_deref(), Some("hypr.workspace"));
        assert_eq!(
            tool("previous workspace").as_deref(),
            Some("hypr.workspace")
        );
        assert_eq!(tool("former workspace").as_deref(), Some("hypr.workspace"));
        assert_eq!(tool("toggle touchpad").as_deref(), Some("toggle.touchpad"));
        assert_eq!(
            tool("switch audio output").as_deref(),
            Some("audio.output_switch")
        );
        assert_eq!(
            tool("keyboard brighter").as_deref(),
            Some("display.keyboard_up")
        );
        assert_eq!(tool("change theme").as_deref(), Some("theme.switcher"));
        assert_eq!(tool("open discord").as_deref(), Some("app.launch"));
        assert_eq!(tool("open obsidian").as_deref(), Some("app.launch"));
        assert_eq!(tool("open chatgpt").as_deref(), Some("app.launch"));
        assert_eq!(tool("open music").as_deref(), Some("app.launch"));
        assert_eq!(tool("capture text").as_deref(), Some("capture.text"));
        assert_eq!(
            tool("toggle scratchpad").as_deref(),
            Some("hypr.scratchpad")
        );
        assert_eq!(
            tool("move to scratchpad").as_deref(),
            Some("hypr.scratchpad_move")
        );
        assert_eq!(tool("toggle floating").as_deref(), Some("hypr.float"));
        assert_eq!(tool("toggle split").as_deref(), Some("hypr.split"));
        assert_eq!(tool("open passwords").as_deref(), Some("app.launch"));
        assert_eq!(tool("open omawrite").as_deref(), Some("app.launch"));
        assert_eq!(tool("private browser").as_deref(), Some("app.launch"));
        assert_eq!(tool("focus left").as_deref(), Some("hypr.focus"));
        assert_eq!(tool("next window").as_deref(), Some("hypr.window_cycle"));
        assert_eq!(tool("open calculator").as_deref(), Some("app.launch"));
        assert_eq!(tool("color picker").as_deref(), Some("app.launch"));
        assert_eq!(tool("open the menu").as_deref(), Some("app.launch"));
        assert_eq!(tool("apps menu").as_deref(), Some("app.launch"));
        assert_eq!(tool("emoji picker").as_deref(), Some("app.launch"));
        assert_eq!(tool("clipboard history").as_deref(), Some("app.launch"));
        assert_eq!(tool("show keybindings").as_deref(), Some("app.launch"));
        assert_eq!(
            tool("record the screen").as_deref(),
            Some("capture.screenrecord")
        );
        assert_eq!(
            tool("stop recording").as_deref(),
            Some("capture.screenrecord")
        );
        assert_eq!(
            tool("pop the window out").as_deref(),
            Some("hypr.window_pop")
        );
        assert_eq!(tool("close"), None);
        assert!(is_filler("Yeah."));
        assert!(is_filler("ok"));
        assert!(!is_filler("lock the screen"));
    }

    #[test]
    fn user_apps_file_can_change_files_without_rebuild() {
        let apps = [assistant_core::AppLaunch {
            phrases: vec!["open files".into()],
            reply: "Opening files.".into(),
            omarchy: vec![],
            exec: vec![
                "env".into(),
                "GSK_RENDERER=gl".into(),
                "/usr/bin/strata".into(),
            ],
        }];
        match route_with_apps("open files", &apps) {
            Intent::Desktop { omarchy, .. } => {
                assert_eq!(omarchy[0], "exec");
                assert!(omarchy.iter().any(|a| a == "/usr/bin/strata"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn commands_file_can_change_phrases_without_rebuild() {
        let mut cmds = assistant_core::CommandsFile::shipped();
        cmds.command.retain(|c| c.tool != "system.lock");
        cmds.command.push(assistant_core::CommandPhrase {
            phrases: vec!["lock the screen".into()],
            tool: "system.lock".into(),
            reply: "Locked custom.".into(),
            omarchy: vec!["system".into(), "lock".into()],
        });
        match route_with("lock the screen", &[], &cmds) {
            Intent::Desktop { spoken, .. } => assert_eq!(spoken, "Locked custom."),
            other => panic!("{other:?}"),
        }
        assert_eq!(tool("lock").as_deref(), Some("system.lock"));
    }

    #[test]
    fn every_known_agent_is_routed_by_name() {
        for (said, id) in [
            ("ask grok to explain this error", "grok"),
            ("tell muse to write the tests", "muse"),
            ("have cursor agent fix the build", "cursor-agent"),
            ("ask open code to refactor", "opencode"),
            ("ask github copilot to review", "copilot"),
            ("open hermes", "hermes"),
        ] {
            match route(said) {
                Intent::AgentHandoff { agent, .. } => assert_eq!(agent, id, "{said}"),
                other => panic!("{said}: {other:?}"),
            }
        }
        match route("code a parser for toml") {
            Intent::AgentHandoff { agent, prompt, .. } => assert_eq!((agent.as_str(), prompt.as_str()), ("", "a parser for toml")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn agent_handoff_routing() {
        match route("ask claude to fix the auth test") {
            Intent::AgentHandoff {
                agent,
                prompt,
                spoken,
            } => {
                assert_eq!(agent, "claude");
                assert_eq!(prompt, "fix the auth test");
                assert_eq!(spoken, "Handing off to Claude Code.");
            }
            other => panic!("{other:?}"),
        }

        match route("ask agy to review the latest commit") {
            Intent::AgentHandoff {
                agent,
                prompt,
                spoken,
            } => {
                assert_eq!(agent, "agy");
                assert_eq!(prompt, "review the latest commit");
                assert_eq!(spoken, "Handing off to Antigravity.");
            }
            other => panic!("{other:?}"),
        }

        match route("open coding agent") {
            Intent::AgentHandoff {
                agent,
                prompt,
                spoken,
            } => {
                assert_eq!(agent, "", "the configured default");
                assert_eq!(prompt, "");
                assert_eq!(spoken, "Opening your coding agent.");
            }
            other => panic!("{other:?}"),
        }

        match route("open claude") {
            Intent::AgentHandoff {
                agent,
                prompt,
                spoken,
            } => {
                assert_eq!(agent, "claude");
                assert_eq!(prompt, "");
                assert_eq!(spoken, "Opening Claude Code.");
            }
            other => panic!("{other:?}"),
        }
    }
}
