use assistant_core::config::Config;
use gtk::gdk::prelude::*;
use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const CSS: &str = r#"
.assistant-aura { background: transparent; }
.assistant-canvas { min-width: 380px; }
.assistant-canvas .canvas-body { padding: 18px; }
.assistant-canvas .canvas-title { font-size: 18px; font-weight: 700; }
.assistant-canvas .canvas-card { padding: 12px; border-radius: 10px; background: alpha(@theme_surface, 0.55); }
.assistant-canvas .canvas-muted { opacity: 0.65; }
"#;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Listening,
    Thinking,
    Speaking,
    Attention,
    Done,
    Error,
}

struct Aura {
    windows: RefCell<Vec<gtk::Window>>,
    profile: RefCell<Option<assistant_core::VoiceProfile>>,
    state: Cell<State>,
    conversation: Cell<bool>,
    changed: Cell<Instant>,
    phase: Cell<f64>,
    cfg: assistant_core::config::UiCfg,
}

impl Aura {
    fn new(cfg: assistant_core::config::UiCfg) -> Rc<Self> {
        Rc::new(Self {
            windows: RefCell::new(Vec::new()),
            profile: RefCell::new(None),
            state: Cell::new(State::Idle),
            conversation: Cell::new(false),
            changed: Cell::new(Instant::now()),
            phase: Cell::new(0.0),
            cfg,
        })
    }

    fn rebuild(self: &Rc<Self>, app: &gtk::Application) {
        for window in self.windows.borrow_mut().drain(..) {
            window.close();
        }
        if !gtk4_layer_shell::is_supported() || !self.cfg.enabled {
            return;
        }
        let Some(display) = gdk::Display::default() else { return };
        let monitors = display.monitors();
        for i in 0..monitors.n_items() {
            let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() else { continue };
            if !self.cfg.aura_monitors.is_empty()
                && !monitor.connector().is_some_and(|name| self.cfg.aura_monitors.iter().any(|allowed| allowed == name.as_str()))
            {
                continue;
            }
            let window = gtk::Window::new();
            window.set_application(Some(app));
            window.set_decorated(false);
            window.add_css_class("assistant-aura");
            window.init_layer_shell();
            window.set_namespace(Some("nyx-assistant-aura"));
            window.set_layer(Layer::Overlay);
            window.set_keyboard_mode(KeyboardMode::None);
            window.set_exclusive_zone(0);
            window.set_monitor(Some(&monitor));
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                window.set_anchor(edge, true);
            }
            let area = gtk::DrawingArea::new();
            area.set_hexpand(true);
            area.set_vexpand(true);
            let aura = self.clone();
            area.set_draw_func(move |_, cr, w, h| aura.draw(cr, w as f64, h as f64));
            let redraw = area.clone();
            nyx_theme::subscribe(&area, move || redraw.queue_draw());
            window.set_child(Some(&area));
            window.connect_realize(|w| {
                if let Some(surface) = w.surface() {
                    surface.set_input_region(Some(&gtk::cairo::Region::create()));
                }
            });
            window.present();
            self.windows.borrow_mut().push(window);
        }
    }

    fn set_state(&self, state: State) {
        if self.state.replace(state) != state {
            self.changed.set(Instant::now());
        }
        for w in self.windows.borrow().iter() {
            w.queue_draw();
        }
    }

    fn set_persona(&self, id: &str) {
        *self.profile.borrow_mut() = assistant_core::VoiceProfile::load(id).ok().flatten();
        for window in self.windows.borrow().iter() { window.queue_draw(); }
    }

    fn style(&self) -> String {
        self.profile.borrow().as_ref().and_then(|profile| profile.aura_style.clone())
            .unwrap_or_else(|| self.cfg.aura_style.clone())
    }

    fn tick(&self) {
        if self.state.get() == State::Done && self.changed.get().elapsed() > Duration::from_secs(3) {
            self.set_state(State::Idle);
        }
        if self.state.get() == State::Error && self.changed.get().elapsed() > Duration::from_secs(6) {
            self.set_state(State::Idle);
        }
        if self.style() == "edge-flow" && self.visible() {
            self.phase.set((self.phase.get() + 0.012) % 1.0);
            for w in self.windows.borrow().iter() {
                w.queue_draw();
            }
        }
    }

    fn visible(&self) -> bool {
        self.state.get() != State::Idle
            || (self.cfg.aura_visibility == "persistent" && self.conversation.get())
    }

    fn draw(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        if !self.visible() || w < 10.0 || h < 10.0 {
            return;
        }
        let theme = nyx_theme::palette();
        let profile = self.profile.borrow();
        let tint = profile.as_ref().map(|profile| profile.aura_tint.as_str()).unwrap_or("accent");
        let color = match self.state.get() {
            State::Listening | State::Error => &theme.danger,
            State::Speaking | State::Done => &theme.highlight,
            State::Attention => &theme.border,
            State::Thinking | State::Idle => match tint {
                "highlight" => &theme.highlight,
                "danger" => &theme.danger,
                "border" => &theme.border,
                _ => &theme.accent,
            },
        };
        let (r, g, b) = rgb(color);
        let width = self.cfg.aura_thickness;
        let pad = width * 2.0 + 2.0;
        let perim = 2.0 * (w + h - 4.0 * pad);
        let idle_factor = if self.state.get() == State::Idle { 0.22 } else { 1.0 };
        for (multiple, alpha) in [(3.2, 0.13), (1.8, 0.3), (1.0, 0.8)] {
            cr.new_path();
            cr.rectangle(pad, pad, w - 2.0 * pad, h - 2.0 * pad);
            if self.style() == "edge-flow" {
                cr.set_dash(&[perim * 0.14, perim * 0.86], -self.phase.get() * perim);
            } else {
                cr.set_dash(&[], 0.0);
            }
            cr.set_source_rgba(r, g, b, alpha * self.cfg.aura_intensity * idle_factor);
            cr.set_line_width(width * multiple);
            let _ = cr.stroke();
        }
    }
}

