use assistant_core::config::Config;
use gtk::gdk::prelude::*;
use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
    cfg: RefCell<assistant_core::config::UiCfg>,
    opacity: Cell<f64>,
    level: Cell<f64>,
    color: Cell<(f64, f64, f64)>,
    last_frame: Cell<Instant>,
    selected: RefCell<Vec<String>>,
    bar_heights: RefCell<HashMap<String, f64>>,
    fullscreen: Cell<bool>,
    system_reduce_motion: bool,
}

impl Aura {
    fn new(cfg: assistant_core::config::UiCfg, system_reduce_motion: bool) -> Rc<Self> {
        Rc::new(Self {
            windows: RefCell::new(Vec::new()),
            profile: RefCell::new(None),
            state: Cell::new(State::Idle),
            conversation: Cell::new(false),
            changed: Cell::new(Instant::now()),
            phase: Cell::new(0.0),
            cfg: RefCell::new(cfg),
            opacity: Cell::new(0.0),
            level: Cell::new(0.0),
            color: Cell::new((0.5, 0.7, 0.8)),
            last_frame: Cell::new(Instant::now()),
            selected: RefCell::new(Vec::new()),
            bar_heights: RefCell::new(HashMap::new()),
            fullscreen: Cell::new(false),
            system_reduce_motion,
        })
    }

    fn apply_config(self: &Rc<Self>, app: &gtk::Application, cfg: assistant_core::config::UiCfg) {
        *self.cfg.borrow_mut() = cfg;
        self.rebuild(app);
        self.redraw();
    }

    fn monitor_state() -> (Vec<String>, Option<String>, bool, HashMap<String, f64>) {
        let output = std::process::Command::new("hyprctl").args(["-j", "monitors"]).output();
        let state = output.ok().filter(|o| o.status.success())
            .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
            .and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let monitors: Vec<String> = state.iter()
            .filter_map(|m| m["name"].as_str().filter(|n| !n.starts_with("HEADLESS-")).map(str::to_string)).collect();
        let focused = state.iter()
            .find(|m| m["focused"] == true).and_then(|m| m["name"].as_str().map(str::to_string));
        let fullscreen = std::process::Command::new("hyprctl").args(["-j", "activewindow"]).output().ok()
            .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
            .is_some_and(|v| v["fullscreen"].as_i64().unwrap_or(0) != 0);
        let mut bar_heights = HashMap::new();
        if let Some(layers) = std::process::Command::new("hyprctl").args(["-j", "layers"]).output().ok()
            .filter(|o| o.status.success())
            .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
            .and_then(|v| v.as_object().cloned())
        {
            for (connector, output) in layers {
                if let Some(levels) = output["levels"].as_object() {
                    for layer in levels.values().filter_map(Value::as_array).flatten() {
                        if layer["namespace"] == "omarchy-bar" && layer["y"].as_i64() == Some(0) {
                            if let Some(height) = layer["h"].as_f64() {
                                bar_heights.insert(connector.clone(), height.clamp(0.0, 200.0));
                            }
                        }
                    }
                }
            }
        }
        (monitors, focused, fullscreen, bar_heights)
    }

    fn refresh_monitors(self: &Rc<Self>, app: &gtk::Application) {
        let (physical, focused, fullscreen, bar_heights) = Self::monitor_state();
        if self.fullscreen.replace(fullscreen) != fullscreen { self.redraw(); }
        let cfg = self.cfg.borrow();
        let mut selection = if !cfg.aura_monitors.is_empty() {
            physical.iter().filter(|name| cfg.aura_monitors.contains(name)).cloned().collect()
        } else if cfg.aura_monitor_mode == "all" {
            physical
        } else if cfg.aura_monitor_mode == "primary" {
            physical.into_iter().take(1).collect()
        } else {
            focused.filter(|name| physical.contains(name)).or_else(|| physical.first().cloned()).into_iter().collect()
        };
        selection.sort();
        drop(cfg);
        let changed = *self.selected.borrow() != selection || *self.bar_heights.borrow() != bar_heights;
        if changed {
            *self.selected.borrow_mut() = selection;
            *self.bar_heights.borrow_mut() = bar_heights;
            self.rebuild(app);
        }
    }

