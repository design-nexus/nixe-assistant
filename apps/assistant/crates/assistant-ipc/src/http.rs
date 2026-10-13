//! Local HTTP API server for the voice assistant.
//!
//! Provides localhost-only HTTP endpoints for status, health, SSE live event streaming,
//! and control actions (say, cancel, reload, ptt).

use crate::protocol::Command;
use anyhow::{Context, Result};
use assistant_core::Event;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{self, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub type StatusFn = Arc<dyn Fn() -> Value + Send + Sync>;

#[derive(Clone)]
pub struct HttpState {
    pub token: String,
    pub cmd_tx: mpsc::Sender<Command>,
    pub ev_tx: broadcast::Sender<Event>,
    pub status_fn: StatusFn,
}

#[derive(Deserialize)]
struct SayPayload {
    text: String,
}

#[derive(Deserialize)]
struct PttPayload {
    active: bool,
}

pub fn build_router(state: HttpState) -> Router {
    Router::new()
        .route("/health", get(status_handler))
        .route("/status", get(status_handler))
        .route("/events", get(events_handler))
        .route("/action/say", post(say_handler))
        .route("/action/cancel", post(cancel_handler))
        .route("/action/reload", post(reload_handler))
        .route("/action/ptt", post(ptt_handler))
        .route("/action/demo", post(demo_handler))
        .route("/action/mute", post(mute_handler))
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .with_state(state)
}

async fn status_handler(State(state): State<HttpState>) -> impl IntoResponse {
    let json = (state.status_fn)();
    Json(json)
}

async fn events_handler(
    State(state): State<HttpState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<sse::Event, Infallible>>> {
    let rx = state.ev_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(ev) => {
            let json = serde_json::to_string(&ev).unwrap_or_default();
            Some(Ok(sse::Event::default().data(json)))
        }
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(sse::KeepAlive::default())
}

async fn say_handler(
    State(state): State<HttpState>,
    Json(payload): Json<SayPayload>,
) -> impl IntoResponse {
    let ok = state
        .cmd_tx
        .send(Command::Say { text: payload.text })
        .await
        .is_ok();
    (
        if ok {
            StatusCode::ACCEPTED
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({"ok":ok,"queued":ok})),
    )
}

async fn cancel_handler(State(state): State<HttpState>) -> impl IntoResponse {
    let ok = state.cmd_tx.send(Command::Cancel).await.is_ok();
    (
        if ok {
            StatusCode::ACCEPTED
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({"ok":ok,"queued":ok})),
    )
}

async fn reload_handler(State(state): State<HttpState>) -> impl IntoResponse {
    let ok = state.cmd_tx.send(Command::Reload).await.is_ok();
    (
        if ok {
            StatusCode::ACCEPTED
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({"ok":ok,"queued":ok})),
    )
}

async fn ptt_handler(
    State(state): State<HttpState>,
    Json(payload): Json<PttPayload>,
) -> impl IntoResponse {
    let cmd = if payload.active {
        Command::PttStart
    } else {
        Command::PttStop
    };
    let ok = state.cmd_tx.send(cmd).await.is_ok();
    (
        if ok {
            StatusCode::ACCEPTED
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({"ok":ok,"queued":ok})),
    )
}

async fn demo_handler(State(state): State<HttpState>) -> impl IntoResponse {
    let ok = state.cmd_tx.send(Command::Demo).await.is_ok();
    (
        if ok {
            StatusCode::ACCEPTED
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({"ok":ok,"queued":ok})),
    )
}

#[derive(Deserialize, Default)]
struct MutePayload {
    #[serde(default)]
    muted: Option<bool>,
}

async fn mute_handler(
    State(state): State<HttpState>,
    body: Option<Json<MutePayload>>,
) -> impl IntoResponse {
    let muted = body.and_then(|Json(b)| b.muted);
    let ok = state.cmd_tx.send(Command::Mute { muted }).await.is_ok();
    (
        if ok {
            StatusCode::ACCEPTED
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({"ok":ok,"queued":ok})),
    )
}

pub async fn start_http_server(
    bind: &str,
    port: u16,
    state: HttpState,
) -> Result<tokio::task::JoinHandle<()>> {
    let ip: std::net::IpAddr = bind.parse().context("HTTP bind must be an IP")?;
    if !ip.is_loopback() {
        anyhow::bail!("HTTP is loopback only");
    }
    let addr = std::net::SocketAddr::new(ip, port);
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind HTTP server to {addr}"))?;
    tracing::info!(bind, port, "HTTP server listening");

    let router = build_router(state);
    let handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, router).await {
            tracing::error!(%err, "HTTP server died");
        }
    });

    Ok(handle)
}

async fn authorize(
    State(state): State<HttpState>,
    request: Request<Body>,
    next: Next,
) -> axum::response::Response {
    let expected = format!("Bearer {}", state.token);
    if request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        != Some(expected.as_str())
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}
pub fn bearer_token() -> Result<String> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let path = assistant_core::paths::runtime_dir().join("http.token");
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    assistant_core::config::atomic_write(&path, token.as_bytes())?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_http_router_health() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(8);
        let (ev_tx, _ev_rx) = broadcast::channel(8);
        let status_fn = Arc::new(|| serde_json::json!({ "ok": true, "name": "Echo" }));
        let state = HttpState {
            token: "test".into(),
            cmd_tx,
            ev_tx,
            status_fn,
        };
        let _router = build_router(state);
    }
}
