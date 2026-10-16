//! Render and interaction checks on a populated fixture: every screen and pane at the four
//! sizes, the shared keys, overlays owning input, mouse targets. `AURORA_RENDER_DIR=dir`
//! also writes SVG captures for visual review.

use super::draw;
use crate::app::{App, EditTarget, Tab};
use crate::daemon::DaemonStatus;
use crate::input::SCREENS;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;

/// The assistant's folders point at a temporary place for the whole test run.
fn isolate() {
    static ONCE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        for (var, sub) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "run"),
        ] {
            let p = dir.join(sub);
            std::fs::create_dir_all(&p).unwrap();
            // SAFETY: set once, before any test reads the environment.
            unsafe { std::env::set_var(var, p) };
        }
        dir
    });
}

fn fixture() -> App {
    isolate();
    let mut app = App::new("aurora".into(), None).unwrap();
    app.installed_models = vec![
        "/models/gemma-4-E4B-it-Q4_K_M.gguf".into(),
        "/models/Qwen3.5-9B-Q4_K_M.gguf".into(),
        "/models/GLM-4.6V-Flash-Q4_K_M.gguf".into(),
    ];
    app.config.llm.model = "/models/gemma-4-E4B-it-Q4_K_M.gguf".into();
    app.service_active = true;
    app.daemon_status = Some(DaemonStatus {
        ok: true,
        state: "idle".into(),
        name: "Echo".into(),
        model_loaded: true,
        session: json!({"mode": "wake", "remaining_secs": 0}),
        plugins: json!([]),
        mcp_tools: json!([]),
        ..Default::default()
    });
    app.activity = json!({"turns": [
        {"turn_id": 1, "at_ms": 1_792_100_000_000u64, "events": [
            {"type": "transcript", "text": "What's the weather in Paris?"},
            {"type": "assistant_text", "text": "Sunny, 21 degrees.", "done": true},
            {"type": "timing", "stages": [{"name": "STT complete", "ms": 210}, {"name": "audio playback", "ms": 820}], "total_ms": 820}
        ]},
        {"turn_id": 2, "at_ms": 1_792_100_060_000u64, "events": [
            {"type": "transcript", "text": "Open a new tab in Notepad"},
            {"type": "error", "message": "approval denied"}
        ]}
    ]});
    app.hf_query = "gemma".into();
    app.hf_results =
        vec![crate::models::HfModelEntry { id: "lmstudio-community/gemma-4-E4B-it-GGUF".into(), downloads: 12000, likes: 40 }];
    app.diagnostics = json!({"ok": true, "llm": {"loaded": true}, "audio": {"sink": "default"}});
    app
}

fn render(app: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn click(x: u16, y: u16) -> MouseEvent {
    MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE }
}

#[tokio::test]
async fn every_screen_and_pane_fits_the_size_matrix() {
    let mut app = fixture();
    for screen in 0..SCREENS.len() {
        app.select_screen(screen);
        for pane in 0..app.pane_names().len() {
            app.focus_pane(pane);
            for &(w, h) in &crate::audit::SIZES[..4] {
                let buffer = render(&mut app, w, h);
                let text = crate::audit::text(&buffer);
                assert!(text.contains("(F2)"), "screen {screen} pane {pane} at {w}x{h} must show Navigate");
                assert!(text.contains("Focus:"), "screen {screen} pane {pane} at {w}x{h} must name the focus");
                assert!(buffer.content.iter().any(|c| matches!(c.symbol(), "█" | "▀" | "▄")), "pixel title at {w}x{h}");
                crate::audit::verify_footer(&buffer);
                crate::audit::capture("assistant", &format!("{}-{}", SCREENS[screen].name.to_lowercase(), pane), &buffer);
            }
        }
    }
    // Tiny and odd sizes don't panic, and growing back works on one terminal.
    let mut resizing = Terminal::new(TestBackend::new(120, 40)).unwrap();
    for &(w, h) in &[(120, 40), (60, 20), (30, 10), (1, 1), (80, 24), (20, 8), (100, 30), (120, 34)] {
        resizing.backend_mut().resize(w, h);
        resizing.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(resizing.backend().buffer().area.width, w);
    }
}

#[tokio::test]
async fn shared_keys_move_between_screens_and_panes() {
    let mut app = fixture();
    assert_eq!(app.active_tab, Tab::Dashboard);
    app.on_key(key(KeyCode::Tab));
    assert_eq!(app.active_tab, Tab::Models);
    app.on_key(key(KeyCode::BackTab));
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.active_tab, Tab::Plugins, "wraps");
    app.on_key(key(KeyCode::Char('4')));
    assert_eq!(app.active_tab, Tab::Persona);
    app.on_key(key(KeyCode::F(6)));
    assert_eq!(app.focused_pane(), 1);
    app.on_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::SHIFT));
    assert_eq!(app.focused_pane(), 0);
    // Focus is remembered per screen.
    app.on_key(key(KeyCode::F(6)));
    app.on_key(key(KeyCode::Char('1')));
    app.on_key(key(KeyCode::Char('4')));
    assert_eq!(app.focused_pane(), 1);
    // Esc leaves a pane but never quits.
    assert!(!app.on_key(key(KeyCode::Esc)));
    assert_eq!(app.focused_pane(), 0);
    assert!(!app.on_key(key(KeyCode::Esc)));
    // F2 and Ctrl+K open, Esc closes.
    app.on_key(key(KeyCode::F(2)));
    assert!(app.navigation.open);
    app.on_key(key(KeyCode::Esc));
    assert!(!app.navigation.open);
    app.on_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert!(app.commands.open);
    let buffer = render(&mut app, 100, 30);
    assert!(crate::audit::text(&buffer).contains("Actions"));
    crate::audit::capture("assistant", "actions", &buffer);
    app.on_key(key(KeyCode::Esc));
    assert!(!app.commands.open);
    // Help opens with ? and closes with Esc.
    app.on_key(key(KeyCode::Char('?')));
    assert!(app.help);
    crate::audit::capture("assistant", "help", &render(&mut app, 100, 30));
    app.on_key(key(KeyCode::Esc));
    assert!(!app.help);
    // q quits when nothing is pending.
    assert!(app.on_key(key(KeyCode::Char('q'))));
}

