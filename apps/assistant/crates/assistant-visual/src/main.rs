use assistant_core::config::Config;
use gtk::gdk::prelude::*;
use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};
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

type MonitorSnapshot = (Vec<String>, Option<String>, Vec<String>, HashMap<String, f64>);

struct Aura {
    windows: RefCell<Vec<gtk::Window>>,
    profile: RefCell<Option<assistant_core::VoiceProfile>>,
    state: Cell<State>,
    base_state: Cell<State>,
    completion_pending: Cell<bool>,
    conversation: Cell<bool>,
    changed: Cell<Instant>,
    phase: Cell<f64>,
    cfg: RefCell<assistant_core::config::UiCfg>,
    opacity: Cell<f64>,
    level: Cell<f64>,
    level_target: Cell<f64>,
    last_level: Cell<Instant>,
    color: Cell<(f64, f64, f64)>,
    last_frame: Cell<Instant>,
    last_tick: Cell<i64>,
    last_state_ms:Cell<u64>,
    selected: RefCell<Vec<String>>,
    monitor_snapshot:RefCell<Option<MonitorSnapshot>>,
    bar_heights: RefCell<HashMap<String, f64>>,
    fullscreen: RefCell<Vec<String>>,
    system_reduce_motion: Cell<bool>,
}

impl Aura {
    fn new(cfg: assistant_core::config::UiCfg, system_reduce_motion: bool) -> Rc<Self> {
        Rc::new(Self {
            windows: RefCell::new(Vec::new()),
            profile: RefCell::new(None),
            state: Cell::new(State::Idle),
            base_state: Cell::new(State::Idle),
            completion_pending: Cell::new(false),
            conversation: Cell::new(false),
            changed: Cell::new(Instant::now()),
            phase: Cell::new(0.0),
            cfg: RefCell::new(cfg),
            opacity: Cell::new(0.0),
            level: Cell::new(0.0),
            level_target: Cell::new(0.0),
            last_level: Cell::new(Instant::now()),
            color: Cell::new((0.5, 0.7, 0.8)),
            last_frame: Cell::new(Instant::now()),
            last_tick: Cell::new(0),
            last_state_ms:Cell::new(0),
            selected: RefCell::new(Vec::new()),
            monitor_snapshot:RefCell::new(None),
            bar_heights: RefCell::new(HashMap::new()),
            fullscreen: RefCell::new(Vec::new()),
            system_reduce_motion: Cell::new(system_reduce_motion),
        })
    }

    fn apply_config(self: &Rc<Self>, app: &gtk::Application, cfg: assistant_core::config::UiCfg) {
        *self.cfg.borrow_mut() = cfg;
        let snapshot=self.monitor_snapshot.borrow().clone();
        if let Some(snapshot)=snapshot {self.refresh_monitors(app,snapshot);}
        self.rebuild(app);
        self.redraw();
    }

    fn monitor_state(runtime: &tokio::runtime::Runtime) -> Option<MonitorSnapshot> {
        runtime.block_on(async {
            let state=assistant_omarchy::hypr::json("monitors").await.ok()?;
            let workspaces=assistant_omarchy::hypr::json("workspaces").await.ok()?;
            let layers=assistant_omarchy::hypr::json("layers").await.ok()?;
            Some(Self::snapshot_from(state.as_array()?,workspaces.as_array()?,layers.as_object()?))
        })
    }

    fn snapshot_from(state: &[Value], workspaces: &[Value], layers: &serde_json::Map<String, Value>) -> MonitorSnapshot {
        let mut monitors: Vec<String> = state.iter()
            .filter_map(|m| m["name"].as_str().filter(|n| !n.starts_with("HEADLESS-")).map(str::to_string)).collect();
        // Wayland has no universal primary monitor. Prefer the configured output, then
        // the layout origin, then an internal panel, and finally a stable connector name.
        let preferred=Config::load().ok().map(|c|c.ui.aura_primary_monitor).unwrap_or_default();
        monitors.sort_by_key(|name| {
            let m=state.iter().find(|m|m["name"].as_str()==Some(name.as_str()));
            let origin=m.is_some_and(|m|m["x"]==0 && m["y"]==0);
            (if !preferred.is_empty() && *name==preferred {0} else if origin {1} else if name.starts_with("eDP") {2} else {3},name.clone())
        });
        let focused = state.iter()
            .find(|m| m["focused"] == true).and_then(|m| m["name"].as_str().map(str::to_string));
        let fullscreen = state.iter().filter_map(|monitor| {
            let id = monitor["activeWorkspace"]["id"].as_i64()?;
            workspaces.iter().any(|w| w["id"].as_i64() == Some(id) && w["hasfullscreen"] == true)
                .then(|| monitor["name"].as_str().map(str::to_string)).flatten()
        }).collect();
        let mut bar_heights = HashMap::new();
        for (connector, output) in layers {
            if let Some(levels) = output["levels"].as_object() {
                for layer in levels.values().filter_map(Value::as_array).flatten() {
                    let monitor_y = state.iter().find(|m| m["name"].as_str() == Some(connector.as_str()))
                        .and_then(|m| m["y"].as_i64());
                    if layer["namespace"] == "omarchy-bar" && layer["y"].as_i64() == monitor_y {
                        if let Some(height) = layer["h"].as_f64() {
                            bar_heights.insert(connector.clone(), height.clamp(0.0, 200.0));
                        }
                    }
                }
            }
        }
        (monitors, focused, fullscreen, bar_heights)
    }