fn rgb(s: &str) -> (f64, f64, f64) {
    let s = s.trim_start_matches('#');
    if s.len() < 6 {
        return (0.55, 0.72, 1.0);
    }
    let c = |i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(160) as f64 / 255.0;
    (c(0), c(2), c(4))
}

struct Canvas {
    window: gtk::ApplicationWindow,
    body: gtk::Box,
    pinned: Cell<bool>,
}

impl Canvas {
    fn new(app: &gtk::Application) -> Rc<Self> {
        let window = gtk::ApplicationWindow::new(app);
        window.set_title(Some("Canvas"));
        window.set_default_size(760, 560);
        window.set_resizable(true);
        window.set_hide_on_close(true);
        window.add_css_class("assistant-canvas");
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = gtk::HeaderBar::new();
        let pin = gtk::ToggleButton::with_label("Pin");
        pin.set_tooltip_text(Some("Keep this content during follow-up requests"));
        header.pack_start(&pin);
        let maximize = gtk::Button::from_icon_name("view-fullscreen-symbolic");
        maximize.set_tooltip_text(Some("Maximize"));
        let w = window.clone();
        maximize.connect_clicked(move |_| {
            if w.is_maximized() {
                w.unmaximize();
            } else {
                w.maximize();
            }
        });
        header.pack_end(&maximize);
        let fullscreen = gtk::Button::from_icon_name("view-restore-symbolic");
        fullscreen.set_tooltip_text(Some("Toggle fullscreen"));
        let w = window.clone();
        fullscreen.connect_clicked(move |_| {
            if w.is_fullscreen() {
                w.unfullscreen();
            } else {
                w.fullscreen();
            }
        });
        header.pack_end(&fullscreen);
        outer.append(&header);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.add_css_class("canvas-body");
        scroll.set_child(Some(&body));
        outer.append(&scroll);
        window.set_child(Some(&outer));
        let canvas = Rc::new(Self { window, body, pinned: Cell::new(false) });
        let pin_state = canvas.clone();
        pin.connect_toggled(move |button| pin_state.pinned.set(button.is_active()));
        canvas
    }

    fn show_results(&self, title: &str, results: &[Value]) {
        if self.pinned.get() && self.window.is_visible() {
            return;
        }
        if !self.window.is_visible() {
            self.window.set_default_size(720, (180 + results.len().min(6) as i32 * 110).clamp(400, 760));
        }
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("canvas-title");
        heading.set_xalign(0.0);
        self.body.append(&heading);
        for item in results.iter().take(30) {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
            card.add_css_class("canvas-card");
            let name = item["title"].as_str().or_else(|| item["name"].as_str()).unwrap_or("Result");
            let label = gtk::Label::new(Some(name));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.add_css_class("heading");
            card.append(&label);
            if let Some(url) = item["url"].as_str() {
                let link = gtk::LinkButton::with_label(url, url);
                link.set_halign(gtk::Align::Start);
                card.append(&link);
            }
            if let Some(text) = item["snippet"].as_str().or_else(|| item["text"].as_str()) {
                let snippet = gtk::Label::new(Some(&text.chars().take(2000).collect::<String>()));
                snippet.set_xalign(0.0);
                snippet.set_wrap(true);
                snippet.set_selectable(true);
                snippet.add_css_class("canvas-muted");
                card.append(&snippet);
            }
            self.body.append(&card);
        }
        if !self.window.is_visible() {
            self.window.present();
        }
    }
}

