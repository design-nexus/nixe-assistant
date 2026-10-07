//! Local HTTP API server for Nyx.
//!
//! Provides localhost-only HTTP endpoints for status, health, SSE live event streaming,
//! and control actions (say, cancel, reload, ptt).

use crate::protocol::Command;
use anyhow::{Context, Result};
use assistant_core::Event;
use axum::extract::State;
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
    let _ = state.cmd_tx.send(Command::Say { text: payload.text }).await;
    Json(serde_json::json!({ "ok": true }))
}

async fn cancel_handler(State(state): State<HttpState>) -> impl IntoResponse {
    let _ = state.cmd_tx.send(Command::Cancel).await;
    Json(serde_json::json!({ "ok": true }))
}

async fn reload_handler(State(state): State<HttpState>) -> impl IntoResponse {
    let _ = state.cmd_tx.send(Command::Reload).await;
    Json(serde_json::json!({ "ok": true }))
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
    let _ = state.cmd_tx.send(cmd).await;
    Json(serde_json::json!({ "ok": true }))
}

async fn demo_handler(State(state): State<HttpState>) -> impl IntoResponse {
    let _ = state.cmd_tx.send(Command::Demo).await;
    Json(serde_json::json!({ "ok": true }))
}

pub async fn start_http_server(
    bind: &str,
    port: u16,
    state: HttpState,
) -> Result<tokio::task::JoinHandle<()>> {
    let addr = format!("{bind}:{port}");
    let listener = TcpListener::bind(&addr)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_http_router_health() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(8);
        let (ev_tx, _ev_rx) = broadcast::channel(8);
        let status_fn = Arc::new(|| serde_json::json!({ "ok": true, "name": "Nyx" }));
        let state = HttpState {
            cmd_tx,
            ev_tx,
            status_fn,
        };
        let _router = build_router(state);
    }
}