    fn refresh_monitors(self: &Rc<Self>, app: &gtk::Application, snapshot: MonitorSnapshot) {
        *self.monitor_snapshot.borrow_mut()=Some(snapshot.clone());
        let (physical, focused, fullscreen, bar_heights) = snapshot;
        if *self.fullscreen.borrow() != fullscreen {
            *self.fullscreen.borrow_mut() = fullscreen;
            self.redraw();
        }
        let cfg = self.cfg.borrow();
        let mut selection = if !cfg.aura_monitors.is_empty() {
            physical.iter().filter(|name| cfg.aura_monitors.contains(name)).cloned().collect()
        } else if cfg.aura_monitor_mode == "all" {
            physical
        } else if cfg.aura_monitor_mode == "primary" {
            if physical.contains(&cfg.aura_primary_monitor){vec![cfg.aura_primary_monitor.clone()]}else{physical.into_iter().take(1).collect()}
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
        if !self.cfg.borrow().aura_monitors.is_empty() && self.selected.borrow().is_empty() { return; }
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
            let connector = monitor.connector().map(|s| s.to_string()).unwrap_or_default();
            let aura = self.clone();
            area.set_draw_func(move |_, cr, w, h| {
                if !aura.fullscreen.borrow().contains(&connector) { aura.draw(cr, w as f64, h as f64, bar_height); }
            });
            let aura = self.clone();
            area.add_tick_callback(move |widget, _| {
                if aura.tick_frame(widget.frame_clock().map(|c|c.frame_time()).unwrap_or(0)) { widget.queue_draw(); } else { widget.set_visible(false); }
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

    fn set_state(&self, mut state: State) {
        if self.state.get() == State::Error && matches!(state, State::Attention | State::Done)
            && self.changed.get().elapsed() < Duration::from_secs(4) { return; }
        if state == State::Error { self.completion_pending.set(false); }
        if state == State::Done && matches!(self.base_state.get(), State::Thinking | State::Speaking) {
            self.completion_pending.set(true);
            return;
        }
        if matches!(state, State::Listening | State::Thinking) && self.base_state.get() != state {
            self.completion_pending.set(false);
        }
        if matches!(state, State::Idle | State::Listening | State::Thinking | State::Speaking) {
            let previous = self.base_state.get();
            if previous!=state {self.level_target.set(0.0);self.level.set(0.0);}
            self.base_state.set(state);
            if matches!(self.state.get(), State::Done | State::Error | State::Attention)
                && self.changed.get().elapsed() < if self.state.get() == State::Error { Duration::from_secs(4) } else { Duration::from_secs(2) }
                && !(state==State::Listening && self.state.get()==State::Done)
            { return; }
            if state == State::Idle && matches!(previous, State::Thinking | State::Speaking)
                && self.completion_pending.replace(false) { state = State::Done; }
        } else if state == State::Done && self.base_state.get() != State::Idle {
            return;
        }
        if self.state.replace(state) != state {
            self.changed.set(Instant::now());
        }
        self.redraw();
    }

    // A window's queue_draw does not re-run its child's draw function in GTK4.
    fn redraw(&self) {
        for window in self.windows.borrow().iter() {
            if let Some(child) = window.child() {
                child.set_visible(true);
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
            "on" => true, "off" => false, _ => self.system_reduce_motion.get(),
        }
    }

    fn tick_frame(&self, frame: i64) -> bool {
        if frame > 0 && self.last_tick.replace(frame) == frame { return self.visible() || self.opacity.get()>0.01; }
        // Different outputs may have distinct clocks; don't advance twice for the same instant.
        if self.last_frame.get().elapsed()<Duration::from_millis(2) { return self.visible() || self.opacity.get()>0.01; }
        self.tick()
    }

    fn tick(&self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame.replace(now)).as_secs_f64().min(0.1);
        if matches!(self.state.get(), State::Done | State::Attention) && self.changed.get().elapsed() > Duration::from_secs(2) {
            self.state.set(self.base_state.get());
            self.changed.set(now);
        }
        if self.state.get() == State::Error && self.changed.get().elapsed() > Duration::from_secs(4) {
            self.state.set(self.base_state.get());
            self.changed.set(now);
        }
        let target = if self.visible() { 1.0 } else { 0.0 };
        let fade = if self.reduced_motion() { 1.0 } else { (dt * 7.0).min(1.0) };
        let old = self.opacity.get();
        self.opacity.set(old + (target - old) * fade);
        let target_level=if self.last_level.get().elapsed()<Duration::from_millis(180) {self.level_target.get()}else{0.0};
        let level=self.level.get();let tau=if target_level>level {0.035}else{0.16};
        self.level.set(level+(target_level-level)*(1.0-(-dt/tau).exp()));
        if !self.reduced_motion() && target > 0.0 {
            let speed = self.cfg.borrow().aura_speed;
            let factor = match self.state.get() { State::Thinking => 0.65, State::Listening => 0.3, _ => 1.0 };
            self.phase.set((self.phase.get() + dt * 0.12 * speed * factor) % 1.0);
        }
        target > 0.0 || self.opacity.get() > 0.01
    }

    fn visible(&self) -> bool {
        let cfg = self.cfg.borrow();
        cfg.enabled && cfg.aura_enabled
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
        let top = bar_height + pad + width * 5.5;
        let bottom = h - pad;
        if top + radius >= bottom || w <= 2.0 * (pad + radius) { return; }
        let flow_length = 2.0 * (bottom - radius - top)
            + (w - 2.0 * pad - 2.0 * radius)
            + std::f64::consts::PI * radius;
        let style = self.style();
        let idle_factor = if self.state.get() == State::Idle { 0.2 } else { 1.0 };
        let audio = if cfg.aura_audio_reactive { self.level.get().clamp(0.0, 1.0) * 0.65 } else { 0.0 };
        let breath = if self.reduced_motion() { 1.0 } else { 0.88 + 0.12 * (self.phase.get() * std::f64::consts::TAU).sin().abs() };
        let base_intensity = cfg.aura_intensity * self.opacity.get() * idle_factor * breath;
        if style == "full" {
            let talking = matches!(self.state.get(), State::Listening | State::Speaking);
            let pulse = if cfg.aura_audio_reactive && talking && !self.reduced_motion() {
                self.level.get().clamp(0.0,1.0)
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
            cr.set_line_width(width * multiple * (1.0 + 1.8 * audio));
            let _ = cr.stroke();
        }
    }
}

fn meter_fraction(rms: f64, previous: f64) -> f64 {
    let open=0.004;let close=0.0025;
    if !rms.is_finite() || rms<=0.0 || (previous<=0.0 && rms<open) || (previous>0.0 && rms<close) {return 0.0;}
    let floor=20.0_f64*open.log10();((20.0*rms.log10()-floor)/(-12.0-floor)).clamp(0.0,1.0)
}

fn draw_side_glow(
    cr: &gtk::cairo::Context, w: f64, top: f64, h: f64, thickness: f64,
    color: (f64, f64, f64), intensity: f64, pulse: f64,
) {
    let reach = ((24.0 + thickness * 7.0) * (1.0 + 2.0 * pulse)).min(w * 0.18);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_meter_responds_at_ordinary_microphone_levels() {
        assert_eq!(meter_fraction(0.001,0.0),0.0);
        assert!(meter_fraction(0.01,0.0)>0.2);
        assert!(meter_fraction(0.04,0.0)>meter_fraction(0.01,0.0));
        assert_eq!(meter_fraction(f64::NAN,0.5),0.0);
        let aura=Aura::new(Default::default(),false);
        aura.set_state(State::Done);aura.set_state(State::Listening);
        assert!(aura.state.get()==State::Listening);
        aura.level_target.set(meter_fraction(0.02,0.0));
        aura.last_frame.set(Instant::now()-Duration::from_millis(16));aura.tick();
        let high=aura.level.get();assert!(high>0.1);
        aura.level_target.set(0.0);aura.last_frame.set(Instant::now()-Duration::from_millis(16));aura.tick();
        assert!(aura.level.get()<high);
        let reach=|pulse| {
            let mut surface=gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32,800,300).unwrap();
            {let cr=gtk::cairo::Context::new(&surface).unwrap();draw_side_glow(&cr,800.0,30.0,300.0,3.0,(0.1,0.8,1.0),0.4,pulse);}
            surface.flush();let stride=surface.stride() as usize;let pixels=surface.data().unwrap();
            assert!(pixels[..stride*30].iter().all(|b|*b==0),"top bar remains clear");
            (0..400).filter(|x|pixels[stride*150+x*4+3]>3).count()
        };
        assert!(reach(0.8)>reach(0.0)*3/2,"speech expands the visible soft glow");
    }

    #[test]
    fn temporary_status_restores_latest_assistant_state() {
        let aura = Aura::new(Default::default(), false);
        aura.set_state(State::Thinking);
        aura.set_state(State::Error);
        aura.set_state(State::Speaking);
        assert!(aura.state.get() == State::Error);
        aura.changed.set(Instant::now() - Duration::from_secs(5));
        aura.tick();
        assert!(aura.state.get() == State::Speaking);
        aura.set_state(State::Done);
        assert!(aura.state.get() == State::Speaking);
        aura.set_state(State::Idle);
        assert!(aura.state.get() == State::Done);
        aura.changed.set(Instant::now() - Duration::from_secs(3));
        aura.tick();
        assert!(aura.state.get() == State::Idle);
    }

    #[test]
    fn monitor_snapshot_tracks_fullscreen_and_bar_per_output() {
        let monitors = serde_json::json!([
            {"name":"A", "y":0, "focused":true, "activeWorkspace":{"id":1}},
            {"name":"B", "y":-900, "focused":false, "activeWorkspace":{"id":2}},
            {"name":"HEADLESS-1", "y":0, "activeWorkspace":{"id":3}}
        ]);
        let workspaces = serde_json::json!([
            {"id":1,"hasfullscreen":false}, {"id":2,"hasfullscreen":true}
        ]);
        let layers = serde_json::json!({
            "A":{"levels":{"2":[{"namespace":"omarchy-bar","y":0,"h":26}]}},
            "B":{"levels":{"2":[{"namespace":"omarchy-bar","y":-900,"h":32}]}}
        });
        let (physical, focused, fullscreen, bars) = Aura::snapshot_from(
            monitors.as_array().unwrap(), workspaces.as_array().unwrap(), layers.as_object().unwrap());
        assert_eq!(physical, ["A", "B"]);
        assert_eq!(focused.as_deref(), Some("A"));
        assert_eq!(fullscreen, ["B"]);
        assert_eq!(bars.get("A"), Some(&26.0));
        assert_eq!(bars.get("B"), Some(&32.0));
    }

    #[test]
    fn line_chart_renders_into_cairo_surface() {
        let mut surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 640, 360).unwrap();
        let cr = gtk::cairo::Context::new(&surface).unwrap();
        draw_line_chart(&cr, 640, 360, &[(0.0, -2.0), (1.0, 3.0), (2.0, 1.0)]);
        drop(cr);
        surface.flush();
        assert!(surface.data().unwrap().iter().any(|&byte| byte != 0));
    }

    #[test]
    fn image_size_fits_the_monitor_and_preserves_aspect() {
        let (w, h) = image_window_size(4000, 2000, (1600, 1000));
        assert!(w <= 1520 && h <= 920);
        assert!(((w - 60) as f64 / (h - 140) as f64 - 2.0).abs() < 0.02);
        assert_eq!(image_window_size(100, 100, (1600, 1000)), (440, 320));
    }

    #[test]
    fn video_renderer_loads_local_fixture_when_requested() {
        let Ok(path) = std::env::var("NYX_CANVAS_VIDEO_FIXTURE") else { return };
        gtk::init().unwrap();
        let app = gtk::Application::new(Some("io.github.design_nexus.nyx.canvas.test"), gtk::gio::ApplicationFlags::NON_UNIQUE);
        app.register(None::<&gtk::gio::Cancellable>).unwrap();
        let window = gtk::ApplicationWindow::new(&app);
        let (player, _wakeup) = nyx_playback::mpv::Mpv::new(&[("vo", "libmpv"), ("ao", "null"), ("pause", "yes"), ("config", "no"), ("terminal", "no")]).unwrap();
        nyx_playback::video::init(&window, &player, || (320, 180)).unwrap();
        let loaded = player.command(&["loadfile", &path, "replace", "-1", "start=0"]).is_ok();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut duration = None;
        while loaded && Instant::now() < deadline {
            while player.next_event().is_some() {}
            duration = player.get_f64("duration");
            if duration.is_some() { break; }
            std::thread::sleep(Duration::from_millis(50));
        }
        nyx_playback::video::shutdown();
        player.destroy_later();
        assert!(loaded && duration.is_some_and(|d| d > 0.0), "local video failed to load");
    }

    #[test]
    fn pinned_canvas_keeps_content_on_followup_when_requested() {
        if std::env::var_os("NYX_CANVAS_VIDEO_FIXTURE").is_none() { return; }
        gtk::init().unwrap();
        let app = gtk::Application::new(Some("io.github.design_nexus.nyx.canvas.pin-test"), gtk::gio::ApplicationFlags::NON_UNIQUE);
        app.register(None::<&gtk::gio::Cancellable>).unwrap();
        let canvas = Canvas::new(&app);
        let first = canvas.prepare("First");
        first.append(&gtk::Label::new(Some("Pinned")));
        canvas.pin.set_active(true);
        let second = canvas.prepare("Second");
        second.append(&gtk::Label::new(Some("Follow-up")));
        assert_eq!(canvas.pages.n_pages(), 2);
        assert_eq!(canvas.pages.current_page(), Some(1));
        assert!(first.first_child().is_some());
        assert!(canvas.pins.borrow()[0]);
        assert!(!canvas.pins.borrow()[1]);
    }

    #[test]
    fn canvas_content_types_construct_without_opening_a_window() {
        if std::env::var_os("NYX_CANVAS_VIDEO_FIXTURE").is_none() { return; }
        gtk::init().unwrap();
        let app = gtk::Application::new(Some("io.github.design_nexus.nyx.canvas.content-test"), gtk::gio::ApplicationFlags::NON_UNIQUE);
        app.register(None::<&gtk::gio::Cancellable>).unwrap();
        let canvas = Canvas::new(&app);
        let cases = [
            serde_json::json!({"version":1,"kind":"text","title":"Text","text":"Hello"}),
            serde_json::json!({"version":1,"kind":"table","title":"Table","columns":["A"],"rows":[["B"]]}),
            serde_json::json!({"version":1,"kind":"image","title":"Image","path":"/tmp/nyx-canvas-image-test.png"}),
            serde_json::json!({"version":1,"kind":"gallery","title":"Gallery","paths":["/tmp/nyx-canvas-image-test.png"]}),
            serde_json::json!({"version":1,"kind":"chart","title":"Bars","items":[{"label":"A","value":2}]}),
            serde_json::json!({"version":1,"kind":"line_chart","title":"Line","points":[{"x":0,"y":1},{"x":1,"y":2}]}),
            serde_json::json!({"version":1,"kind":"pdf","title":"PDF","path":"/tmp/nyx-canvas-doc-test.pdf"}),
            serde_json::json!({"version":1,"kind":"document","title":"Document","path":concat!(env!("CARGO_MANIFEST_DIR"),"/../../docs/CANVAS.md")}),
            serde_json::json!({"version":1,"kind":"map","title":"Map","latitude":41.88,"longitude":-87.63}),
            serde_json::json!({"version":1,"kind":"video","title":"Video","path":"/tmp/nyx-canvas-video-test.mp4"}),
            serde_json::json!({"version":1,"kind":"links","title":"Links","links":[{"label":"Example","url":"https://example.org"}]}),
        ];
        assert!(pdf_view("/tmp/nyx-canvas-doc-test.pdf").is::<gtk::Box>());
        for content in cases { assert!(canvas.render_content(&content)); }
        assert!(!canvas.window.is_visible());
        assert_eq!(canvas.video.active.get(), 0);
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
    pages: gtk::Notebook,
    bodies: RefCell<Vec<gtk::Box>>,
    pins: RefCell<Vec<bool>>,
    pin: gtk::ToggleButton,
    active_monitor: RefCell<Option<String>>,
    requested_size: Rc<Cell<(i32, i32)>>,
    switching_page: Cell<bool>,
    video: Rc<VideoController>,
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
        let pages = gtk::Notebook::new();
        pages.set_vexpand(true);
        pages.set_show_tabs(false);
        let (scroll, body) = Self::page();
        pages.append_page(&scroll, Some(&gtk::Label::new(Some("Canvas"))));
        outer.append(&pages);
        window.set_child(Some(&outer));
        let requested_size = Rc::new(Cell::new((760, 560)));
        let size_on_map = requested_size.clone();
        window.connect_map(move |w| {
            let w = w.downgrade();
            let size = size_on_map.get();
            glib::idle_add_local_once(move || {
                if let Some(w) = w.upgrade() { if !w.is_maximized() && !w.is_fullscreen() { w.set_default_size(size.0, size.1); } }
            });
        });
        let canvas = Rc::new(Self { window, pages, bodies: RefCell::new(vec![body]), pins: RefCell::new(vec![false]), pin: pin.clone(), active_monitor: RefCell::new(None), requested_size, switching_page: Cell::new(false), video: Rc::new(VideoController::default()) });
        let pin_state = canvas.clone();
        pin.connect_toggled(move |button| {
            if pin_state.switching_page.get() { return; }
            if let Some(index) = pin_state.pages.current_page() {
                if let Some(pinned) = pin_state.pins.borrow_mut().get_mut(index as usize) { *pinned = button.is_active(); }
            }
        });
        let switch_state = canvas.clone();
        canvas.pages.connect_switch_page(move |_, _, index| {
            let pinned = switch_state.pins.borrow().get(index as usize).copied().unwrap_or(false);
            switch_state.switching_page.set(true);
            switch_state.pin.set_active(pinned);
            switch_state.switching_page.set(false);
        });
        canvas
    }

    fn page() -> (gtk::ScrolledWindow, gtk::Box) {
        let scroll = gtk::ScrolledWindow::new();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.add_css_class("canvas-body");
        scroll.set_child(Some(&body));
        (scroll, body)
    }

    fn prepare(&self, title: &str) -> gtk::Box {
        let mut index = self.pages.current_page().unwrap_or(0);
        if self.pins.borrow().get(index as usize).copied().unwrap_or(false) {
            let (scroll, body) = Self::page();
            index = self.pages.append_page(&scroll, Some(&gtk::Label::new(Some(title))));
            self.bodies.borrow_mut().push(body);
            self.pins.borrow_mut().push(false);
            self.pages.set_show_tabs(true);
            self.pages.set_current_page(Some(index));
            self.pin.set_active(false);
        }
        let page = self.pages.nth_page(Some(index)).unwrap();
        self.pages.set_tab_label_text(&page, title);
        let body = self.bodies.borrow()[index as usize].clone();
        while let Some(child) = body.first_child() { body.remove(&child); }
        body
    }

    fn show_content(&self, content: &Value) {
        if !self.render_content(content) { return; }
        if !self.window.is_maximized() && !self.window.is_fullscreen() {
            let (width, height) = match content["kind"].as_str().unwrap_or("") {
                "pdf" | "document" => (760, 800),
                "image" => (760, 600),
                "gallery" | "video" => (900, 650),
                "map" | "chart" | "line_chart" | "table" => (820, 620),
                "text" => (680, (160 + content["text"].as_str().unwrap_or("").len().min(2400) as i32 / 2).clamp(280, 650)),
                _ => (680, 500),
            };
            let (width, height) = self.fit_size(width, height);
            self.requested_size.set((width, height));
            self.window.set_default_size(width, height);
        }
        if !self.window.is_visible() { self.window.set_visible(true); }
    }

    fn monitor_size(&self) -> (i32, i32) {
        let Some(display) = gdk::Display::default() else { return (1280, 800) };
        let monitors = display.monitors();
        let active = self.active_monitor.borrow();
        let mut fallback = None;
        for i in 0..monitors.n_items() {
            let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() else { continue };
            let connector = monitor.connector();
            if connector.as_ref().is_some_and(|name| name.starts_with("HEADLESS-")) { continue; }
            let geometry = monitor.geometry();
            let size = (geometry.width(), geometry.height());
            if active.as_deref().is_some_and(|name| connector.as_ref().is_some_and(|c| c.as_str() == name)) { return size; }
            fallback.get_or_insert(size);
        }
        fallback.unwrap_or((1280, 800))
    }

    fn fit_size(&self, width: i32, height: i32) -> (i32, i32) {
        let (monitor_w, monitor_h) = self.monitor_size();
        (width.min((monitor_w - 80).max(380)), height.min((monitor_h - 80).max(280)))
    }

    fn render_content(&self, content: &Value) -> bool {
        if content["version"] != 1 { return false; }
        let Some(kind) = content["kind"].as_str() else { return false };
        if !matches!(kind, "text" | "table" | "image" | "gallery" | "chart" | "line_chart" | "pdf" | "document" | "map" | "video" | "links") { return false; }
        let title = content["title"].as_str().unwrap_or("Canvas");
        let body = self.prepare(title);
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("canvas-title");
        heading.set_xalign(0.0);
        heading.set_wrap(true);
        body.append(&heading);
        match kind {
            "text" => {
                let label = gtk::Label::new(content["text"].as_str());
                label.set_xalign(0.0);
                label.set_wrap(true);
                label.set_selectable(true);
                body.append(&label);
            }
            "table" => {
                let grid = gtk::Grid::new();
                grid.set_column_spacing(24);
                grid.set_row_spacing(8);
                if let Some(columns) = content["columns"].as_array() {
                    for (col, item) in columns.iter().take(12).enumerate() {
                        let label = gtk::Label::new(item.as_str());
                        label.add_css_class("heading");
                        label.set_xalign(0.0);
                        grid.attach(&label, col as i32, 0, 1, 1);
                    }
                }
                if let Some(rows) = content["rows"].as_array() {
                    for (row, cells) in rows.iter().take(50).enumerate() {
                        if let Some(cells) = cells.as_array() {
                            for (col, cell) in cells.iter().take(12).enumerate() {
                                let label = gtk::Label::new(cell.as_str());
                                label.set_xalign(0.0);
                                label.set_wrap(true);
                                label.set_selectable(true);
                                grid.attach(&label, col as i32, row as i32 + 1, 1, 1);
                            }
                        }
                    }
                }
                let scroll = gtk::ScrolledWindow::new();
                scroll.set_child(Some(&grid));
                body.append(&scroll);
            }
            "image" => if let Some(path) = content["path"].as_str() {
                let image = image_view(path);image.set_widget_name("canvas-image");
                let window=self.window.downgrade();let requested=self.requested_size.clone();let monitor=self.monitor_size();
                image.connect_paintable_notify(move |picture| {
                    if let (Some(window),Some(paintable))=(window.upgrade(),picture.paintable()) {
                        if !window.is_maximized()&&!window.is_fullscreen() && picture.root().is_some() {
                            let size=image_window_size(paintable.intrinsic_width(),paintable.intrinsic_height(),monitor);requested.set(size);window.set_default_size(size.0,size.1);
                        }
                    }
                });
                image.set_can_shrink(true);
                image.set_content_fit(gtk::ContentFit::Contain);
                image.set_size_request(320, 320);
                body.append(&image);
            },
            "gallery" => if let Some(paths) = content["paths"].as_array() {
                let grid = gtk::FlowBox::new();
                grid.set_selection_mode(gtk::SelectionMode::None);
                grid.set_max_children_per_line(3);
                grid.set_row_spacing(12);
                grid.set_column_spacing(12);
                for path in paths.iter().take(12).filter_map(Value::as_str) {
                    let image = image_view(path);
                    image.set_can_shrink(true);
                    image.set_content_fit(gtk::ContentFit::Contain);
                    image.set_size_request(200, 160);
                    grid.insert(&image, -1);
                }
                body.append(&grid);
            },
            "chart" => if let Some(items) = content["items"].as_array() {
                body.append(&bar_chart(items));
            },
            "line_chart" => if let Some(points) = content["points"].as_array() {
                let points = points.iter().filter_map(|p| Some((p["x"].as_f64()?, p["y"].as_f64()?))).collect::<Vec<_>>();
                body.append(&line_chart(points));
            },
            "pdf" => if let Some(path) = content["path"].as_str() { body.append(&pdf_view(path)); },
            "document" => if let Some(path) = content["path"].as_str() {
                let view = gtk::TextView::new();
                view.set_editable(false);
                view.set_cursor_visible(false);
                view.set_wrap_mode(gtk::WrapMode::WordChar);
                view.buffer().set_text("Loading document…");
                let buffer=view.buffer();let path=path.to_string();
                glib::spawn_future_local(async move {
                    let text=gtk::gio::spawn_blocking(move || {
                        let file=std::fs::File::open(path).ok()?;let mut bytes=Vec::new();file.take(1024*1024+1).read_to_end(&mut bytes).ok()?;
                        if bytes.len()>1024*1024{return None;}String::from_utf8(bytes).ok()
                    }).await.ok().flatten().unwrap_or_else(||"Unable to read this text document.".into());
                    buffer.set_text(&text);
                });
                body.append(&view);
            },
            "map" => if let (Some(lat), Some(lon)) = (content["latitude"].as_f64(), content["longitude"].as_f64()) {
                body.append(&map_view(lat, lon, content["zoom"].as_f64().unwrap_or(13.0)));
            },
            "video" => if let Some(path) = content["path"].as_str() {
                body.append(&video_view(&self.window, path, self.video.clone()));
            },
            "links" => if let Some(links) = content["links"].as_array() {
                if let Some(text) = content["text"].as_str() {
                    let label = gtk::Label::new(Some(text));
                    label.set_xalign(0.0);
                    label.set_wrap(true);
                    label.set_selectable(true);
                    body.append(&label);
                }
                for link in links.iter().take(20) {
                    if let (Some(label), Some(url)) = (link["label"].as_str(), link["url"].as_str()) {
                        let button = gtk::LinkButton::with_label(url, label);
                        button.set_halign(gtk::Align::Start);
                        body.append(&button);
                    }
                }
            },
            _ => unreachable!(),
        }
        true
    }
}

fn image_window_size(width: i32, height: i32, monitor: (i32, i32)) -> (i32, i32) {
    if width <= 0 || height <= 0 { return (760, 600); }
    let max_w = (monitor.0 - 140).max(320) as f64;
    let max_h = (monitor.1 - 220).max(200) as f64;
    let scale = (max_w / width as f64).min(max_h / height as f64).min(1.0);
    (((width as f64 * scale) as i32 + 60).max(440), ((height as f64 * scale) as i32 + 140).max(320))
}

#[derive(Default)]
struct VideoController {
    player: RefCell<Option<nyx_playback::mpv::Mpv>>,
    active: Cell<u64>,
    next: Cell<u64>,
}

impl VideoController {
    fn stop(&self, token: u64) {
        if self.active.get() != token { return; }
        self.active.set(0);
        nyx_playback::video::shutdown();
        if let Some(player) = self.player.borrow_mut().take() { player.destroy_later(); }
    }
}

impl Drop for VideoController {
    fn drop(&mut self) {
        nyx_playback::video::shutdown();
        if let Some(player) = self.player.get_mut().take() { player.destroy_later(); }
    }
}

fn video_view(window: &gtk::ApplicationWindow, path: &str, controller: Rc<VideoController>) -> gtk::Widget {
    use nyx_playback::{controls, mpv, video};
    let token = controller.next.get().wrapping_add(1).max(1);
    controller.next.set(token);
    let position = Rc::new(Cell::new(0.0f64));
    let map_epoch = Rc::new(Cell::new(0u64));
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let picture = gtk::Picture::for_paintable(&video::paintable());
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_size_request(400, 300);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    outer.append(&picture);
    let seek = controls::SeekWidgets::new();
    let bar = controls::OsdControls::new(&seek.root, 1.0, 1.0, false);
    for widget in [&bar.prev, &bar.next] { widget.set_visible(false); }
    for widget in [&bar.chapters, &bar.shape, &bar.speed, &bar.audio, &bar.subs] { widget.set_visible(false); }
    outer.append(&bar.root);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    outer.append(&status);

    let w = window.downgrade();
    let file = path.to_string();
    let c = controller.clone();
    let saved_position = position.clone();
    let notice = status.clone();
    picture.connect_map(move |picture| {
        let old = c.active.get();
        if old != 0 { c.stop(old); }
        let Some(window) = w.upgrade() else { return };
        let (player, _wakeup) = match mpv::Mpv::new(&[("vo", "libmpv"), ("idle", "yes"), ("pause", "yes"),
            ("config", "no"), ("terminal", "no"), ("ytdl", "no"), ("load-scripts", "no")]) {
            Ok(player) => player,
            Err(error) => { notice.set_text(&format!("Unable to start video: {error}")); return; }
        };
        let weak_picture = picture.downgrade();
        if let Err(error) = video::init(&window, &player, move || {
            weak_picture.upgrade().map(|p| (p.width().max(1) * p.scale_factor(), p.height().max(1) * p.scale_factor())).unwrap_or((1, 1))
        }) {
            notice.set_text(&format!("Unable to display video: {error}"));
            player.destroy_later();
            return;
        }
        let start = format!("start={}", saved_position.get());
        if let Err(error) = player.command(&["loadfile", &file, "replace", "-1", &start]) {
            video::shutdown();
            player.destroy_later();
            notice.set_text(&format!("Unable to open video: {error}"));
            return;
        }
        notice.set_text("");
        c.active.set(token);
        *c.player.borrow_mut() = Some(player);
    });
    let c = controller.clone();
    let saved_position = position.clone();
    let epoch_on_unmap = map_epoch.clone();
    picture.connect_unmap(move |_| {
        epoch_on_unmap.set(epoch_on_unmap.get().wrapping_add(1));
        if c.active.get() == token {
            if let Some(player) = c.player.borrow().as_ref() {
                saved_position.set(player.get_f64("time-pos").unwrap_or(saved_position.get()));
            }
            c.stop(token);
        }
    });
    for (button, command, amount) in [(&bar.back, "seek", "-5"), (&bar.fwd, "seek", "5"), (&bar.play, "cycle", "pause"), (&bar.mute, "cycle", "mute")] {
        let c = controller.clone();
        button.connect_clicked(move |_| {
            if c.active.get() == token {
                if let Some(player) = c.player.borrow().as_ref() { let _ = player.command(&[command, amount]); }
            }
        });
    }
    let c = controller.clone();
    bar.volume.connect_value_changed(move |volume| {
        if c.active.get() == token {
            if let Some(player) = c.player.borrow().as_ref() { let _ = player.set_f64("volume", volume.value() * 100.0); }
        }
    });
    let c = controller.clone();
    seek.seek.connect_change_value(move |_, _, value| {
        if c.active.get() == token {
            if let Some(player) = c.player.borrow().as_ref() { let _ = player.set_f64("time-pos", value); }
        }
        glib::Propagation::Proceed
    });
    let w = window.downgrade();
    bar.full.connect_clicked(move |_| if let Some(w) = w.upgrade() {
        if w.is_fullscreen() { w.unfullscreen(); } else { w.fullscreen(); }
    });
    let pic = picture.clone();
    bar.close.connect_clicked(move |_| pic.set_visible(false));
    let c = controller.clone();
    let progress = seek.seek.clone();
    let elapsed = seek.pos.clone();
    let duration = seek.dur.clone();
    let play = bar.play.clone();
    let mute = bar.mute.clone();
    let volume = bar.volume.clone();
    picture.connect_map(move |picture| {
        let epoch = map_epoch.get().wrapping_add(1);
        map_epoch.set(epoch);
        let map_epoch = map_epoch.clone();
        let weak_picture = picture.downgrade();
        let c = c.clone();
        let progress = progress.clone();
        let elapsed = elapsed.clone();
        let duration = duration.clone();
        let play = play.clone();
        let mute = mute.clone();
        let volume = volume.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(picture) = weak_picture.upgrade() else { return glib::ControlFlow::Break };
            if !picture.is_mapped() || map_epoch.get() != epoch { return glib::ControlFlow::Break; }
            if c.active.get() == token {
                if let Some(player) = c.player.borrow().as_ref() {
                    while player.next_event().is_some() {}
                    let pos = player.get_f64("time-pos").unwrap_or(0.0).max(0.0);
                    let total = player.get_f64("duration").unwrap_or(0.0).max(0.0);
                    progress.set_range(0.0, total.max(1.0));
                    progress.set_value(pos.min(total.max(1.0)));
                    elapsed.set_text(&controls::time(pos));
                    duration.set_text(&controls::time(total));
                    let paused = player.get_str("pause").is_none_or(|s| s == "yes");
                    play.set_icon_name(if paused { "media-playback-start-symbolic" } else { "media-playback-pause-symbolic" });
                    let muted = player.get_str("mute").is_some_and(|s| s == "yes");
                    mute.set_icon_name(controls::volume_icon(volume.value(), muted));
                }
            }
            glib::ControlFlow::Continue
        });
    });
    let keys = gtk::EventControllerKey::new();
    let c = controller.clone();
    let w = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::f {
            if let Some(w) = w.upgrade() { if w.is_fullscreen() { w.unfullscreen(); } else { w.fullscreen(); } }
            return glib::Propagation::Stop;
        }
        let command = match key {
            gdk::Key::space | gdk::Key::k => Some(("cycle", "pause")),
            gdk::Key::Left => Some(("seek", "-5")),
            gdk::Key::Right => Some(("seek", "5")),
            gdk::Key::m => Some(("cycle", "mute")),
            _ => None,
        };
        if let Some((command, value)) = command {
            if c.active.get() == token {
                if let Some(player) = c.player.borrow().as_ref() { let _ = player.command(&[command, value]); }
            }
            glib::Propagation::Stop
        } else { glib::Propagation::Proceed }
    });
    outer.add_controller(keys);
    outer.upcast()
}

