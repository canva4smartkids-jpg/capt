//! axum router for the companion REST/WebSocket API.

use crate::pairing::{PairError, Pairing};
use crate::protocol::{routes, HubEvent, PairRequest, PairResponse, RecordingListResponse, RemoteCommand, RemoteCommandResult};
use crate::HubBackend;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tower::ServiceExt;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeFile;

#[derive(Clone)]
pub(crate) struct AppState {
    pub backend: Arc<dyn HubBackend>,
    pub pairing: Arc<Pairing>,
    pub events: broadcast::Sender<HubEvent>,
    pub shutdown: watch::Receiver<bool>,
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

/// Runs a (possibly blocking) backend call off the async workers.
async fn call<T: Send + 'static>(state: &AppState, f: impl FnOnce(&dyn HubBackend) -> T + Send + 'static) -> Result<T, Response> {
    let backend = state.backend.clone();
    tokio::task::spawn_blocking(move || f(backend.as_ref()))
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("backend call failed: {e}")))
}

pub(crate) fn router(state: AppState) -> Router {
    // TV web apps run from file:// or packaged-app origins, so allow any origin. Auth is a
    // bearer token (header or ?token=), never cookies, so a wildcard origin does not expose
    // anything to a page that has not paired.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS, Method::HEAD])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::RANGE])
        .expose_headers([header::CONTENT_RANGE, header::ACCEPT_RANGES, header::CONTENT_LENGTH, header::RETRY_AFTER])
        .allow_private_network(true)
        .max_age(Duration::from_secs(600));

    let authed = Router::new()
        .route(routes::RECORDINGS, get(list_recordings))
        .route("/api/v1/recordings/{id}/file", get(recording_file))
        .route("/api/v1/recordings/{id}/thumbnail", get(recording_thumbnail))
        .route(routes::STATUS, get(status))
        .route(routes::COMMAND, post(command))
        .route(routes::EVENTS, get(events))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .route(routes::INFO, get(info))
        .route(routes::PAIR, post(pair))
        .merge(authed)
        .fallback(|| async { json_error(StatusCode::NOT_FOUND, "not found") })
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(cors)
        .with_state(state)
}

/// Extracts the bearer token from `Authorization: Bearer` or the `token` query parameter
/// (needed for `<video src>`, `<img src>` and WebSocket URLs, which cannot set headers).
pub(crate) fn token_from(headers: &HeaderMap, query: Option<&str>) -> Option<String> {
    if let Some(v) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if v.len() > 7 && v[..7].eq_ignore_ascii_case("bearer ") {
            return Some(v[7..].trim().to_string());
        }
    }
    query?.split('&').find_map(|kv| kv.strip_prefix("token=")).map(crate::protocol::percent_decode)
}

async fn require_token(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }
    let token = token_from(req.headers(), req.uri().query());
    match token.and_then(|t| state.pairing.authenticate(&t)) {
        Some(_device) => next.run(req).await,
        None => {
            let mut resp = json_error(StatusCode::UNAUTHORIZED, "missing or invalid token; pair with the 6-digit code shown on the desktop");
            resp.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            resp
        }
    }
}

async fn info(State(state): State<AppState>) -> Response {
    match call(&state, |b| b.hub_info()).await {
        Ok(info) => Json(info).into_response(),
        Err(r) => r,
    }
}

async fn pair(State(state): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, body: axum::body::Bytes) -> Response {
    let req: PairRequest = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(mut v) => {
            // A companion's capability report is informational; an unknown/newer shape must
            // not make pairing fail, so drop it if it does not parse.
            let caps_ok = v.get("capabilities").map(|c| serde_json::from_value::<capture_core::CapabilityReport>(c.clone()).is_ok()).unwrap_or(true);
            if !caps_ok {
                if let Some(o) = v.as_object_mut() {
                    o.remove("capabilities");
                }
            }
            match serde_json::from_value(v) {
                Ok(r) => r,
                Err(e) => return json_error(StatusCode::BAD_REQUEST, format!("invalid pair request: {e}")),
            }
        }
        Err(e) => return json_error(StatusCode::BAD_REQUEST, format!("invalid JSON: {e}")),
    };
    let pairing = state.pairing.clone();
    let ip = peer.ip();
    let result = tokio::task::spawn_blocking(move || pairing.pair(ip, &req, capture_core::now_iso8601())).await;
    match result {
        Ok(Ok((token, device))) => {
            log::info!("companion hub: paired {:?} ({})", device.name, device.device_kind.as_str());
            match call(&state, |b| b.hub_info()).await {
                Ok(hub) => Json(PairResponse { token, hub }).into_response(),
                Err(r) => r,
            }
        }
        Ok(Err(PairError::Invalid(m))) => json_error(StatusCode::BAD_REQUEST, m),
        Ok(Err(PairError::WrongCode { remaining_attempts })) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "wrong pairing code", "remainingAttempts": remaining_attempts })),
        )
            .into_response(),
        Ok(Err(PairError::LockedOut { retry_after })) => {
            let secs = retry_after.as_secs_f64().ceil().max(1.0) as u64;
            let mut resp = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({ "error": "too many wrong codes; try again later", "retryAfterSec": secs })),
            )
                .into_response();
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
            resp
        }
        Ok(Err(PairError::TooManyDevices)) => json_error(StatusCode::CONFLICT, "too many paired devices; remove one on the desktop first"),
        Ok(Err(PairError::Storage(m))) => json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("could not store pairing: {m}")),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn list_recordings(State(state): State<AppState>) -> Response {
    let items = match call(&state, |b| {
        b.list_recordings()
            .into_iter()
            .map(|mut m| {
                // Companions get hub-relative URLs instead of the desktop's local file paths.
                let has_thumb = b.thumbnail_path(&m.id).is_some();
                m.path = routes::recording_file(&m.id);
                m.thumbnail_path = has_thumb.then(|| routes::recording_thumbnail(&m.id));
                m
            })
            .collect::<Vec<_>>()
    })
    .await
    {
        Ok(items) => items,
        Err(r) => return r,
    };
    Json(RecordingListResponse { items }).into_response()
}