    fn rebuild(self: &Rc<Self>, app: &gtk::Application) {
        for window in self.windows.borrow_mut().drain(..) {
            window.close();
        }
        if !gtk4_layer_shell::is_supported() || !self.cfg.borrow().enabled || !self.cfg.borrow().aura_enabled {
            return;
        }
        let Some(display) = gdk::Display::default() else { return };
        let monitors = display.monitors();
        for i in 0..monitors.n_items() {
            let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() else { continue };
            if !self.selected.borrow().is_empty()
                && !monitor.connector().is_some_and(|name| self.selected.borrow().iter().any(|allowed| allowed == name.as_str()))
            {
                continue;
            }
            if monitor.connector().is_some_and(|name| name.starts_with("HEADLESS-")) { continue; }
            let window = gtk::Window::new();
            window.set_application(Some(app));
            window.set_decorated(false);
            window.add_css_class("assistant-aura");
            window.init_layer_shell();
            window.set_namespace(Some("nyx-assistant-aura"));
            window.set_layer(Layer::Overlay);
            window.set_keyboard_mode(KeyboardMode::None);
            // -1 ignores other layers' reserved zones, including the top bar.
            window.set_exclusive_zone(-1);
            window.set_monitor(Some(&monitor));
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                window.set_anchor(edge, true);
            }
            let area = gtk::DrawingArea::new();
            area.set_hexpand(true);
            area.set_vexpand(true);
            let bar_height = monitor.connector().and_then(|name| self.bar_heights.borrow().get(name.as_str()).copied()).unwrap_or(0.0);
            let aura = self.clone();
            area.set_draw_func(move |_, cr, w, h| aura.draw(cr, w as f64, h as f64, bar_height));
            let aura = self.clone();
            area.add_tick_callback(move |widget, _| {
                if aura.tick() { widget.queue_draw(); }
                glib::ControlFlow::Continue
            });
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
        if state == State::Idle && matches!(self.state.get(), State::Done | State::Error | State::Attention)
            && self.changed.get().elapsed() < if self.state.get() == State::Error { Duration::from_secs(4) } else { Duration::from_secs(2) }
        { return; }
        if self.state.replace(state) != state {
            self.changed.set(Instant::now());
        }
        self.redraw();
    }

    // A window's queue_draw does not re-run its child's draw function in GTK4.
    fn redraw(&self) {
        for window in self.windows.borrow().iter() {
            if let Some(child) = window.child() {
                child.queue_draw();
            }
        }
    }

    fn set_persona(&self, id: &str) {
        *self.profile.borrow_mut() = assistant_core::VoiceProfile::load(id).ok().flatten();
        self.redraw();
    }

    fn style(&self) -> String {
        self.profile.borrow().as_ref().and_then(|profile| profile.aura_style.clone())
            .unwrap_or_else(|| self.cfg.borrow().aura_style.clone())
    }

    fn reduced_motion(&self) -> bool {
        match self.cfg.borrow().aura_reduce_motion.as_str() {
            "on" => true, "off" => false, _ => self.system_reduce_motion,
        }
    }