fn line_chart(points: Vec<(f64, f64)>) -> gtk::Widget {
    let area = gtk::DrawingArea::new();
    area.set_content_width(640);
    area.set_content_height(360);
    area.set_hexpand(true);
    area.set_draw_func(move |_, cr, w, h| draw_line_chart(cr, w, h, &points));
    area.upcast()
}

fn draw_line_chart(cr: &gtk::cairo::Context, w: i32, h: i32, points: &[(f64, f64)]) {
        use plotters::prelude::*;
        if points.len() < 2 || w < 100 || h < 100 { return; }
        let Ok(backend) = plotters_cairo::CairoBackend::new(cr, (w as u32, h as u32)) else { return };
        let root = backend.into_drawing_area();
        let (mut xmin, mut xmax) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
        for &(x, y) in points { xmin = xmin.min(x); xmax = xmax.max(x); ymin = ymin.min(y); ymax = ymax.max(y); }
        if xmin == xmax { xmin -= 0.5; xmax += 0.5; }
        if ymin == ymax { ymin -= 0.5; ymax += 0.5; }
        let (r, g, b) = rgb(&nyx_theme::palette().accent);
        let accent = RGBColor((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8);
        let (r, g, b) = rgb(&nyx_theme::palette().text);
        let foreground = RGBColor((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8);
        if let Ok(mut chart) = ChartBuilder::on(&root).margin(12).x_label_area_size(35).y_label_area_size(60)
            .build_cartesian_2d(xmin..xmax, ymin..ymax) {
            let _ = chart.configure_mesh().axis_style(foreground).label_style(("sans-serif", 12).into_font().color(&foreground)).draw();
            let _ = chart.draw_series(LineSeries::new(points.iter().copied(), accent.stroke_width(2)));
        }
        let _ = root.present();
}

fn map_view(lat: f64, lon: f64, zoom: f64) -> gtk::Widget {
    use libshumate::prelude::*;
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 6);
    outer.set_vexpand(true);
    let map = libshumate::SimpleMap::new();
    map.set_size_request(400, 360);
    map.set_hexpand(true);
    map.set_vexpand(true);
    let source = libshumate::RasterRenderer::new_full_from_url(
        "osm-standard", "OpenStreetMap", "© OpenStreetMap contributors",
        "https://www.openstreetmap.org/copyright", 0, 19, 256,
        libshumate::MapProjection::Mercator, "https://tile.openstreetmap.org/{z}/{x}/{y}.png");
    map.set_map_source(Some(&source));
    let status = gtk::Label::new(Some("Loading map…"));
    status.set_xalign(0.0);
    status.set_wrap(true);
    if let Some(layer) = map.base_map_layer() {
        let notice = status.clone();
        layer.connect_tile_error(move |_, _, error| {
            eprintln!("Canvas map tile failed: {error}");
            notice.set_text("Map tiles could not be loaded. Check the network connection.");
        });
        let notice = status.clone();
        layer.connect_map_loaded(move |_, errors| {
            notice.set_text(if errors { "Some map tiles could not be loaded." } else { "" });
        });
    }
    if let Some(viewport) = map.viewport() {
        viewport.set_zoom_level(zoom.clamp(1.0, 19.0));
        viewport.set_location(lat, lon);
        let layer = libshumate::MarkerLayer::new(&viewport);
        let marker = libshumate::Marker::new();
        marker.set_location(lat, lon);
        marker.set_child(Some(&gtk::Image::from_icon_name("mark-location-symbolic")));
        layer.add_marker(&marker);
        map.add_overlay_layer(&layer);
    }
    outer.append(&map);
    outer.append(&status);
    outer.upcast()
}

fn bar_chart(items:&[Value])->gtk::DrawingArea {
    let items:Vec<(String,f64)>=items.iter().take(50).filter_map(|v|Some((v["label"].as_str()?.to_string(),v["value"].as_f64()?))).collect();
    let area=gtk::DrawingArea::new();area.set_content_height((items.len() as i32*34).max(100));area.set_hexpand(true);
    area.set_draw_func(move |_,cr,w,_|{
        let min=items.iter().map(|i|i.1).fold(0.0,f64::min);let max=items.iter().map(|i|i.1).fold(0.0,f64::max);
        let span=(max-min).max(1.0);let left=150.0;let width=(w as f64-left-90.0).max(10.0);let zero=left-min/span*width;
        let palette=nyx_theme::palette();let foreground=rgb(&palette.text);let accent=rgb(&palette.accent);
        cr.set_font_size(13.0);
        for (index,(label,value)) in items.iter().enumerate(){let y=index as f64*34.0+22.0;
            cr.set_source_rgb(foreground.0,foreground.1,foreground.2);cr.move_to(0.0,y);let _=cr.show_text(&label.chars().take(20).collect::<String>());
            let end=left+(*value-min)/span*width;cr.set_source_rgba(accent.0,accent.1,accent.2,0.8);cr.rectangle(zero.min(end),y-15.0,(end-zero).abs().max(1.0),20.0);let _=cr.fill();
            cr.set_source_rgb(foreground.0,foreground.1,foreground.2);cr.move_to(left+width+8.0,y);let _=cr.show_text(&value.to_string());
        }
        cr.set_source_rgba(foreground.0,foreground.1,foreground.2,0.5);cr.move_to(zero,0.0);cr.line_to(zero,items.len() as f64*34.0+6.0);let _=cr.stroke();
    });area
}

struct MediaFrame {w:i32,h:i32,stride:usize,count:u32,bytes:Vec<u8>}
impl MediaFrame {
    fn texture(self)->gdk::MemoryTexture {
        gdk::MemoryTexture::new(self.w,self.h,gdk::MemoryFormat::B8g8r8a8Premultiplied,&glib::Bytes::from_owned(self.bytes),self.stride)
    }
}
static MEDIA_SLOTS: Mutex<usize> = Mutex::new(0);
static MEDIA_READY: std::sync::Condvar = std::sync::Condvar::new();
struct MediaSlot;
impl Drop for MediaSlot {fn drop(&mut self){let mut slots=MEDIA_SLOTS.lock().unwrap_or_else(|e|e.into_inner());*slots=slots.saturating_sub(1);MEDIA_READY.notify_one();}}
fn media_job(path:&str,kind:&str,page:u32)->Result<MediaFrame,String> {
    let mut slots=MEDIA_SLOTS.lock().unwrap_or_else(|e|e.into_inner());
    while *slots>=2 {slots=MEDIA_READY.wait(slots).unwrap_or_else(|e|e.into_inner());}
    *slots+=1;drop(slots);let _slot=MediaSlot;
    // Rendering runs outside GTK and is killed on timeout; output is owner-only and bounded.
    let output=tempfile::NamedTempFile::new().map_err(|e|e.to_string())?;
    let mut child=std::process::Command::new(std::env::current_exe().map_err(|e|e.to_string())?)
        .args(["--canvas-media",kind,path,&page.to_string()]).arg(output.path())
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn().map_err(|e|e.to_string())?;
    let start=Instant::now();
    loop {
        match child.try_wait(){Ok(Some(status)) if status.success()=>break,Ok(Some(_))=>return Err("Media could not be loaded".into()),Err(e)=>{let _=child.kill();let _=child.wait();return Err(e.to_string())},_=>{}}
        if start.elapsed()>Duration::from_secs(10){let _=child.kill();let _=child.wait();return Err("Media loading timed out".into());}
        std::thread::sleep(Duration::from_millis(20));
    }
    let file=std::fs::File::open(output.path()).map_err(|e|e.to_string())?;
    let mut reader=BufReader::new(file);let mut header=String::new();reader.by_ref().take(1024).read_line(&mut header).map_err(|e|e.to_string())?;
    let v:Value=serde_json::from_str(&header).map_err(|e|e.to_string())?;
    let w=v["w"].as_i64().ok_or("width")? as i32;let h=v["h"].as_i64().ok_or("height")? as i32;
    if w<=0||h<=0||w>1600||h>1600{return Err("Media dimensions exceed limits".into());}
    let stride=v["stride"].as_u64().ok_or("stride")? as usize;
    if stride<w as usize*4||stride>w as usize*4+64{return Err("Media stride exceeds limits".into());}
    let mut bytes=Vec::new();reader.take((stride*h as usize+1) as u64).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    if bytes.len()!=stride*h as usize{return Err("Media frame malformed".into());}
    Ok(MediaFrame{w,h,stride,count:v["count"].as_u64().unwrap_or(1) as u32,bytes})
}
fn media_child(args:&[String])->Result<(),Box<dyn std::error::Error>> {
    if args.len()!=6{return Err("Invalid media arguments".into());}
    // Keep malformed media from exhausting the assistant or desktop.
    unsafe {
        let memory=libc::rlimit{rlim_cur:2*1024*1024*1024,rlim_max:2*1024*1024*1024};libc::setrlimit(libc::RLIMIT_AS,&memory);
        let cpu=libc::rlimit{rlim_cur:8,rlim_max:8};libc::setrlimit(libc::RLIMIT_CPU,&cpu);
        let file=libc::rlimit{rlim_cur:16*1024*1024,rlim_max:16*1024*1024};libc::setrlimit(libc::RLIMIT_FSIZE,&file);
    }
    let kind=&args[2];let path=&args[3];let index:u32=args[4].parse()?;
    let (w,h,stride,count,bytes)=if kind=="pdf" {
        let uri=gtk::gio::File::for_path(path).uri();let doc=poppler::Document::from_file(&uri,None)?;
        let page=doc.page(index as i32).ok_or("No such PDF page")?;let (pw,ph)=page.size();
        if !pw.is_finite()||!ph.is_finite()||pw<=0.0||ph<=0.0{return Err("Invalid page size".into());}
        let scale=(1200.0/pw).min(1500.0/ph);let w=(pw*scale).ceil().max(1.0) as i32;let h=(ph*scale).ceil().max(1.0) as i32;
        let mut surface=gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32,w,h)?;
        {let cr=gtk::cairo::Context::new(&surface)?;cr.set_source_rgb(1.0,1.0,1.0);cr.paint()?;cr.scale(scale,scale);page.render(&cr);}
        surface.flush();let stride=surface.stride() as usize;let bytes=surface.data()?.to_vec();(w,h,stride,doc.n_pages() as u32,bytes)
    }else{
        let (_,w,h)=gtk::gdk_pixbuf::Pixbuf::file_info(path).ok_or("Invalid image")?;
        if w<=0||h<=0||w as u64*h as u64>40_000_000{return Err("Image exceeds 40 megapixels".into());}
        let pixbuf=gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path,1200,1200,true)?;
        let (w,h)=(pixbuf.width(),pixbuf.height());let channels=pixbuf.n_channels() as usize;let stride=pixbuf.rowstride() as usize;
        let pixels=pixbuf.read_pixel_bytes();let input=pixels.as_ref();let mut bytes=Vec::with_capacity(w as usize*h as usize*4);
        for y in 0..h as usize {for x in 0..w as usize {let i=y*stride+x*channels;let alpha=if channels==4{input[i+3]}else{255};for c in [2,1,0]{bytes.push((input[i+c] as u16*alpha as u16/255) as u8);}bytes.push(alpha);}}
        (w,h,w as usize*4,1,bytes)
    };
    let mut file=std::fs::OpenOptions::new().write(true).truncate(true).open(&args[5])?;
    writeln!(file,"{}",serde_json::json!({"w":w,"h":h,"stride":stride,"count":count}))?;file.write_all(&bytes)?;Ok(())
}
fn image_view(path:&str)->gtk::Picture {
    let picture=gtk::Picture::new();let path=path.to_string();let weak=picture.downgrade();
    glib::spawn_future_local(async move {
        let result=gtk::gio::spawn_blocking(move ||media_job(&path,"image",0)).await;
        if let Some(picture)=weak.upgrade(){match result.ok().and_then(Result::ok){Some(frame)=>{
                picture.set_paintable(Some(&frame.texture()));
            },None=>picture.set_tooltip_text(Some("Unable to load image within the resource limits."))}}
    });picture
}

