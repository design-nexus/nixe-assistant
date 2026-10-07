use crate::protocol::{self, Command};
use anyhow::Context;
use assistant_core::paths;
use assistant_core::Event;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};

pub type Subscriber = broadcast::Receiver<Event>;

pub struct Ipc {
    pub commands: mpsc::Receiver<Command>,
    pub command_tx: mpsc::Sender<Command>,
    pub events: broadcast::Sender<Event>,
}

impl Ipc {
    pub fn subscribe(&self) -> Subscriber {
        self.events.subscribe()
    }

    pub fn emit(&self, ev: Event) {
        let _ = self.events.send(ev);
    }
}

pub async fn bind() -> anyhow::Result<Ipc> {
    let dir = paths::runtime_dir();
    fs::create_dir_all(&dir).context("create runtime dir")?;
    let mut perms = fs::metadata(&dir)?.permissions();
    perms.set_mode(0o700);
    fs::set_permissions(&dir, perms)?;

    let sock = paths::socket_path();
    let _ = fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).context("bind assistant.sock")?;
    let mut perms = fs::metadata(&sock)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&sock, perms)?;

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (ev_tx, _) = broadcast::channel(256);
    let events = ev_tx.clone();

    let listener_cmd_tx = cmd_tx.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let cmd_tx = listener_cmd_tx.clone();
                    let ev_rx = events.subscribe();
                    tokio::spawn(handle_client(stream, cmd_tx, ev_rx));
                }
                Err(err) => {
                    tracing::warn!(%err, "ipc accept failed");
                }
            }
        }
    });

    Ok(Ipc {
        commands: cmd_rx,
        command_tx: cmd_tx,
        events: ev_tx,
    })
}

async fn handle_client(
    stream: UnixStream,
    cmd_tx: mpsc::Sender<Command>,
    mut ev_rx: broadcast::Receiver<Event>,
) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();

    let writer = tokio::spawn(async move {
        loop {
            let ev = match ev_rx.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::debug!(skipped, "ipc client lagged");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };
            match protocol::encode_event(&ev) {
                Ok(line) => {
                    if write.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                    if write.write_all(b"\n").await.is_err() {
                        break;
                    }
                    if write.flush().await.is_err() {
                        break;
                    }
                }
                Err(err) => tracing::warn!(%err, "encode event"),
            }
        }
    });

    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(cmd) = protocol::parse_command(&line) {
            if cmd_tx.send(cmd).await.is_err() {
                break;
            }
        }
    }
    writer.abort();
}

pub async fn send_command(cmd: &Command) -> anyhow::Result<()> {
    let sock = paths::socket_path();
    let mut stream = UnixStream::connect(&sock)
        .await
        .with_context(|| format!("connect {}", sock.display()))?;
    let line = serde_json::to_string(cmd)?;
    stream.write_all(line.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    Ok(())
}

pub async fn send_and_listen(cmd: Option<Command>, timeout_ms: u64) -> anyhow::Result<Vec<Event>> {
    let sock = paths::socket_path();
    let stream = UnixStream::connect(&sock)
        .await
        .with_context(|| format!("connect {}", sock.display()))?;
    let (read, mut write) = stream.into_split();
    if let Some(cmd) = cmd {
        let line = serde_json::to_string(&cmd)?;
        write.write_all(line.as_bytes()).await?;
        write.write_all(b"\n").await?;
        write.flush().await?;
    }
    let mut lines = BufReader::new(read).lines();
    let mut out = Vec::new();
    let deadline = tokio::time::Duration::from_millis(timeout_ms.max(50));
    loop {
        match tokio::time::timeout(deadline, lines.next_line()).await {
            Ok(Ok(Some(line))) => {
                if let Ok(ev) = serde_json::from_str::<Event>(&line) {
                    let done = matches!(
                        ev,
                        Event::Health { .. }
                            | Event::Error { .. }
                            | Event::Timing { .. }
                            | Event::State {
                                state: assistant_core::AssistantState::Idle,
                                ..
                            }
                    );
                    out.push(ev);
                    if done && timeout_ms < 2_000 {
                        break;
                    }
                }
            }
            _ => break,
        }
        if timeout_ms < 2_000 && !out.is_empty() {
            break;
        }
    }
    Ok(out)
}

/// Connect, optionally send a command, and invoke `on_event` for each event until the socket closes.
pub async fn stream_events(
    cmd: Option<Command>,
    mut on_event: impl FnMut(Event),
) -> anyhow::Result<()> {
    let sock = paths::socket_path();
    let stream = UnixStream::connect(&sock)
        .await
        .with_context(|| format!("connect {}", sock.display()))?;
    let (read, mut write) = stream.into_split();
    if let Some(cmd) = cmd {
        let line = serde_json::to_string(&cmd)?;
        write.write_all(line.as_bytes()).await?;
        write.write_all(b"\n").await?;
        write.flush().await?;
    }
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Ok(ev) = serde_json::from_str::<Event>(&line) {
            on_event(ev);
        }
    }
    Ok(())
}

/// Send a command and collect events until a turn finishes (`timing` / `error`) or the deadline.
pub async fn send_until_done(cmd: Command, timeout_ms: u64) -> anyhow::Result<Vec<Event>> {
    let sock = paths::socket_path();
    let stream = UnixStream::connect(&sock)
        .await
        .with_context(|| format!("connect {}", sock.display()))?;
    let (read, mut write) = stream.into_split();
    let line = serde_json::to_string(&cmd)?;
    write.write_all(line.as_bytes()).await?;
    write.write_all(b"\n").await?;
    write.flush().await?;
    let mut lines = BufReader::new(read).lines();
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms.max(1_000));
    loop {
        let remain = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remain.is_zero() {
            break;
        }
        match tokio::time::timeout(remain, lines.next_line()).await {
            Ok(Ok(Some(line))) => {
                if let Ok(ev) = serde_json::from_str::<Event>(&line) {
                    let done = matches!(ev, Event::Timing { .. } | Event::Error { .. });
                    out.push(ev);
                    if done {
                        break;
                    }
                }
            }
            _ => break,
        }
    }
    Ok(out)
}