#[tokio::test]
async fn an_unsaved_draft_asks_before_quitting() {
    let mut app = fixture();
    app.changed();
    assert!(!app.on_key(key(KeyCode::Char('q'))));
    assert!(app.quit_confirm);
    let buffer = render(&mut app, 80, 24);
    assert!(crate::audit::text(&buffer).contains("Discard"));
    crate::audit::capture("assistant", "quit", &buffer);
    assert!(!app.on_key(key(KeyCode::Esc)));
    assert!(!app.quit_confirm, "Esc cancels");
    app.on_key(key(KeyCode::Char('q')));
    assert!(app.on_key(key(KeyCode::Enter)), "Enter confirms");
}

#[tokio::test]
async fn text_input_owns_digits_letters_and_tab() {
    let mut app = fixture();
    app.on_key(key(KeyCode::Char('4')));
    app.on_key(key(KeyCode::Char('n')));
    assert_eq!(app.edit_target, EditTarget::PersonaCreateName);
    for c in "q1?".chars() {
        assert!(!app.on_key(key(KeyCode::Char(c))));
    }
    assert_eq!(app.edit_buffer, "q1?");
    assert_eq!(app.active_tab, Tab::Persona, "digits typed, not screens");
    crate::audit::capture("assistant", "edit", &render(&mut app, 80, 24));
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.edit_target, EditTarget::None);
    assert!(app.edit_buffer.is_empty(), "cancel applies nothing");
    assert!(!app.is_dirty);
}

#[tokio::test]
async fn disabled_actions_say_why_and_do_nothing() {
    let mut app = fixture();
    app.on_key(key(KeyCode::Char('3')));
    // Cancel download with nothing downloading.
    app.on_key(key(KeyCode::Char('c')));
    assert!(app.toast.as_ref().is_some_and(|(t, _)| t.contains("No download")));
    let actions = app.command_actions();
    let cancel = actions.iter().find(|a| a.label == "Cancel download").unwrap();
    assert!(cancel.disabled.is_some());
}

#[tokio::test]
async fn approvals_are_a_dialog_that_owns_input() {
    let mut app = fixture();
    app.daemon_status.as_mut().unwrap().pending_approvals.insert(
        "plugin-1-1".into(),
        json!({"tool": "files.write", "summary": "Add to ~/.bashrc", "arguments": {"path": "~/.bashrc", "content": "x", "mode": "append"},
               "expires_at_ms": chrono::Utc::now().timestamp_millis() as u64 + 42_000, "task_scope_allowed": false, "voice_allowed": false}),
    );
    let buffer = render(&mut app, 100, 30);
    let text = crate::audit::text(&buffer);
    assert!(text.contains("Approval needed") && text.contains("Add to ~/.bashrc"));
    assert!(text.contains("doesn't count"), "key-press approvals say so");
    assert!(text.contains("expires in 4"), "a live countdown");
    assert!(!text.contains("allow for this task"), "only when offered");
    crate::audit::capture("assistant", "approval", &buffer);
    // Screen keys don't reach the background.
    app.on_key(key(KeyCode::Char('3')));
    assert_eq!(app.active_tab, Tab::Dashboard);
    // Esc puts it aside; y brings it back.
    app.on_key(key(KeyCode::Esc));
    assert!(app.approval().is_none());
    assert!(app.toast.as_ref().is_some_and(|(t, _)| t.contains("(y) review")));
    app.toast = None;
    assert!(crate::audit::text(&render(&mut app, 100, 30)).contains("An approval is waiting"));
    app.on_key(key(KeyCode::Char('y')));
    assert!(app.approval().is_some());
}

#[tokio::test]
async fn mouse_selects_rows_and_screens_and_dialogs_own_it() {
    let mut app = fixture();
    app.on_key(key(KeyCode::Char('2')));
    let buffer = render(&mut app, 120, 40);
    // Click the second model row.
    let text = crate::audit::text(&buffer);
    let (y, line) = text.lines().enumerate().find(|(_, l)| l.contains("Qwen3.5-9B")).unwrap();
    let x = line.chars().position(|_| true).unwrap() as u16 + 10;
    app.on_mouse(click(x, y as u16));
    assert_eq!(app.selected_model_idx, 1);
    // Click a screen tab in the navigation bar.
    let (y, line) = text.lines().enumerate().find(|(_, l)| l.contains("(4) Persona")).unwrap();
    let x = line.find("(4) Persona").map(|b| line[..b].chars().count()).unwrap() as u16;
    app.on_mouse(click(x + 1, y as u16));
    assert_eq!(app.active_tab, Tab::Persona);
    // Off-screen clicks do nothing.
    app.on_mouse(click(500, 500));
    assert_eq!(app.active_tab, Tab::Persona);
    // A dialog owns the mouse.
    app.changed();
    app.on_key(key(KeyCode::Char('q')));
    let _ = render(&mut app, 120, 40);
    app.on_mouse(click(x + 1, y as u16));
    assert!(app.quit_confirm, "the background didn't take the click");
}

#[test]
fn theme_choice_is_aurora_by_default() {
    isolate();
    let cfg = crate::tuiconfig::TuiConfig::load(&crate::tuiconfig::path());
    assert_eq!(cfg.theme, "aurora");
}
