//! Synthetic input, without root: Hyprland moves the cursor (layout coordinates), a Wayland
//! virtual pointer presses buttons and scrolls, and `wtype` (virtual keyboard) types.
//!
//! Every press is paired with its release, including when a call fails or is cancelled
//! halfway (the pointer releases anything still held when it is dropped).

use anyhow::{bail, ensure, Context, Result};
use assistant_omarchy::hypr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

impl Button {
    fn code(self) -> u32 {
        match self {
            Self::Left => BTN_LEFT,
            Self::Right => BTN_RIGHT,
        }
    }
}

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwlrVirtualPointerManagerV1, _: <ZwlrVirtualPointerManagerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwlrVirtualPointerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwlrVirtualPointerV1, _: <ZwlrVirtualPointerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

fn now_ms() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u32).unwrap_or(0)
}

/// A virtual pointer on the compositor's seat. Blocking: use from `spawn_blocking`.
struct Pointer {
    conn: Connection,
    queue: EventQueue<State>,
    ptr: ZwlrVirtualPointerV1,
    held: Option<u32>,
}

impl Pointer {
    fn new() -> Result<Self> {
        let conn = Connection::connect_to_env().context("no Wayland display")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).context("Wayland registry")?;
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).context("no Wayland seat")?;
        let manager: ZwlrVirtualPointerManagerV1 =
            globals.bind(&qh, 1..=2, ()).context("the compositor has no virtual pointer support")?;
        let ptr = manager.create_virtual_pointer(Some(&seat), &qh, ());
        queue.roundtrip(&mut State).context("Wayland roundtrip")?;
        Ok(Self { conn, queue, ptr, held: None })
    }

    fn flush(&mut self) -> Result<()> {
        self.ptr.frame();
        self.conn.flush().context("Wayland flush")?;
        self.queue.roundtrip(&mut State).context("Wayland roundtrip")?;
        Ok(())
    }

    fn button(&mut self, code: u32, pressed: bool) -> Result<()> {
        let state = if pressed { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
        self.ptr.button(now_ms(), code, state);
        self.held = if pressed { Some(code) } else { None };
        self.flush()
    }

    fn scroll(&mut self, steps: i32) -> Result<()> {
        // One wheel notch is 15 units in libinput's convention.
        self.ptr.axis_source(wl_pointer::AxisSource::Wheel);
        self.ptr.axis_discrete(now_ms(), wl_pointer::Axis::VerticalScroll, 15.0 * steps as f64, steps);
        self.flush()
    }
}

impl Drop for Pointer {
    fn drop(&mut self) {
        if let Some(code) = self.held.take() {
            let _ = self.button(code, false);
        }
        self.ptr.destroy();
        let _ = self.conn.flush();
    }
}

/// Whether synthetic input can work here (a virtual pointer can be created); presses nothing.
pub async fn probe() -> Result<()> {
    tokio::task::spawn_blocking(|| Pointer::new().map(drop)).await?
}

/// Move the cursor to a layout position.
pub async fn move_to(x: i32, y: i32) -> Result<()> {
    hypr::move_cursor(x, y).await
}

/// Click at a layout position: move, a short settle for hover effects, press, release.
pub async fn click(x: i32, y: i32, button: Button, double: bool) -> Result<()> {
    move_to(x, y).await?;
    tokio::time::sleep(Duration::from_millis(60)).await;
    let code = button.code();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut p = Pointer::new()?;
        for _ in 0..if double { 2 } else { 1 } {
            p.button(code, true)?;
            std::thread::sleep(Duration::from_millis(35));
            p.button(code, false)?;
            std::thread::sleep(Duration::from_millis(60));
        }
        Ok(())
    })
    .await?
}

/// Scroll at a layout position; positive steps scroll down.
pub async fn scroll(x: i32, y: i32, steps: i32) -> Result<()> {
    ensure!((-30..=30).contains(&steps) && steps != 0, "scroll 1–30 steps up or down");
    move_to(x, y).await?;
    tokio::time::sleep(Duration::from_millis(40)).await;
    tokio::task::spawn_blocking(move || Pointer::new()?.scroll(steps)).await?
}

