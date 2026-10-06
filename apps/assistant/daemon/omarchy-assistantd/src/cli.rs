use assistant_ipc::{send_and_listen, send_command, send_until_done, stream_events, Command};
use std::io::{self, Write};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".into());
    match cmd.as_str() {
        "ptt-start" => send_command(&Command::PttStart).await?,
        "ptt-stop" => send_command(&Command::PttStop).await?,
        "cancel" => send_command(&Command::Cancel).await?,
        "cancel-esc" => cancel_esc().await?,
        "reload" => {
            send_command(&Command::Reload).await?;
            let events = send_and_listen(Some(Command::Status), 400).await?;
            print_events(events);
        }
        "shutdown" => send_command(&Command::Shutdown).await?,
        "status" => {
            let events = send_and_listen(Some(Command::Status), 400).await?;
            print_events(events);
        }
        "listen" => {
            stream_events(Some(Command::Status), |ev| {
                print_event(&ev);
            })
            .await?;
        }
        "say" => {
            let text = args.collect::<Vec<_>>().join(" ");
            if text.trim().is_empty() {
                anyhow::bail!("usage: omarchy-assistant say <text>");
            }
            print_events(send_until_done(Command::Say { text }, 120_000).await?);
        }
        "demo" => {
            print_events(send_until_done(Command::Demo, 180_000).await?);
        }
        "help" | "-h" | "--help" => print_help(),
        other => {
            eprintln!("unknown command: {other}");
            print_help();
            std::process::exit(1);
        }
    }
    Ok(())
}

fn print_event(ev: &assistant_core::Event) {
    let mut out = io::stdout();
    if let Ok(line) = serde_json::to_string(ev) {
        let _ = writeln!(out, "{line}");
    }
    let _ = out.flush();
}

fn print_events(events: Vec<assistant_core::Event>) {
    for ev in events {
        print_event(&ev);
    }
}

async fn cancel_esc() -> anyhow::Result<()> {
    let events = send_and_listen(Some(Command::Status), 80).await?;
    let busy = events.iter().any(|ev| match ev {
        assistant_core::Event::Health { state, .. }
        | assistant_core::Event::State { state, .. } => {
            !matches!(
                state,
                assistant_core::AssistantState::Idle | assistant_core::AssistantState::Interrupted
            )
        }
        _ => false,
    });
    if busy {
        send_command(&Command::Cancel).await?;
        return Ok(());
    }
    let _ = std::process::Command::new("hyprctl")
        .args(["dispatch", "sendshortcut", ",Escape,"])
        .status();
    Ok(())
}

fn print_help() {
    eprintln!(
        "omarchy-assistant — Nyx voice assistant client\n\n\
         Commands:\n\
           ptt-start   start push-to-talk capture\n\
           ptt-stop    stop capture (key-up)\n\
           cancel      interrupt the current turn\n\
           cancel-esc  Escape: stop Nyx, or pass Escape through when idle\n\
           status      print a health event\n\
           say TEXT    skip STT; speak an LLM reply\n\
           demo        Kokoro → Parakeet → LLM → speakers\n\
           listen      print events until idle/timeout\n\
           reload      reread apps.toml, commands.toml, and config.toml without restarting models\n\
           shutdown    stop the daemon\n"
    );
}
