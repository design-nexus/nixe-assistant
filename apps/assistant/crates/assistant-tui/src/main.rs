mod app;
// The Aurora kit (audit, aurora, commands, navigation, theme) is copied whole between the
// apps that share it; not every app uses every piece.
#[cfg(test)]
#[allow(dead_code)]
mod audit;
#[allow(dead_code)]
mod aurora;
#[allow(dead_code)]
mod commands;
mod daemon;
mod input;
mod models;
#[allow(dead_code)]
mod navigation;
#[allow(dead_code)]
mod theme;
mod tuiconfig;
mod ui;
use app::App;
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io::stdout, time::Duration};

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen, DisableBracketedPaste, event::DisableMouseCapture, crossterm::cursor::Show);
}

struct TtyGuard;
impl Drop for TtyGuard {
    fn drop(&mut self) {
        restore();
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("nixe-assistant-tui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!(
            "Nixe Assistant — settings, personas, models and activity\n\n\
             Usage: nixe-assistant-tui [--no-mouse] [--help] [--version]\n\n\
             Tab screens  F6 panes  F2 Navigate  Ctrl+K actions  Ctrl+S apply  ? help  q quit\n\
             --no-mouse leaves the mouse to the terminal, so text can be selected.\n\
             The theme is kept in ~/.config/nixe-assistant/tui.toml."
        );
        return Ok(());
    }
    let mouse = !std::env::args().any(|a| a == "--no-mouse");
    let found = theme::find_omarchy();
    let (omarchy, watch) = match found {
        Some((_, theme, path)) => (Some(theme), Some(path)),
        None => (None, None),
    };
    let cfg = tuiconfig::TuiConfig::load(&tuiconfig::path());
    let mut app = App::new(cfg.theme, omarchy)?;
    app.mouse = mouse;
    // Follow Omarchy refreshes live when the desktop theme changes.
    let (theme_tx, mut theme_rx) = tokio::sync::mpsc::unbounded_channel();
    if let Some(path) = watch {
        let dir = path.clone();
        theme::spawn_watcher(path, move || {
            if let Some((_, theme)) = theme::omarchy_load(&dir) {
                let _ = theme_tx.send(theme);
            }
        });
    }
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous_hook(info);
    }));
    enable_raw_mode()?;
    let _guard = TtyGuard;
    execute!(stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    if mouse {
        execute!(stdout(), event::EnableMouseCapture)?;
    }
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    app.poll_daemon();
    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    let mut poll = tokio::time::interval(Duration::from_millis(1500));
    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        tokio::select! {
            Some(theme) = theme_rx.recv() => {
                if app.theme_id == "omarchy" {
                    app.theme = theme.clone();
                }
                app.omarchy = Some(theme);
            }
            Some(update) = app.updates.recv() => app.update(update),
            _ = poll.tick() => app.poll_daemon(),
            _ = ticker.tick() => {
                app.clear_expired_toast();
                app.tick_download();
            }
            ev = events.next() => match ev {
                Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                    if app.on_key(key) {
                        break;
                    }
                }
                Some(Ok(Event::Paste(text))) => app.paste(&text),
                Some(Ok(Event::Mouse(mouse))) => {
                    app.on_mouse(mouse);
                    if app.quit_now {
                        break;
                    }
                }
                Some(Err(err)) => return Err(err.into()),
                None => break,
                _ => {}
            }
        }
    }
    app.cancel_download();
    Ok(())
}