async fn serve_path(path: Option<PathBuf>, req: Request) -> Response {
    let Some(path) = path else {
        return json_error(StatusCode::NOT_FOUND, "no such recording");
    };
    if !path.is_file() {
        return json_error(StatusCode::NOT_FOUND, "recording file is missing on the desktop");
    }
    // ServeFile implements Range / If-Range / HEAD and guesses Content-Type from the extension.
    match ServeFile::new(&path).oneshot(req).await {
        Ok(resp) => {
            let mut resp = resp.map(Body::new);
            resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
            resp
        }
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn recording_file(State(state): State<AppState>, Path(id): Path<String>, req: Request) -> Response {
    match call(&state, move |b| b.recording_path(&id)).await {
        Ok(p) => serve_path(p, req).await,
        Err(r) => r,
    }
}

async fn recording_thumbnail(State(state): State<AppState>, Path(id): Path<String>, req: Request) -> Response {
    match call(&state, move |b| b.thumbnail_path(&id)).await {
        Ok(p) => serve_path(p, req).await,
        Err(r) => r,
    }
}

async fn status(State(state): State<AppState>) -> Response {
    match call(&state, |b| b.status()).await {
        Ok(s) => Json(s).into_response(),
        Err(r) => r,
    }
}

async fn command(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    let cmd: RemoteCommand = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(RemoteCommandResult::err(format!("invalid command: {e}")))).into_response(),
    };
    match call(&state, move |b| b.command(cmd)).await {
        Ok(result) => Json(result).into_response(),
        Err(r) => r,
    }
}

async fn events(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(64 * 1024).on_upgrade(move |socket| event_socket(socket, state))
}

async fn send_event(socket: &mut futures_util::stream::SplitSink<WebSocket, Message>, ev: &HubEvent) -> bool {
    match serde_json::to_string(ev) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => true,
    }
}

async fn event_socket(socket: WebSocket, state: AppState) {
    // Subscribe before sending the snapshot so no broadcast is missed in between.
    let mut rx = state.events.subscribe();
    let mut shutdown = state.shutdown.clone();
    let (mut tx, mut incoming) = socket.split();
    let Ok(hub) = call(&state, |b| b.hub_info()).await else { return };
    if !send_event(&mut tx, &HubEvent::Hello { hub }).await {
        return;
    }
    let Ok(status) = call(&state, |b| b.status()).await else { return };
    if !send_event(&mut tx, &HubEvent::RecordingStatus { status }).await {
        return;
    }
    let mut keepalive = tokio::time::interval(Duration::from_secs(25));
    keepalive.tick().await;
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => if !send_event(&mut tx, &ev).await { break },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Slow client: resync with the current status instead of replaying.
                    let Ok(status) = call(&state, |b| b.status()).await else { break };
                    if !send_event(&mut tx, &HubEvent::RecordingStatus { status }).await { break }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            msg = incoming.next() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {} // pings are answered by axum; companions send nothing else
            },
            _ = keepalive.tick() => {
                if tx.send(Message::Ping(Vec::new().into())).await.is_err() { break }
            }
            _ = shutdown.changed() => break,
        }
    }
    let _ = tx.send(Message::Close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_extraction() {
        let mut h = HeaderMap::new();
        assert_eq!(token_from(&h, None), None);
        assert_eq!(token_from(&h, Some("a=1&token=abc%20d")), Some("abc d".into()));
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer  xyz "));
        assert_eq!(token_from(&h, Some("token=abc")), Some("xyz".into()));
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic xyz"));
        assert_eq!(token_from(&h, None), None);
    }
}
