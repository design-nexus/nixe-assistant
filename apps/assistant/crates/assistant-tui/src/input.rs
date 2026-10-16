//! Screens, panes and every action, with one table for keys, the Ctrl+K palette and the mouse.
//!
//! Keys resolve to a command name (the label shown in the palette) and `run` carries it out,
//! so a key, a palette choice and a click all take the same path.

use crate::app::{App, EditTarget, PersonaPane, Tab};
use crate::commands::Action;
use crate::navigation::{self, Intent, Screen};
use assistant_ipc::{ApprovalScope, Control};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

pub const SCREENS: &[Screen] = &[
    Screen { name: "Overview", key: '1' },
    Screen { name: "Audio", key: '2' },
    Screen { name: "Download", key: '3' },
    Screen { name: "Persona", key: '4' },
    Screen { name: "Features", key: '5' },
    Screen { name: "Diagnostics", key: '6' },
    Screen { name: "Activity", key: '7' },
    Screen { name: "Calibration", key: '8' },
    Screen { name: "Commands", key: '9' },
    Screen { name: "Plugins", key: '0' },
];

/// Number of rows in the Features list.
pub const FEATURES: usize = 13;

/// One command: palette label, key, modifiers, and why it can't run now (if it can't).
struct Cmd {
    label: &'static str,
    key: KeyCode,
    mods: KeyModifiers,
    disabled: Option<String>,
}

fn cmd(label: &'static str, key: KeyCode, mods: KeyModifiers) -> Cmd {
    Cmd { label, key, mods, disabled: None }
}
fn plain(label: &'static str, c: char) -> Cmd {
    cmd(label, KeyCode::Char(c), KeyModifiers::NONE)
}
fn ctrl(label: &'static str, c: char) -> Cmd {
    cmd(label, KeyCode::Char(c), KeyModifiers::CONTROL)
}
fn alt(label: &'static str, c: char) -> Cmd {
    cmd(label, KeyCode::Char(c), KeyModifiers::ALT)
}
impl Cmd {
    fn unless(mut self, reason: Option<&str>) -> Self {
        self.disabled = reason.map(String::from);
        self
    }
}

impl App {
    pub fn screen(&self) -> usize {
        self.active_tab.index()
    }