fn listen(tx: mpsc::Sender<Value>) {
    std::thread::spawn(move || loop {
        let socket = assistant_core::paths::socket_path();
        if let Ok(stream) = UnixStream::connect(&socket) {
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if let Ok(value) = serde_json::from_str(&line) {
                    if tx.send(value).is_err() {
                        return;
                    }
                }
            }
        }
        if tx.send(serde_json::json!({"type": "state", "state": "idle"})).is_err() { return; }
        if tx.send(serde_json::json!({"type": "session", "mode": "wake", "remaining_secs": 0})).is_err() { return; }
        std::thread::sleep(Duration::from_secs(2));
    });
}

fn handle_event(event: Value, aura: &Aura, canvas: &Canvas, config: &Config) {
    match event["type"].as_str().unwrap_or("") {
        "state" => {
            let state = match event["state"].as_str().unwrap_or("") {
                "listening" | "transcribing" => State::Listening,
                "thinking" | "executing" => State::Thinking,
                "speaking" => State::Speaking,
                "error" => State::Error,
                _ => State::Idle,
            };
            aura.set_state(state);
        }
        "tool_approval" => aura.set_state(State::Attention),
        "settings" => {
            if let Some(id) = event["running"]["assistant"]["persona_id"].as_str() { aura.set_persona(id); }
        }
        "session" => {
            aura.conversation.set(matches!(event["mode"].as_str(), Some("conversation" | "follow_up"))
                && event["remaining_secs"].as_u64().unwrap_or(0) > 0);
            for window in aura.windows.borrow().iter() { window.queue_draw(); }
        }
        "error" => aura.set_state(State::Error),
        "proactive" => aura.set_state(State::Attention),
        "turn" if event["event"]["type"] == "tool" && config.ui.canvas_enabled => {
            let tool = event["event"]["tool"].as_str().unwrap_or("");
            let data = &event["event"]["result"]["structuredContent"];
            if (tool == "web.search" || tool.ends_with(".search")) && data["results"].as_array().is_some_and(|r| !r.is_empty()) {
                canvas.show_results("Search results", data["results"].as_array().unwrap());
            }
        }
        "assistant_text" if event["done"] == true => aura.set_state(State::Done),
        _ => {}
    }
}

fn poll_events(rx: &Receiver<Value>, aura: &Aura, canvas: &Canvas, config: &Config) {
    for event in rx.try_iter().take(200) {
        handle_event(event, aura, canvas, config);
    }
}

fn main() {
    let config = Config::load().unwrap_or_default();
    let app = gtk::Application::new(Some("io.github.design_nexus.nyx.assistant.visual"), Default::default());
    app.connect_activate(move |app| {
        if !config.ui.enabled {
            return;
        }
        nyx_theme::install(nyx_theme::Style {
            window_class: "assistant-canvas",
            base_skip: &[],
            css: CSS,
            no_glow_css: "",
            extra_colors: None,
            prefs: || nyx_theme::ThemePrefs {
                mode: nyx_theme::ThemeMode::Omarchy,
                theme: String::new(),
                glow: true,
                reduce_motion: false,
            },
            set_theme: None,
        });
        let hold = app.hold();
        let aura = Aura::new(config.ui.clone());
        aura.set_persona(&config.assistant.persona_id);
        aura.rebuild(app);
        let canvas = Canvas::new(app);
        let (tx, rx) = mpsc::channel();
        listen(tx);
        let config = config.clone();
        let aura_for_tick = aura.clone();
        glib::timeout_add_local(Duration::from_millis(40), move || {
            let _ = &hold;
            poll_events(&rx, &aura_for_tick, &canvas, &config);
            aura_for_tick.tick();
            glib::ControlFlow::Continue
        });
        if let Some(display) = gdk::Display::default() {
            let aura = aura.clone();
            let app = app.clone();
            display.monitors().connect_items_changed(move |_, _, _, _| aura.rebuild(&app));
        }
    });
    app.run();
}