async fn wtype(args: &[String]) -> Result<()> {
    let out = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new("wtype").args(args).kill_on_drop(true).output(),
    )
    .await
    .context("wtype timed out")?
    .context("wtype isn't available")?;
    ensure!(out.status.success(), "wtype failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

/// `wtype` arguments that release every modifier a combo can press: a `wtype` killed
/// between `-M` and `-m` can leave one held.
pub fn release_args() -> Vec<String> {
    let mut names: Vec<&str> = MODIFIERS.iter().map(|(_, name)| *name).collect();
    names.sort_unstable();
    names.dedup();
    names.into_iter().flat_map(|m| ["-m".to_string(), m.to_string()]).collect()
}

/// After an emergency stop: let go of any modifier still held. Pointer buttons need nothing
/// here: each click's press and release run together on a blocking thread that finishes
/// even when the call is cancelled, and a dropped pointer releases what it held.
pub async fn release_all() -> Result<()> {
    wtype(&release_args()).await
}

/// Type text into the focused window.
pub async fn type_text(text: &str) -> Result<()> {
    ensure!(!text.is_empty() && text.chars().count() <= 2000, "type 1–2000 characters");
    wtype(&["--".into(), text.into()]).await
}

const MODIFIERS: &[(&str, &str)] = &[
    ("ctrl", "ctrl"),
    ("control", "ctrl"),
    ("shift", "shift"),
    ("alt", "alt"),
    ("super", "logo"),
    ("meta", "logo"),
    ("win", "logo"),
    ("logo", "logo"),
    ("altgr", "altgr"),
];

/// Key names people say, as XKB keysym names.
fn keysym(key: &str) -> Option<String> {
    let named = match key.to_lowercase().as_str() {
        "enter" | "return" => "Return",
        "tab" => "Tab",
        "esc" | "escape" => "Escape",
        "space" => "space",
        "backspace" => "BackSpace",
        "delete" | "del" => "Delete",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" | "page_up" => "Prior",
        "pagedown" | "page_down" => "Next",
        k if k.len() >= 2 && k.starts_with('f') && k[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) => return Some(key.to_uppercase()),
        k if k.chars().count() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric() || ",./;'[]-=`".contains(c)) => {
            return Some(match k {
                "," => "comma",
                "." => "period",
                "/" => "slash",
                ";" => "semicolon",
                "'" => "apostrophe",
                "[" => "bracketleft",
                "]" => "bracketright",
                "-" => "minus",
                "=" => "equal",
                "`" => "grave",
                other => other,
            }
            .to_string())
        }
        _ => return None,
    };
    Some(named.to_string())
}

/// `ctrl+s`, `alt+F4`, `enter`: as wtype arguments.
pub fn combo_args(combo: &str) -> Result<Vec<String>> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let Some((key, mods)) = parts.split_last() else { bail!("no key given") };
    let mut args = Vec::new();
    let mut held = Vec::new();
    for m in mods {
        let Some((_, name)) = MODIFIERS.iter().find(|(said, _)| said.eq_ignore_ascii_case(m)) else { bail!("unknown modifier “{m}”") };
        args.extend(["-M".to_string(), name.to_string()]);
        held.push(*name);
    }
    let sym = keysym(key).with_context(|| format!("unknown key “{key}”"))?;
    args.extend(["-k".to_string(), sym]);
    for name in held.iter().rev() {
        args.extend(["-m".to_string(), name.to_string()]);
    }
    Ok(args)
}

/// `ctrl+s` as Hyprland's modifiers and keysym (`("CTRL", "s")`); `None` without a modifier.
pub fn shortcut(combo: &str) -> Result<Option<(String, String)>> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let Some((key, mods)) = parts.split_last() else { bail!("no key given") };
    if mods.is_empty() {
        return Ok(None);
    }
    let mut names: Vec<&str> = Vec::new();
    for m in mods {
        let Some((_, name)) = MODIFIERS.iter().find(|(said, _)| said.eq_ignore_ascii_case(m)) else { bail!("unknown modifier “{m}”") };
        let hypr = match *name {
            "ctrl" => "CTRL",
            "shift" => "SHIFT",
            "alt" => "ALT",
            _ => "SUPER",
        };
        if !names.contains(&hypr) {
            names.push(hypr);
        }
    }
    let sym = keysym(key).with_context(|| format!("unknown key “{key}”"))?;
    Ok(Some((names.join(" "), sym)))
}