fn pdf_view(path:&str)->gtk::Widget {
    let outer=gtk::Box::new(gtk::Orientation::Vertical,8);
    let controls=gtk::Box::new(gtk::Orientation::Horizontal,12);
    let previous=gtk::Button::from_icon_name("go-previous-symbolic");let next=gtk::Button::from_icon_name("go-next-symbolic");
    let status=gtk::Label::new(Some("Loading PDF…"));let picture=gtk::Picture::new();picture.set_can_shrink(true);picture.set_vexpand(true);
    controls.append(&previous);controls.append(&status);controls.append(&next);outer.append(&controls);outer.append(&picture);
    let page=Rc::new(Cell::new(0u32));let epoch=Rc::new(Cell::new(0u64));
    let cache:Rc<RefCell<HashMap<u32,(gdk::MemoryTexture,u32)>>>=Rc::new(RefCell::new(HashMap::new()));
    let load:Rc<dyn Fn(u32)>=Rc::new({let path=path.to_string();let picture=picture.clone();let status=status.clone();let previous=previous.clone();let next=next.clone();move |index| {
        let token=epoch.get().wrapping_add(1);epoch.set(token);previous.set_sensitive(false);next.set_sensitive(false);
        if let Some((texture,count))=cache.borrow().get(&index){picture.set_paintable(Some(texture));status.set_text(&format!("Page {} of {count}",index+1));previous.set_sensitive(index>0);next.set_sensitive(index+1<*count);return;}
        status.set_text("Loading PDF…");
        let (path,picture,status,previous,next,epoch,cache)=(path.clone(),picture.clone(),status.clone(),previous.clone(),next.clone(),epoch.clone(),cache.clone());
        glib::spawn_future_local(async move {
            let result=gtk::gio::spawn_blocking(move || media_job(&path,"pdf",index)).await;
            if epoch.get()!=token{return;}
            match result.ok().and_then(Result::ok) {
                Some(frame)=>{let count=frame.count;let texture=frame.texture();picture.set_paintable(Some(&texture));status.set_text(&format!("Page {} of {count}",index+1));previous.set_sensitive(index>0);next.set_sensitive(index+1<count);
                    // Keep a small rendered-page cache, not the full document in GTK.
                    let mut cached=cache.borrow_mut();if cached.len()>=3 {cached.clear();}cached.insert(index,(texture,count));},
                None=>status.set_text("Unable to render PDF within the resource limits."),
            }
        });
    }});
    for (button,delta) in [(&previous,-1i32),(&next,1i32)] {let page=page.clone();let load=load.clone();button.connect_clicked(move |_|{let index=(page.get() as i32+delta).max(0) as u32;page.set(index);load(index);});}
    load(0);outer.upcast()
}