    fn tick(&self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame.replace(now)).as_secs_f64().min(0.1);
        if matches!(self.state.get(), State::Done | State::Attention) && self.changed.get().elapsed() > Duration::from_secs(2) {
            self.set_state(State::Idle);
        }
        if self.state.get() == State::Error && self.changed.get().elapsed() > Duration::from_secs(4) {
            self.set_state(State::Idle);
        }
        let target = if self.visible() { 1.0 } else { 0.0 };
        let fade = if self.reduced_motion() { 1.0 } else { (dt * 7.0).min(1.0) };
        let old = self.opacity.get();
        self.opacity.set(old + (target - old) * fade);
        let level = self.level.get();
        self.level.set(level * (1.0 - (dt * 8.0).min(1.0)));
        if !self.reduced_motion() && target > 0.0 {
            let speed = self.cfg.borrow().aura_speed;
            let factor = match self.state.get() { State::Thinking => 0.65, State::Listening => 0.3, _ => 1.0 };
            self.phase.set((self.phase.get() + dt * 0.12 * speed * factor) % 1.0);
        }
        target > 0.0 || self.opacity.get() > 0.01
    }

    fn visible(&self) -> bool {
        let cfg = self.cfg.borrow();
        cfg.enabled && cfg.aura_enabled && !self.fullscreen.get()
            && (self.state.get() != State::Idle || (cfg.aura_visibility == "persistent" && self.conversation.get()))
    }

    fn draw(&self, cr: &gtk::cairo::Context, w: f64, h: f64, bar_height: f64) {
        if self.opacity.get() < 0.01 || w < 10.0 || h < 10.0 {
            return;
        }
        let theme = nyx_theme::palette();
        let profile = self.profile.borrow();
        let tint = profile.as_ref().map(|profile| profile.aura_tint.as_str()).unwrap_or("accent");
        let swatches = nyx_theme::swatches();
        let named = |name: &str, fallback: &str| swatches.iter().find(|(n, _)| n == name).map(|(_, c)| c.as_str()).unwrap_or(fallback).to_string();
        let color = match self.state.get() {
            State::Listening => named("blue", &theme.accent),
            State::Thinking => named("violet", &theme.accent),
            State::Speaking => named("teal", &theme.highlight),
            State::Attention => named("amber", &theme.border),
            State::Done => named("green", &theme.highlight),
            State::Error => theme.danger.clone(),
            State::Idle => match tint { "highlight" => theme.highlight.clone(), "danger" => theme.danger.clone(), "border" => theme.border.clone(), _ => theme.accent.clone() },
        };
        let mut target = rgb(&color);
        if self.cfg.borrow().aura_persona_colors && self.state.get() != State::Error {
            let tint_color = match tint { "highlight" => &theme.highlight, "danger" => &theme.danger, "border" => &theme.border, _ => &theme.accent };
            let p = rgb(tint_color);
            target = (target.0 * 0.82 + p.0 * 0.18, target.1 * 0.82 + p.1 * 0.18, target.2 * 0.82 + p.2 * 0.18);
        }
        let old = self.color.get();
        let blend = if self.reduced_motion() { 1.0 } else { 0.15 };
        let mixed = (old.0 + (target.0-old.0)*blend, old.1 + (target.1-old.1)*blend, old.2 + (target.2-old.2)*blend);
        self.color.set(mixed);
        let cfg = self.cfg.borrow();
        let width = cfg.aura_thickness;
        let pad = width + 2.0;
        let radius = 18.0;
        // Keep even the widest glow and round caps below the bar's bottom edge.
        let top = bar_height + pad + width * 2.5;
        let bottom = h - pad;
        if top + radius >= bottom || w <= 2.0 * (pad + radius) { return; }
        let flow_length = 2.0 * (bottom - radius - top)
            + (w - 2.0 * pad - 2.0 * radius)
            + std::f64::consts::PI * radius;
        let style = self.style();
        let idle_factor = if self.state.get() == State::Idle { 0.2 } else { 1.0 };
        let audio = if cfg.aura_audio_reactive { self.level.get().clamp(0.0, 0.25) * 1.5 } else { 0.0 };
        let breath = if self.reduced_motion() { 1.0 } else { 0.88 + 0.12 * (self.phase.get() * std::f64::consts::TAU).sin().abs() };
        let base_intensity = cfg.aura_intensity * self.opacity.get() * idle_factor * breath;
        if style == "full" {
            let talking = matches!(self.state.get(), State::Listening | State::Speaking);
            let pulse = if cfg.aura_audio_reactive && talking && !self.reduced_motion() {
                ((self.level.get() - 0.003) / 0.06).clamp(0.0, 1.0)
            } else { 0.0 };
            draw_side_glow(cr, w, top, h, width, mixed, base_intensity, pulse);
            return;
        }
        let intensity = base_intensity + cfg.aura_intensity * self.opacity.get() * idle_factor * audio;
        for (multiple, alpha) in [(5.0, 0.045), (3.0, 0.11), (1.5, 0.24), (1.0, 0.64)] {
            edge_flow_path(cr, pad, top, w - pad, bottom, radius);
            let segment = flow_length * cfg.aura_segment_length;
            // A smooth round trip avoids jumping across the bar when the segment wraps.
            let travel = if self.reduced_motion() { 0.5 } else {
                (1.0 - (self.phase.get() * std::f64::consts::TAU).cos()) * 0.5
            };
            cr.set_dash(&[segment, flow_length], -travel * (flow_length - segment));
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            cr.set_source_rgba(mixed.0, mixed.1, mixed.2, alpha * intensity);
            cr.set_line_width(width * multiple);
            let _ = cr.stroke();
        }
    }
}

