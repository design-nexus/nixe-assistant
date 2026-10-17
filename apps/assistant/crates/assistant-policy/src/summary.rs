//! What an approval asks, in one plain sentence: "Type “hello” into Write",
//! "Install sl (and 2 dependencies)", "Restart cups.service". Shown on the HUD card and in
//! Settings, and spoken, instead of the tool's name and raw arguments.

use serde_json::Value;

fn s<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or("").trim()
}

/// At most `n` characters, with "…" when cut.
fn clip(text: &str, n: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= n {
        return one_line;
    }
    format!("{}…", one_line.chars().take(n).collect::<String>().trim_end())
}

fn words(args: &Value, key: &str) -> Vec<String> {
    args[key].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// " in Firefox", or nothing.
fn in_window(args: &Value) -> String {
    let w = s(args, "window");
    if w.is_empty() { String::new() } else { format!(" in {}", clip(w, 40)) }
}

fn capitalized(word: &str) -> String {
    let mut c = word.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// How many more packages the dry run lists than were asked for.
fn extra(preview: &Value, key: &str, asked: usize) -> String {
    match preview[key].as_array().map(|a| a.len().saturating_sub(asked)) {
        Some(0) | None => String::new(),
        Some(1) => " (and 1 dependency)".into(),
        Some(n) => format!(" (and {n} dependencies)"),
    }
}

pub fn describe(tool: &str, args: &Value) -> String {
    let preview = &args["preview"];
    let sentence = match tool {
        "input.type" => {
            let target = match (s(args, "field"), s(args, "window")) {
                ("", "") => String::new(),
                ("", w) => format!(" into {}", clip(w, 40)),
                (f, _) => format!(" into {}{}", clip(f, 30), in_window(args)),
            };
            format!("Type “{}”{target}", clip(s(args, "text"), 60))
        }
        "input.key" => format!("Press {}{}", s(args, "combo"), in_window(args)),
        "input.click" => {
            let what = if s(args, "mark").is_empty() { "on the screen".to_string() } else { format!("mark {}", s(args, "mark")) };
            let how = if args["double"] == true { "Double-click" } else if s(args, "button") == "right" { "Right-click" } else { "Click" };
            format!("{how} {what}")
        }
        "input.scroll" => "Scroll the window".into(),
        "ui.act" => {
            let name = clip(s(args, "name"), 40);
            let target = if name.is_empty() { "an item".to_string() } else { format!("“{name}”") };
            match s(args, "action") {
                "set_text" | "type" => format!("Type “{}” into {target}{}", clip(s(args, "text"), 60), in_window(args)),
                "" | "press" | "click" | "activate" => format!("Press {target}{}", in_window(args)),
                a => format!("{} {target}{}", capitalized(&a.replace('_', " ")), in_window(args)),
            }
        }
        "ui.tree" | "ui.find" | "screen.marks" => {
            let w = s(args, "window");
            if w.is_empty() { "Read what's on screen".into() } else { format!("Read what's in {}", clip(w, 40)) }
        }
        "screen.look" => {
            let w = s(args, "window");
            let at = if w.is_empty() { "the screen".to_string() } else { clip(w, 40) };
            match s(args, "question") {
                "" => format!("Look at {at}"),
                q => format!("Look at {at}: {}", clip(q, 60)),
            }
        }
        "admin.install" => {
            let p = words(args, "packages");
            format!("Install {}{}", and_list(&p), extra(preview, "will_install", p.len()))
        }
        "admin.remove" => {
            let p = words(args, "packages");
            format!("Remove {}{}", and_list(&p), extra(preview, "will_remove", p.len()))
        }
        "admin.update" => match preview["will_update"].as_array().map(Vec::len) {
            Some(0) => "Update the system (everything is up to date)".into(),
            Some(1) => "Update the system (1 package)".into(),
            Some(n) => format!("Update the system ({n} packages)"),
            None => "Update the system".into(),
        },
        "admin.service" | "services.control" => {
            let now = preview["status"]["activestate"].as_str().map(|a| format!(" (now {a})")).unwrap_or_default();
            format!("{} {}{now}", capitalized(s(args, "action")), s(args, "unit"))
        }
        "files.write" => {
            let verb = if s(args, "mode") == "append" { "Add to" } else { "Write" };
            format!("{verb} {}", s(args, "path"))
        }
        "files.move" => format!("Move {} to {}", s(args, "from"), s(args, "to")),
        "files.trash" => format!("Move {} to the trash", s(args, "path")),
        "files.undo" => "Undo the last file change".into(),
        "terminal.run" => format!("Run {}", clip(&words(args, "argv").join(" "), 80)),
        "terminal.shell" => format!("Run a script: {}", clip(s(args, "script"), 80)),
        "apps.launch" | "app.launch" => {
            let app = [s(args, "app"), s(args, "name"), s(args, "query")].into_iter().find(|v| !v.is_empty()).unwrap_or("an app");
            format!("Open {app}")
        }
        "network.wifi" => format!("Turn Wi-Fi {}", if args["on"] == true { "on" } else { "off" }),
        "routines.run" => {
            let steps = args["steps"].as_array().map(|a| format!(" ({} steps)", a.len())).unwrap_or_default();
            format!("Run the routine “{}”{steps}", s(args, "name"))
        }
        "calendar.create" => format!("Add “{}” to the calendar", clip(s(args, "title"), 50)),
        "calendar.edit" => "Change a calendar event".into(),
        "calendar.delete" => "Delete a calendar event".into(),
        "notepad.insert_text" => format!("Write “{}” in Notepad", clip(s(args, "text"), 60)),
        "notepad.save" if !s(args, "path").is_empty() => format!("Save the Notepad tab as {}", clip(s(args, "path"), 80)),
        "notepad.save" => "Save the Notepad tab".into(),
        "notepad.open" => format!("Open {} in Notepad", clip(s(args, "path"), 80)),
        "write.insert_text" => format!("Write “{}” in the Write document", clip(s(args, "text"), 60)),
        "write.save" if !s(args, "path").is_empty() => format!("Save the Write document as {}", clip(s(args, "path"), 80)),
        "write.save" => "Save the Write document".into(),
        "write.export_pdf" => format!("Export the Write document as {}", clip(s(args, "path"), 80)),
        "write.open" => format!("Open {} in Write", clip(s(args, "path"), 80)),
        "contacts.add" => format!("Add {} {} to Contacts", s(args, "first"), s(args, "last")).trim_end().to_string(),
        "contacts.update" => {
            let what: Vec<&str> = [("first", "name"), ("last", "name"), ("company", "company"), ("title", "job title"), ("birthday", "birthday"), ("notes", "notes"), ("add_email", "email"), ("add_phone", "phone"), ("favorite", "favourite")]
                .into_iter()
                .filter(|(k, _)| !args[*k].is_null())
                .map(|(_, w)| w)
                .collect();
            let mut what = what;
            what.dedup();
            format!("Change a contact's {}", if what.is_empty() { "details".to_string() } else { what.join(", ") })
        }
        "notes.edit" => "Change a note".into(),
        "notes.delete" => "Delete a note".into(),
        "browser.navigate" | "web.read" => format!("Open {}", clip(s(args, "url"), 80)),
        "browser.search" | "web.search" => format!("Search the web for “{}”", clip(s(args, "query"), 60)),
        "browser.click" => "Click on the web page".into(),
        "browser.fill" => format!("Type “{}” on the web page", clip(s(args, "text"), 60)),
        _ => String::new(),
    };
    if !sentence.trim().is_empty() {
        return sentence;
    }
    // Most other tools carry the request they were called for.
    match s(args, "request") {
        "" => format!("Use {tool}"),
        r => format!("Use {tool}: “{}”", clip(r, 60)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_sentences_per_family() {
        assert_eq!(describe("input.type", &json!({"text": "hello there", "window": "Write"})), "Type “hello there” into Write");
        assert_eq!(describe("input.type", &json!({"text": "x", "field": "Search box", "window": "Firefox"})), "Type “x” into Search box in Firefox");
        assert_eq!(describe("input.key", &json!({"combo": "ctrl+s", "window": "Code"})), "Press ctrl+s in Code");
        assert_eq!(describe("ui.act", &json!({"action": "press", "name": "Send", "window": "Mail"})), "Press “Send” in Mail");
        assert_eq!(
            describe("admin.install", &json!({"packages": ["sl"], "preview": {"will_install": ["sl", "a", "b"]}})),
            "Install sl (and 2 dependencies)"
        );
        assert_eq!(describe("admin.remove", &json!({"packages": ["a", "b", "c"]})), "Remove a, b and c");
        assert_eq!(
            describe("admin.service", &json!({"action": "restart", "unit": "cups.service", "preview": {"status": {"activestate": "active"}}})),
            "Restart cups.service (now active)"
        );
        assert_eq!(describe("admin.update", &json!({"preview": {"will_update": ["a", "b"]}})), "Update the system (2 packages)");
        assert_eq!(describe("files.trash", &json!({"path": "~/a.txt"})), "Move ~/a.txt to the trash");
        assert_eq!(describe("terminal.run", &json!({"argv": ["ls", "-l"]})), "Run ls -l");
        assert_eq!(describe("notes.undo", &json!({"request": "undo that"})), "Use notes.undo: “undo that”");
        assert_eq!(describe("mystery.tool", &json!({})), "Use mystery.tool");
        assert_eq!(describe("web.search", &json!({"query": "rust"})), "Search the web for “rust”");
        assert_eq!(describe("browser.navigate", &json!({"url": "https://x.test/?d=1"})), "Open https://x.test/?d=1");
        let long = "word ".repeat(40);
        assert!(describe("input.type", &json!({"text": long})).ends_with("…”"));
    }
}