#[derive(Clone)]
struct EventSink {tx:mpsc::SyncSender<Value>,wake:Arc<Mutex<UnixStream>>}
impl EventSink {
    fn send(&self,event:Value)->Result<(),()> {
        self.tx.send(event).map_err(|_|())?;
        let _=self.wake.lock().unwrap().write(&[1]);Ok(())
    }
}
fn monitor_events(sink:EventSink) {
    std::thread::spawn(move || {
        let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().expect("monitor runtime");
        loop {
            if let Some(snapshot)=Aura::monitor_state(&runtime) {if sink.send(serde_json::json!({"type":"monitor_snapshot","snapshot":snapshot})).is_err(){break;}}
            let stream=assistant_omarchy::hypr::socket_path().ok().and_then(|p|UnixStream::connect(p.with_file_name(".socket2.sock")).ok());
            let Some(stream)=stream else {std::thread::sleep(Duration::from_secs(5));continue;};
            let _=stream.set_read_timeout(Some(Duration::from_secs(60)));let mut reader=BufReader::new(stream);
            loop {
                let mut line=String::new();let read=reader.read_line(&mut line);
                if matches!(&read,Ok(0)){break;}
                let reconcile=read.as_ref().err().is_some_and(|e|matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut));
                if read.is_err() && !reconcile{break;}
                if reconcile || ["focusedmon>>","monitoradded","monitorremoved","fullscreen>>","workspace>>","moveworkspace","openlayer>>","closelayer>>","configreloaded>>"].iter().any(|p|line.starts_with(p)) {
                    if let Some(snapshot)=Aura::monitor_state(&runtime){if sink.send(serde_json::json!({"type":"monitor_snapshot","snapshot":snapshot})).is_err(){return;}}
                }
            }
        }
    });
}