/// Press a key combination in the focused window. A letter or digit shortcut goes to `window`
/// (a Hyprland address) through Hyprland: Electron apps such as VS Code drop a virtual
/// keyboard's modifiers for those, so wtype's ctrl+s never saves there.
pub async fn key(combo: &str, window: Option<&str>) -> Result<()> {
    match (shortcut(combo)?, window) {
        // Only letter and digit shortcuts: Electron drops wtype's modifiers there (its
        // accelerators read the keymap), while ctrl+End and the like work through wtype and
        // not through Hyprland.
        (Some((mods, sym)), Some(address)) if sym.len() == 1 && sym.chars().all(|c| c.is_ascii_alphanumeric()) => {
            let address = address.to_string();
            // Down, then up shortly after (as Omarchy's own shortcuts do). The release runs on
            // its own task, so a stop in between can't leave the key held.
            hypr::send_key_state(&address, &mods, &sym, true).await?;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                hypr::send_key_state(&address, &mods, &sym, false).await
            })
            .await
            .context("release the key")?
        }
        _ => wtype(&combo_args(combo)?).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_press_and_release_in_order() {
        assert_eq!(combo_args("ctrl+s").unwrap(), ["-M", "ctrl", "-k", "s", "-m", "ctrl"]);
        assert_eq!(combo_args("Ctrl+Shift+Tab").unwrap(), ["-M", "ctrl", "-M", "shift", "-k", "Tab", "-m", "shift", "-m", "ctrl"]);
        assert_eq!(combo_args("enter").unwrap(), ["-k", "Return"]);
        assert_eq!(combo_args("alt+F4").unwrap(), ["-M", "alt", "-k", "F4", "-m", "alt"]);
        assert_eq!(combo_args("ctrl+,").unwrap(), ["-M", "ctrl", "-k", "comma", "-m", "ctrl"]);
        assert!(combo_args("hyper+x").is_err());
        assert!(combo_args("ctrl+notakey").is_err());
        // Every modifier pressed is released.
        for c in ["ctrl+alt+delete", "super+shift+q", "shift+pagedown"] {
            let a = combo_args(c).unwrap();
            assert_eq!(a.iter().filter(|x| *x == "-M").count(), a.iter().filter(|x| *x == "-m").count(), "{c}");
        }
    }

    #[test]
    fn shortcuts_name_hyprland_modifiers() {
        assert_eq!(shortcut("ctrl+s").unwrap(), Some(("CTRL".into(), "s".into())));
        assert_eq!(shortcut("Control+Shift+End").unwrap(), Some(("CTRL SHIFT".into(), "End".into())));
        assert_eq!(shortcut("super+q").unwrap(), Some(("SUPER".into(), "q".into())));
        assert_eq!(shortcut("enter").unwrap(), None);
        assert!(shortcut("hyper+x").is_err());
    }

    #[test]
    fn release_lets_go_of_every_modifier() {
        let a = release_args();
        assert!(!a.contains(&"-M".to_string()), "never presses");
        for m in ["shift", "ctrl", "alt", "logo"] {
            assert!(a.windows(2).any(|w| w[0] == "-m" && w[1] == m), "{m}");
        }
        // Every modifier a combo can press is one the stop releases.
        for combo in ["ctrl+shift+alt+super+x", "Ctrl+Alt+Delete"] {
            let pressed: Vec<_> = combo_args(combo).unwrap().windows(2).filter(|w| w[0] == "-M").map(|w| w[1].clone()).collect();
            for m in pressed {
                assert!(a.contains(&m), "{combo}: {m}");
            }
        }
    }
}