    pub fn pane_names(&self) -> &'static [&'static str] {
        match self.active_tab {
            Tab::Dashboard => &["Overview"],
            Tab::Models => &["Models", "Voice", "Detection"],
            Tab::Downloader => &["Recommended", "Search results", "Files"],
            Tab::Persona => &["Personas", "Wake aliases", "Details"],
            Tab::Features => &["Features", "Details"],
            Tab::Diagnostics => &["Report"],
            Tab::Activity => &["Turns", "Details"],
            Tab::Calibration => &["Calibration"],
            Tab::Commands => &["Apps", "Commands", "Details"],
            Tab::Plugins => &["Plugins", "Details"],
        }
    }

    pub fn focused_pane(&self) -> usize {
        self.navigation.focus[self.screen()].min(self.pane_names().len().saturating_sub(1))
    }

    /// Text is being typed: number keys and letters belong to the field.
    pub fn editing(&self) -> bool {
        self.edit_target != EditTarget::None
    }

    pub fn focus_pane(&mut self, pane: usize) {
        let pane = pane.min(self.pane_names().len().saturating_sub(1));
        let screen = self.screen();
        self.navigation.focus[screen] = pane;
        match self.active_tab {
            Tab::Downloader => self.download_pane = pane,
            Tab::Persona if pane < 2 => self.persona_pane = if pane == 0 { PersonaPane::Personas } else { PersonaPane::Aliases },
            Tab::Commands if pane < 2 => {
                let commands = pane == 1;
                if commands != self.commands_pane {
                    self.commands_pane = commands;
                    self.selected_route = 0;
                }
            }
            _ => {}
        }
    }

    pub fn select_screen(&mut self, screen: usize) {
        if screen >= SCREENS.len() || screen == self.screen() {
            return;
        }
        self.active_tab = Tab::ALL[screen];
        let pane = self.navigation.focus[screen];
        self.focus_pane(pane);
    }

    /// The approval to show: the first one waiting, unless it was put aside.
    pub fn approval(&self) -> Option<(&String, &serde_json::Value)> {
        let (id, value) = self.daemon_status.as_ref()?.pending_approvals.iter().next()?;
        (self.approval_hidden.as_deref() != Some(id.as_str())).then_some((id, value))
    }

    fn any_approval(&self) -> bool {
        self.daemon_status.as_ref().is_some_and(|s| !s.pending_approvals.is_empty())
    }

    fn session_mode(&self) -> &str {
        self.daemon_status.as_ref().and_then(|s| s.session["mode"].as_str()).unwrap_or("wake")
    }

    fn cancellable(&self) -> Option<&'static str> {
        if self.download_cancel_flag.is_some() {
            Some("download")
        } else if self.operation_busy && self.active_tab == Tab::Calibration {
            Some("calibration")
        } else {
            None
        }
    }

    /// Everything that can be done here, in palette order. Keys are matched against this list.
    fn command_table(&self) -> Vec<Cmd> {
        let offline = (!self.service_active).then_some("The assistant isn't running");
        let mut c = Vec::new();
        let pane = self.focused_pane();
        match self.active_tab {
            Tab::Dashboard => {}
            Tab::Models => {
                c.push(
                    cmd("Use selected model", KeyCode::Enter, KeyModifiers::NONE)
                        .unless((pane != 0 || self.installed_models.is_empty()).then_some("Select a model in Models first")),
                );
                c.push(plain("Next voice", 'v'));
                c.push(plain("Switch voice model size", 'x'));
                c.push(plain("Faster speech", '+'));
                c.push(plain("Slower speech", '-'));
                c.push(cmd("More sensitive detection", KeyCode::Left, KeyModifiers::NONE));
                c.push(cmd("Less sensitive detection", KeyCode::Right, KeyModifiers::NONE));
                c.push(plain("Switch speech detector", 'b'));
            }
            Tab::Downloader => {
                let busy = self.download_cancel_flag.is_some().then_some("A download is already running");
                let reason = match pane {
                    1 if self.hf_results.is_empty() => Some("Search Hugging Face first"),
                    2 if self.hf_files.is_empty() => Some("Open a repository's files first"),
                    _ => busy,
                };
                c.push(
                    cmd(
                        match pane {
                            1 => "List repository files",
                            _ => "Download selected",
                        },
                        KeyCode::Enter,
                        KeyModifiers::NONE,
                    )
                    .unless(reason),
                );
                c.push(plain("Search Hugging Face", '/'));
                c.push(plain("Download from URL", 'u').unless(busy));
                c.push(
                    plain("Cancel download", 'c').unless(self.download_cancel_flag.is_none().then_some("No download is running")),
                );
            }
            Tab::Persona => {
                let p = self.selected_persona();
                let protected =
                    p.is_some_and(|p| assistant_core::Persona::is_prebuilt(&p.id) || p.id == self.config.assistant.persona_id);
                if pane == 1 {
                    let none = p.is_none_or(|p| p.aliases.is_empty()).then_some("This persona has no aliases");
                    c.push(plain("Add alias", 'a'));
                    c.push(plain("Edit alias", 'e').unless(none));
                    c.push(plain("Delete alias", 'd').unless(none));
                    c.push(plain("Generate aliases", 'g'));
                } else {
                    c.push(cmd("Use persona", KeyCode::Enter, KeyModifiers::NONE));
                    c.push(plain("New persona", 'n'));
                    c.push(plain("Rename persona", 'e'));
                    c.push(
                        plain("Delete persona", 'd').unless(protected.then_some("Built-in and active personas can't be deleted")),
                    );
                    c.push(plain("Next persona voice", 'v'));
                    c.push(plain("Add alias", 'a'));
                    c.push(plain("Generate aliases", 'g'));
                }
                c.push(plain("Toggle wake word", 'w'));
            }
            Tab::Features => {
                c.push(cmd("Toggle feature", KeyCode::Enter, KeyModifiers::NONE));
                c.push(plain("Edit city", 'l'));
                c.push(cmd("More sensitive detection", KeyCode::Left, KeyModifiers::NONE));
                c.push(cmd("Less sensitive detection", KeyCode::Right, KeyModifiers::NONE));
            }
            Tab::Diagnostics => c.push(plain("Run diagnostics", 'r').unless(offline)),
            Tab::Activity => {
                c.push(plain("Clear activity", 'c').unless(offline));
            }
            Tab::Calibration => {
                c.push(cmd("Start calibration", KeyCode::Enter, KeyModifiers::NONE).unless(offline));
                c.push(
                    plain("Use calibration suggestions", 'a')
                        .unless(self.calibration["suggestions"].is_null().then_some("Run a calibration first")),
                );
                c.push(plain("Cancel calibration", 'c').unless(offline));
            }
            Tab::Commands => {
                let empty = self.route_entries().is_empty().then_some("Nothing selected");
                c.push(cmd("Edit entry", KeyCode::Enter, KeyModifiers::NONE).unless(empty));
                c.push(plain("New entry", 'n'));
                c.push(plain("Delete entry", 'd').unless(empty));
                c.push(plain("Preview routing", 'p'));
            }
            Tab::Plugins => {
                let none = self.plugin_draft.is_empty().then_some("No plugins installed");
                c.push(cmd("Toggle plugin", KeyCode::Enter, KeyModifiers::NONE).unless(none));
                c.push(plain("Plugin settings", 'e').unless(none));
                c.push(plain("Retry plugin", 'r').unless(none.or(offline)));
            }
        }
        // Everywhere.
        c.push(ctrl("Apply draft", 's'));
        c.push(ctrl("Apply and restart", 'r'));
        c.push(plain(if self.service_active { "Stop assistant" } else { "Start assistant" }, 's'));
        let muted = self.daemon_status.as_ref().is_some_and(|s| s.muted);
        c.push(plain(if muted { "Unmute" } else { "Mute" }, 'm').unless(offline));
        c.push(plain("Voice preview", 't').unless(offline));
        c.push(plain("Review approval", 'y').unless((!self.any_approval()).then_some("No approval is waiting")));
        c.push(ctrl("Toggle conversation", 'o').unless(offline));
        c.push(ctrl("Toggle dictation", 'd').unless(offline));
        c.push(ctrl("Pause or resume dictation", 'p').unless(offline));
        c.push(ctrl("Browser takeover", 'g').unless(offline));
        c.push(ctrl("Browser resume", 'b').unless(offline));
        c.push(ctrl("Toggle tool mode", 't').unless(offline));
        c.push(ctrl("Cancel operation", 'x').unless(self.cancellable().is_none().then_some("Nothing cancellable is running")));
        c.push(alt("Theme · Aurora", 'a'));
        c.push(alt("Theme · Follow Omarchy", 'o').unless(self.omarchy.is_none().then_some("Omarchy's theme wasn't found")));
        c.push(alt("Theme · Terminal", 't'));
        c.push(plain("Help", '?'));
        c.push(plain("Quit", 'q'));
        c
    }

    /// The Ctrl+K list: what can be done here first, then everywhere, then screens and panes.
    pub fn command_actions(&self) -> Vec<Action> {
        let mut actions: Vec<Action> = self
            .command_table()
            .into_iter()
            .map(|c| Action {
                label: c.label,
                key: c.key,
                modifiers: c.mods,
                target: Intent::Command(c.label),
                disabled: c.disabled,
            })
            .collect();
        actions.extend(SCREENS.iter().enumerate().map(|(i, s)| Action::screen(s.name, i, s.key)));
        actions.extend(self.pane_names().iter().enumerate().map(|(i, p)| Action::function(p, 6, Intent::Pane(i))));
        actions.push(Action::function("Navigate", 2, Intent::Menu));
        actions
    }

    pub fn navigation_entries(&self) -> Vec<navigation::Entry> {
        navigation::entries(SCREENS, self.pane_names())
    }

    pub fn skin(&self) -> navigation::Skin {
        let t = &self.theme;
        navigation::Skin {
            base: t.base(),
            muted: ratatui::style::Style::default().fg(t.muted),
            accent: ratatui::style::Style::default().fg(t.accent),
            selected: t.selected(),
            border: ratatui::style::Style::default().fg(t.border),
        }
    }

    /// A key press. Returns true to quit.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return self.request_quit();
        }
        // Overlays own input, top first.
        if self.editing() {
            self.edit_key(key);
            return false;
        }
        if self.quit_confirm {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') => return true,
                KeyCode::Esc | KeyCode::Char('n') => self.quit_confirm = false,
                _ => {}
            }
            return false;
        }
        if self.help {
            match key.code {
                KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') | KeyCode::Char('q') => self.help = false,
                KeyCode::Down | KeyCode::Char('j') => self.help_scroll = self.help_scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(8),
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(8),
                _ => {}
            }
            return false;
        }
        if self.commands.open || (ctrl && key.code == KeyCode::Char('k') && !self.navigation.open) {
            let actions = self.command_actions();
            if let Some(action) = self.commands.handle(key, &actions) {
                self.dispatch(action.target);
            }
            return false;
        }
        if self.navigation.open {
            let entries = self.navigation_entries();
            if let Some(intent) = self.navigation.key(key, &entries) {
                self.dispatch(intent);
            }
            return false;
        }
        if let Some((id, pending)) = self.approval().map(|(i, p)| (i.clone(), p.clone())) {
            let task = pending["task_scope_allowed"] == true;
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => self.approve(true, ApprovalScope::Once),
                KeyCode::Char('t') if task => self.approve(true, ApprovalScope::Task),
                KeyCode::Char('n') => self.approve(false, ApprovalScope::Once),
                KeyCode::Esc => {
                    self.approval_hidden = Some(id);
                    self.set_toast("Approval set aside; it keeps waiting. (y) review");
                }
                KeyCode::Down | KeyCode::Char('j') => self.approval_scroll = self.approval_scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => self.approval_scroll = self.approval_scroll.saturating_sub(1),
                KeyCode::PageDown => self.approval_scroll = self.approval_scroll.saturating_add(8),
                KeyCode::PageUp => self.approval_scroll = self.approval_scroll.saturating_sub(8),
                _ => {}
            }
            return false;
        }
        // Shared navigation.
        match key.code {
            KeyCode::F(1) | KeyCode::Char('?') => {
                self.help = true;
                self.help_scroll = 0;
                return false;
            }
            KeyCode::F(2) => {
                self.navigation.toggle();
                return false;
            }
            KeyCode::F(6) => {
                let d = if key.modifiers.contains(KeyModifiers::SHIFT) { -1 } else { 1 };
                self.focus_pane(navigation::step(self.focused_pane(), self.pane_names().len(), d));
                return false;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let d = if key.code == KeyCode::Tab { 1 } else { -1 };
                self.select_screen(navigation::step(self.screen(), SCREENS.len(), d));
                return false;
            }
            KeyCode::Char(c) if key.modifiers.is_empty() && c.is_ascii_digit() => {
                if let Some(i) = SCREENS.iter().position(|s| s.key == c) {
                    self.select_screen(i);
                }
                return false;
            }
            KeyCode::Esc => {
                // Leaves a details pane for its list; never quits.
                if self.focused_pane() > 0 {
                    self.focus_pane(self.focused_pane() - 1);
                }
                return false;
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.move_in(self.focused_pane(), -1);
                return false;
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.move_in(self.focused_pane(), 1);
                return false;
            }
            KeyCode::PageUp => {
                self.move_in(self.focused_pane(), -8);
                return false;
            }
            KeyCode::PageDown => {
                self.move_in(self.focused_pane(), 8);
                return false;
            }
            KeyCode::Char(' ') if matches!(self.active_tab, Tab::Features | Tab::Plugins | Tab::Persona) => {
                // Space toggles, as Enter does, where rows are switches.
                let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
                return self.on_key(enter);
            }
            _ => {}
        }
        // Older keys kept beside the table's: d downloads, Enter edits an alias, e edits an
        // entry, Enter runs diagnostics.
        let alias = match (self.active_tab, self.focused_pane(), key.code) {
            (Tab::Downloader, _, KeyCode::Char('d')) => Some("Download selected"),
            (Tab::Persona, 1, KeyCode::Enter) => Some("Edit alias"),
            (Tab::Commands, _, KeyCode::Char('e')) => Some("Edit entry"),
            (Tab::Diagnostics, _, KeyCode::Enter) => Some("Run diagnostics"),
            (Tab::Activity, _, KeyCode::Enter) => {
                self.focus_pane(1);
                return false;
            }
            _ => None,
        };
        if let Some(id) = alias {
            return match self.command_table().into_iter().find(|c| c.label == id).and_then(|c| c.disabled) {
                Some(reason) => {
                    self.set_toast(reason);
                    false
                }
                None => self.run(id),
            };
        }
        if let Some(c) = self
            .command_table()
            .into_iter()
            .filter(|c| {
                // s starts or stops the assistant from Overview only; the palette has it everywhere.
                !(matches!(c.label, "Start assistant" | "Stop assistant") && self.active_tab != Tab::Dashboard)
            })
            .find(|c| {
                c.key == key.code
                    && (c.mods == key.modifiers
                        || (c.mods == KeyModifiers::NONE
                            && key.modifiers == KeyModifiers::SHIFT
                            && matches!(c.key, KeyCode::Char(_))))
            })
        {
            match c.disabled {
                Some(reason) => self.set_toast(reason),
                None => return self.run(c.label),
            }
        } else if key.code == KeyCode::Char('=') {
            return self.run("Faster speech");
        }
        false
    }

    pub fn dispatch(&mut self, intent: Intent) -> bool {
        match intent {
            Intent::Screen(s) => {
                self.navigation.open = false;
                self.select_screen(s)
            }
            Intent::Pane(p) => {
                self.navigation.open = false;
                self.focus_pane(p)
            }
            Intent::Menu => self.navigation.toggle(),
            Intent::Details(p) => self.focus_pane(p),
            Intent::ScrollDetails(p, d) | Intent::Scroll(p, d) => {
                if self.approval().is_some() && p == APPROVAL_PANE {
                    self.approval_scroll = (self.approval_scroll as i32 + d).max(0) as u16;
                } else {
                    self.focus_pane(p);
                    self.move_in(p, d);
                }
            }
            Intent::Row(p, i) => {
                self.focus_pane(p);
                self.select_row(p, i);
            }
            Intent::Activate(p, i) => {
                self.focus_pane(p);
                self.select_row(p, i);
                return self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            }
            Intent::Command(id) => return self.run(id),
            Intent::Theme(id) => self.set_theme(id),
            Intent::Selector(_) => {}
        }
        false
    }

    pub fn on_mouse(&mut self, event: MouseEvent) {
        if self.editing() || self.help {
            return;
        }
        if self.commands.open {
            let actions = self.command_actions();
            if let Some(action) = self.commands.mouse(event, &actions) {
                self.dispatch(action.target);
            }
            return;
        }
        if self.navigation.open {
            let entries = self.navigation_entries();
            if let Some(intent) = self.navigation.menu_mouse(event, &entries) {
                self.dispatch(intent);
            }
            return;
        }
        let Some(intent) = self.navigation.mouse(event) else { return };
        // A dialog owns the mouse: only its own buttons and text respond.
        if self.quit_confirm {
            match intent {
                Intent::Command("Confirm") => self.quit_now = true,
                Intent::Command("Cancel") => self.quit_confirm = false,
                _ => {}
            }
            return;
        }
        if self.approval().is_some() {
            match intent {
                Intent::Command(_) | Intent::ScrollDetails(APPROVAL_PANE, _) | Intent::Scroll(APPROVAL_PANE, _) => {
                    self.dispatch(intent);
                }
                _ => {}
            }
            return;
        }
        self.dispatch(intent);
    }

    /// Move the selection or scroll in a pane.
    fn move_in(&mut self, pane: usize, d: i32) {
        let step = |i: usize, len: usize| (i as i32 + d).clamp(0, len.saturating_sub(1) as i32) as usize;
        let screen = self.screen();
        let scroll = |s: u16| (s as i32 + d).max(0) as u16;
        match (self.active_tab, pane) {
            (Tab::Models, 0) => self.selected_model_idx = step(self.selected_model_idx, self.installed_models.len()),
            (Tab::Downloader, 0) => {
                self.selected_curated_idx = step(self.selected_curated_idx, crate::models::CURATED_MODELS.len())
            }
            (Tab::Downloader, 1) => self.selected_hf_idx = step(self.selected_hf_idx, self.hf_results.len()),
            (Tab::Downloader, 2) => self.selected_file_idx = step(self.selected_file_idx, self.hf_files.len()),
            (Tab::Persona, 0) => {
                let next = step(self.selected_persona_idx, self.personas.len());
                if next != self.selected_persona_idx {
                    self.selected_alias_idx = 0;
                }
                self.selected_persona_idx = next;
            }
            (Tab::Persona, 1) => {
                let len = self.selected_persona().map_or(0, |p| p.aliases.len());
                self.selected_alias_idx = step(self.selected_alias_idx, len)
            }
            (Tab::Features, 0) => self.selected_feature_idx = step(self.selected_feature_idx, FEATURES),
            (Tab::Activity, 0) => {
                let len = self.activity["turns"].as_array().map_or(0, Vec::len);
                let next = step(self.selected_turn, len);
                if next != self.selected_turn {
                    self.navigation.scroll[screen] = 0;
                }
                self.selected_turn = next;
            }
            (Tab::Commands, 0 | 1) => self.selected_route = step(self.selected_route, self.route_entries().len()),
            (Tab::Plugins, 0) => self.selected_plugin = step(self.selected_plugin, self.plugin_draft.len()),
            // Text panes scroll.
            _ => self.navigation.scroll[screen] = scroll(self.navigation.scroll[screen]),
        }
    }

    fn select_row(&mut self, pane: usize, i: usize) {
        match (self.active_tab, pane) {
            (Tab::Models, 0) => self.selected_model_idx = i.min(self.installed_models.len().saturating_sub(1)),
            (Tab::Downloader, 0) => self.selected_curated_idx = i.min(crate::models::CURATED_MODELS.len() - 1),
            (Tab::Downloader, 1) => self.selected_hf_idx = i.min(self.hf_results.len().saturating_sub(1)),
            (Tab::Downloader, 2) => self.selected_file_idx = i.min(self.hf_files.len().saturating_sub(1)),
            (Tab::Persona, 0) => {
                if i != self.selected_persona_idx {
                    self.selected_alias_idx = 0;
                }
                self.selected_persona_idx = i.min(self.personas.len().saturating_sub(1))
            }
            (Tab::Persona, 1) => {
                let len = self.selected_persona().map_or(0, |p| p.aliases.len());
                self.selected_alias_idx = i.min(len.saturating_sub(1))
            }
            (Tab::Features, 0) => self.selected_feature_idx = i.min(FEATURES - 1),
            (Tab::Activity, 0) => {
                if i != self.selected_turn {
                    self.navigation.scroll[self.screen()] = 0;
                }
                let len = self.activity["turns"].as_array().map_or(0, Vec::len);
                self.selected_turn = i.min(len.saturating_sub(1))
            }
            (Tab::Commands, 0 | 1) => self.selected_route = i.min(self.route_entries().len().saturating_sub(1)),
            (Tab::Plugins, 0) => self.selected_plugin = i.min(self.plugin_draft.len().saturating_sub(1)),
            _ => {}
        }
    }

    fn edit(&mut self, target: EditTarget, text: String) {
        self.edit_target = target;
        self.edit_buffer = text;
    }

    fn edit_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.edit_target = EditTarget::None;
                self.edit_buffer.clear();
            }
            KeyCode::Enter => self.submit_edit(),
            KeyCode::Backspace => {
                self.edit_buffer.pop();
            }
            KeyCode::Char('u') if ctrl => self.edit_buffer.clear(),
            KeyCode::Char(c) if !ctrl => self.edit_buffer.push(c),
            _ => {}
        }
    }

    pub fn paste(&mut self, text: &str) {
        if self.editing() {
            self.edit_buffer.push_str(&text.replace(['\n', '\r'], " "));
        }
    }

    pub fn submit_edit(&mut self) {
        let text = self.edit_buffer.trim().to_string();
        let target = self.edit_target;
        let result: anyhow::Result<()> = match target {
            EditTarget::PersonaCreateName => {
                if !text.is_empty() {
                    self.create_persona(&text);
                }
                Ok(())
            }
            EditTarget::PersonaRename => {
                if !text.is_empty() {
                    self.rename_persona(&text);
                }
                Ok(())
            }
            EditTarget::AliasAdd | EditTarget::AliasEdit => {
                if !text.is_empty() {
                    self.alias(target, &text);
                }
                Ok(())
            }
            EditTarget::HfSearchQuery => {
                self.hf_query = text;
                self.search_hf();
                Ok(())
            }
            EditTarget::CustomModelUrl => {
                self.custom_download(&text);
                Ok(())
            }
            EditTarget::RouteEntry => self.edit_route(&text, false),
            EditTarget::RouteNew => self.edit_route(&text, true),
            EditTarget::RoutePreview => self.route_test(&text),
            EditTarget::PluginSettings => serde_json::from_str(&text).map_err(anyhow::Error::from).map(|settings| {
                if let Some((_, s)) = self.plugin_draft.iter_mut().nth(self.selected_plugin) {
                    s.settings = settings;
                    self.changed();
                }
            }),
            EditTarget::LocationCity => {
                self.config.location.city = text.trim().into();
                self.config.location.latitude = None;
                self.config.location.longitude = None;
                self.changed();
                Ok(())
            }
            EditTarget::None => Ok(()),
        };
        match result {
            Ok(()) => {
                self.edit_target = EditTarget::None;
                self.edit_buffer.clear();
            }
            Err(e) => self.set_toast(format!("Edit: {e:#}")),
        }
    }

    pub fn request_quit(&mut self) -> bool {
        if self.operation_busy {
            self.set_toast("Wait for the operation to finish; (Ctrl+X) cancels a calibration or download");
            return false;
        }
        if self.is_dirty {
            self.quit_confirm = true;
            return false;
        }
        true
    }

    fn set_theme(&mut self, id: &str) {
        if id == "omarchy" && self.omarchy.is_none() {
            self.set_toast("Omarchy's theme wasn't found");
            return;
        }
        self.theme_id = id.into();
        self.theme = crate::theme::resolve(id, self.omarchy.as_ref());
        let saved = crate::tuiconfig::TuiConfig { theme: id.into() }.save(&crate::tuiconfig::path());
        self.set_toast(match saved {
            Ok(()) => format!("Theme: {}", self.theme.name),
            Err(e) => format!("Theme: {} (not saved: {e})", self.theme.name),
        });
    }

    /// Carry out a command by its palette label. Returns true to quit.
    pub fn run(&mut self, id: &str) -> bool {
        match id {
            "Quit" => return self.request_quit(),
            "Confirm" => {
                if self.quit_confirm {
                    self.quit_now = true;
                }
            }
            "Cancel" => self.quit_confirm = false,
            "Help" => {
                self.help = true;
                self.help_scroll = 0;
            }
            "Apply draft" => self.apply(false),
            "Apply and restart" => self.apply(true),
            "Start assistant" | "Stop assistant" => self.service(),
            "Mute" | "Unmute" => self.mute(),
            "Voice preview" => self.trigger_preview(),
            "Review approval" => {
                self.approval_hidden = None;
                self.approval_scroll = 0;
            }
            "Allow once" => self.approve(true, ApprovalScope::Once),
            "Allow for task" => self.approve(true, ApprovalScope::Task),
            "Deny" => self.approve(false, ApprovalScope::Once),
            "Later" => self.approval_hidden = self.approval().map(|(id, _)| id.clone()),
            "Toggle conversation" => {
                let enabled = self.session_mode() != "conversation";
                self.control("Conversation", Control::Conversation { enabled }, 5000);
            }
            "Toggle dictation" => {
                let action = if self.session_mode() == "dictation" { "stop" } else { "start" };
                self.control("Dictation", Control::Dictation { action: action.into() }, 5000);
            }
            "Pause or resume dictation" => {
                let action = if self.session_mode() == "dictation_paused" { "resume" } else { "pause" };
                self.control("Dictation", Control::Dictation { action: action.into() }, 5000);
            }
            "Browser takeover" => self.control("Browser takeover", Control::BrowserTakeover, 20_000),
            "Browser resume" => self.control("Browser resume", Control::BrowserResume, 20_000),
            "Toggle tool mode" => {
                let enabled = !self.daemon_status.as_ref().is_some_and(|s| s.tool_mode);
                self.control("Tool mode", Control::ToolMode { enabled }, 2000);
            }
            "Cancel operation" => match self.cancellable() {
                Some("download") => self.cancel_download(),
                Some(_) => {
                    tokio::spawn(async {
                        let _ = assistant_ipc::send_command(&assistant_ipc::Command::Cancel).await;
                    });
                    self.set_toast("Cancelling calibration…");
                }
                None => self.set_toast("Nothing cancellable is running"),
            },
            "Theme · Aurora" => self.set_theme("aurora"),
            "Theme · Follow Omarchy" => self.set_theme("omarchy"),
            "Theme · Terminal" => self.set_theme("terminal"),
            // Audio
            "Use selected model" => {
                if let Some(path) = self.installed_models.get(self.selected_model_idx) {
                    self.config.llm.model = path.display().to_string();
                    self.changed();
                }
            }
            "Next voice" => self.cycle_voice(),
            "Switch voice model size" => {
                self.config.tts.variant = if self.config.tts.variant == "fp32" { "quantized" } else { "fp32" }.into();
                self.changed();
            }
            "Faster speech" => {
                self.config.tts.speed = (self.config.tts.speed + 0.05).min(2.0);
                self.changed();
            }
            "Slower speech" => {
                self.config.tts.speed = (self.config.tts.speed - 0.05).max(0.5);
                self.changed();
            }
            "More sensitive detection" => self.adjust_vad(-1.0),
            "Less sensitive detection" => self.adjust_vad(1.0),
            "Switch speech detector" => {
                self.config.vad.backend = if self.config.vad.backend == "silero" { "energy" } else { "silero" }.into();
                self.changed();
            }
            // Download
            "Download selected" | "List repository files" => self.download_enter(),
            "Search Hugging Face" => self.edit(EditTarget::HfSearchQuery, self.hf_query.clone()),
            "Download from URL" => self.edit(EditTarget::CustomModelUrl, String::new()),
            "Cancel download" => self.cancel_download(),
            // Persona
            "Use persona" => self.activate_persona(),
            "New persona" => self.edit(EditTarget::PersonaCreateName, String::new()),
            "Rename persona" => {
                let name = self.selected_persona().map(|p| p.name.clone()).unwrap_or_default();
                self.edit(EditTarget::PersonaRename, name);
            }
            "Delete persona" => self.delete_persona(),
            "Next persona voice" => {
                let voices = self.voices.clone();
                let active = self.config.assistant.persona_id.clone();
                if let Some(p) = self.personas.get_mut(self.selected_persona_idx) {
                    let index = voices.iter().position(|v| *v == p.voice).unwrap_or(0);
                    p.voice = voices[(index + 1) % voices.len()].into();
                    if p.id == active {
                        self.config.tts.voice = p.voice.clone();
                    }
                    self.changed();
                }
            }
            "Toggle wake word" => {
                self.config.wake_word.enabled = !self.config.wake_word.enabled;
                self.changed();
            }
            "Add alias" => {
                self.focus_pane(1);
                self.edit(EditTarget::AliasAdd, String::new());
            }
            "Edit alias" => {
                let text =
                    self.selected_persona().and_then(|p| p.aliases.get(self.selected_alias_idx)).cloned().unwrap_or_default();
                self.edit(EditTarget::AliasEdit, text);
            }
            "Delete alias" => self.alias(EditTarget::None, ""),
            "Generate aliases" => {
                if let Some(p) = self.personas.get_mut(self.selected_persona_idx) {
                    for a in assistant_core::Persona::generate_aliases(&p.name) {
                        if !p.aliases.contains(&a) {
                            p.aliases.push(a);
                        }
                    }
                    if p.id == self.config.assistant.persona_id {
                        self.config.wake_word.phrases = p.aliases.clone();
                    }
                    self.changed();
                }
            }
            // Features
            "Toggle feature" => self.toggle_feature(),
            "Edit city" => self.edit(EditTarget::LocationCity, self.config.location.city.clone()),
            // Diagnostics, Activity, Calibration
            "Run diagnostics" => self.control("Diagnostics", Control::Diagnostics, 15_000),
            "Clear activity" => self.control("Clear Activity", Control::ClearActivity, 2000),
            "Start calibration" => self.control("Calibration", Control::Calibrate, 30_000),
            "Use calibration suggestions" => self.apply_calibration(),
            "Cancel calibration" => {
                tokio::spawn(async {
                    let _ = assistant_ipc::send_command(&assistant_ipc::Command::Cancel).await;
                });
            }
            // Commands
            "Edit entry" => {
                let text = self.route_entries().get(self.selected_route).map(|v| v.to_string()).unwrap_or_default();
                self.edit(EditTarget::RouteEntry, text);
            }
            "New entry" => {
                let template = if self.commands_pane {
                    r#"{"phrases":["my command"],"tool":"system.lock","reply":"Locking.","omarchy":["system","lock"]}"#
                } else {
                    r#"{"phrases":["my app"],"reply":"Opening.","exec":["program","argument with spaces"]}"#
                };
                self.edit(EditTarget::RouteNew, template.into());
            }
            "Delete entry" => {
                if let Err(e) = self.delete_route() {
                    self.set_toast(e.to_string());
                }
            }
            "Preview routing" => self.edit(EditTarget::RoutePreview, String::new()),
            // Plugins
            "Toggle plugin" => {
                if let Some((_, s)) = self.plugin_draft.iter_mut().nth(self.selected_plugin) {
                    s.enabled = !s.enabled;
                    self.changed();
                }
            }
            "Plugin settings" => {
                if let Some((_, s)) = self.plugin_draft.iter().nth(self.selected_plugin) {
                    let text = s.settings.to_string();
                    self.edit(EditTarget::PluginSettings, text);
                }
            }
            "Retry plugin" => {
                if let Some((id, _)) = self.plugin_draft.iter().nth(self.selected_plugin) {
                    let id = id.clone();
                    self.control("Plugin retry", Control::Plugins { action: "reload".into(), id: Some(id) }, 30_000);
                }
            }
            _ => {}
        }
        self.quit_now
    }
}

/// The pane number the approval dialog's text registers for wheel scrolling.
pub const APPROVAL_PANE: usize = 9;