fn listen(tx: EventSink) {
    std::thread::spawn(move || loop {
        let socket = assistant_core::paths::socket_path();
        if let Ok(mut stream) = UnixStream::connect(&socket) {
            let _=stream.write_all(b"{\"type\":\"request\",\"request_id\":\"visual-sync\",\"action\":{\"type\":\"status\"}}\n");
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
    if event["type"]=="result" && event["request_id"]=="visual-sync" && event["ok"]==true {
        let snapshot=&event["data"];
        handle_event(serde_json::json!({"type":"settings","running":snapshot["running_settings"]}),aura,canvas,app);
        handle_event(serde_json::json!({"type":"state","state":snapshot["state"],"at_ms":snapshot["snapshot_at_ms"]}),aura,canvas,app);
        handle_event(serde_json::json!({"type":"session","mode":snapshot["session"]["mode"],"remaining_secs":snapshot["session"]["remaining_secs"]}),aura,canvas,app);
        if snapshot["pending_approvals"].as_object().is_some_and(|a|!a.is_empty()) {aura.set_state(State::Attention);}
        return;
    }
    if event["type"]=="monitor_snapshot" {
        if let Ok(snapshot)=serde_json::from_value::<MonitorSnapshot>(event["snapshot"].clone()) {
            *canvas.active_monitor.borrow_mut()=snapshot.1.clone();aura.refresh_monitors(app,snapshot);
        }
        return;
    }
    match event["type"].as_str().unwrap_or("") {
        "state" => {
            let at=event["at_ms"].as_u64().unwrap_or(0);
            if at>0 && at<aura.last_state_ms.get(){return;}
            if at>0 {aura.last_state_ms.set(at);}
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
            }
        }
        "session" => {
            aura.conversation.set(matches!(event["mode"].as_str(), Some("conversation" | "follow_up"))
                && event["remaining_secs"].as_u64().unwrap_or(0) > 0);
            aura.redraw();
        }
        "error" => aura.set_state(State::Error),
        "proactive" => aura.set_state(State::Attention),
        "level" if aura.base_state.get() == State::Listening => {
            aura.level_target.set(meter_fraction(event["rms"].as_f64().unwrap_or(0.0),aura.level_target.get()));
            aura.last_level.set(Instant::now());
        }
        "playback_level" if aura.base_state.get() == State::Speaking => {
            aura.level_target.set(meter_fraction(event["rms"].as_f64().unwrap_or(0.0),aura.level_target.get()));
            aura.last_level.set(Instant::now());
        }
        "turn" if event["event"]["type"] == "tool" && aura.cfg.borrow().canvas_enabled => {
            let tool = event["event"]["tool"].as_str().unwrap_or("");
            let data = &event["event"]["result"]["structuredContent"];
            if tool == "canvas.show" && event["event"]["result"]["isError"] == false {
                canvas.show_content(&data["canvas"]);
            }
        }
        "assistant_text" if event["done"] == true => aura.set_state(State::Done),
        _ => {}
    }
}

fn poll_events(rx: &Receiver<Value>, aura: &Rc<Aura>, canvas: &Canvas, app: &gtk::Application) {
    for event in rx.try_iter() {
        handle_event(event, aura, canvas, app);
    }
}

fn main() {
    let args:Vec<_>=std::env::args().collect();
    if args.get(1).is_some_and(|s|s=="--canvas-media") {
        if media_child(&args).is_err(){std::process::exit(1);}return;
    }
    let config = Config::load().unwrap_or_default();
    let app = gtk::Application::new(Some("io.github.design_nexus.nyx.assistant.visual"), Default::default());
    app.connect_activate(move |app| {
        if !config.ui.enabled {
            return;
        }
        let system_reduce_motion = gtk::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations());
        nyx_theme::use_design_2();
        nyx_playback::controls::install_icons();
        nyx_theme::install(nyx_theme::Style {
            window_class: "assistant-canvas",
            base_skip: &[],
            css: nyx_playback::controls::stylesheet(CSS),
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
        if let Some(settings)=gtk::Settings::default() {
            let weak=Rc::downgrade(&aura);
            settings.connect_gtk_enable_animations_notify(move |settings| {
                if let Some(aura)=weak.upgrade(){aura.system_reduce_motion.set(!settings.is_gtk_enable_animations());aura.redraw();}
            });
        }
        let canvas = Canvas::new(app);
        let (wake_read,wake_write)=UnixStream::pair().expect("visual wake socket");
        wake_read.set_nonblocking(true).expect("nonblocking visual wake");
        wake_write.set_nonblocking(true).expect("nonblocking visual wake");
        let (tx,rx)=mpsc::sync_channel(512);
        let sink=EventSink{tx,wake:Arc::new(Mutex::new(wake_write))};
        listen(sink.clone());
        monitor_events(sink);
        let aura_for_tick=aura.clone(); let app_for_tick=app.clone();
        let wake_socket=gtk::gio::Socket::from_fd(wake_read.into()).expect("visual wake socket");
        wake_socket.set_blocking(false);
        let source=gtk::gio::prelude::SocketExtManual::create_source(&wake_socket,glib::IOCondition::IN,None::<&gtk::gio::Cancellable>,Some("assistant-events"),glib::Priority::DEFAULT,move |socket,_|{
            let _=&hold;
            let mut bytes=[0u8;512];while socket.receive(&mut bytes,None::<&gtk::gio::Cancellable>).is_ok_and(|n|n>0){}
            poll_events(&rx,&aura_for_tick,&canvas,&app_for_tick);
            glib::ControlFlow::Continue
        });
        source.attach(None);
        if let Some(display) = gdk::Display::default() {
            let aura = aura.clone();
            let app = app.clone();
            display.monitors().connect_items_changed(move |_, _, _, _| {
                aura.rebuild(&app);
            });
        }
    });
    app.run();
}