fn draw_side_glow(
    cr: &gtk::cairo::Context, w: f64, top: f64, h: f64, thickness: f64,
    color: (f64, f64, f64), intensity: f64, pulse: f64,
) {
    let reach = ((24.0 + thickness * 7.0) * (1.0 + 1.3 * pulse)).min(w * 0.18);
    let fade = (24.0 / (h - top)).min(0.25);
    let vertical = gtk::cairo::LinearGradient::new(0.0, top, 0.0, h);
    vertical.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
    vertical.add_color_stop_rgba(fade, 0.0, 0.0, 0.0, 1.0);
    vertical.add_color_stop_rgba(1.0 - fade, 0.0, 0.0, 0.0, 1.0);
    vertical.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
    for (edge, inner) in [(0.0, reach), (w, w - reach)] {
        let horizontal = gtk::cairo::LinearGradient::new(edge, 0.0, inner, 0.0);
        for (stop, alpha) in [(0.0, 0.76), (0.08, 0.66), (0.25, 0.38), (0.55, 0.12), (1.0, 0.0)] {
            horizontal.add_color_stop_rgba(stop, color.0, color.1, color.2, alpha * intensity * (1.0 + 0.55 * pulse));
        }
        let _ = cr.save();
        cr.rectangle(edge.min(inner), top, reach, h - top);
        cr.clip();
        let _ = cr.set_source(&horizontal);
        let _ = cr.mask(&vertical);
        let _ = cr.restore();
    }
}

fn edge_flow_path(cr: &gtk::cairo::Context, left: f64, top: f64, right: f64, bottom: f64, r: f64) {
    cr.new_path();
    cr.move_to(left, top);
    cr.line_to(left, bottom - r);
    cr.arc_negative(left + r, bottom - r, r, std::f64::consts::PI, std::f64::consts::FRAC_PI_2);
    cr.line_to(right - r, bottom);
    cr.arc_negative(right - r, bottom - r, r, std::f64::consts::FRAC_PI_2, 0.0);
    cr.line_to(right, top);
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

fn handle_event(event: Value, aura: &Rc<Aura>, canvas: &Canvas, app: &gtk::Application) {
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
            if let Ok(cfg) = serde_json::from_value::<assistant_core::config::UiCfg>(event["running"]["ui"].clone()) {
                aura.apply_config(app, cfg);
                aura.refresh_monitors(app);
            }
        }
        "session" => {
            aura.conversation.set(matches!(event["mode"].as_str(), Some("conversation" | "follow_up"))
                && event["remaining_secs"].as_u64().unwrap_or(0) > 0);
            aura.redraw();
        }
        "error" => aura.set_state(State::Error),
        "proactive" => aura.set_state(State::Attention),
        "level" if aura.state.get() == State::Listening => {
            let rms = event["rms"].as_f64().unwrap_or(0.0);
            aura.level.set((aura.level.get() * 0.7 + (rms - 0.004).max(0.0) * 0.3).min(0.25));
        }
        "playback_level" if aura.state.get() == State::Speaking => {
            let rms = event["rms"].as_f64().unwrap_or(0.0);
            aura.level.set((aura.level.get() * 0.65 + rms * 0.35).min(0.25));
        }
        "turn" if event["event"]["type"] == "tool" && aura.cfg.borrow().canvas_enabled => {
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

fn poll_events(rx: &Receiver<Value>, aura: &Rc<Aura>, canvas: &Canvas, app: &gtk::Application) {
    for event in rx.try_iter().take(200) {
        handle_event(event, aura, canvas, app);
    }
}

fn main() {
    let config = Config::load().unwrap_or_default();
    let app = gtk::Application::new(Some("io.github.design_nexus.nyx.assistant.visual"), Default::default());
    app.connect_activate(move |app| {
        if !config.ui.enabled {
            return;
        }
        let system_reduce_motion = gtk::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations());
        nyx_theme::use_design_2();
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
                reduce_motion: gtk::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations()),
            },
            set_theme: None,
        });
        let hold = app.hold();
        let aura = Aura::new(config.ui.clone(), system_reduce_motion);
        aura.set_persona(&config.assistant.persona_id);
        aura.refresh_monitors(app);
        let canvas = Canvas::new(app);
        let (tx, rx) = mpsc::channel();
        listen(tx);
        let aura_for_tick = aura.clone();
        let app_for_tick = app.clone();
        glib::timeout_add_local(Duration::from_millis(40), move || {
            let _ = &hold;
            poll_events(&rx, &aura_for_tick, &canvas, &app_for_tick);
            glib::ControlFlow::Continue
        });
        let aura_for_monitors = aura.clone();
        let app_for_monitors = app.clone();
        glib::timeout_add_local(Duration::from_millis(600), move || {
            aura_for_monitors.refresh_monitors(&app_for_monitors);
            glib::ControlFlow::Continue
        });
        if let Some(display) = gdk::Display::default() {
            let aura = aura.clone();
            let app = app.clone();
            display.monitors().connect_items_changed(move |_, _, _, _| aura.refresh_monitors(&app));
        }
    });
    app.run();
}
