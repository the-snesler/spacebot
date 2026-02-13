//! HTTP server setup: router, static file serving, and API routes.

use super::state::{AgentInfo, ApiEvent, ApiState};
use crate::conversation::channels::ChannelStore;
use crate::conversation::history::{ProcessRunLogger, TimelineItem};

use axum::extract::{Query, State};
use axum::http::{header, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Json, Response, Sse};
use axum::routing::get;
use axum::Router;
use futures::stream::Stream;
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

/// Embedded frontend assets from the Vite build output.
#[derive(Embed)]
#[folder = "interface/dist/"]
#[allow(unused)]
struct InterfaceAssets;

// -- Response types --

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct StatusResponse {
    status: &'static str,
    pid: u32,
    uptime_seconds: u64,
}

#[derive(Serialize)]
struct ChannelResponse {
    agent_id: String,
    id: String,
    platform: String,
    display_name: Option<String>,
    is_active: bool,
    last_activity_at: String,
    created_at: String,
}

#[derive(Serialize)]
struct ChannelsResponse {
    channels: Vec<ChannelResponse>,
}

#[derive(Serialize)]
struct MessagesResponse {
    items: Vec<TimelineItem>,
}

#[derive(Serialize)]
struct AgentsResponse {
    agents: Vec<AgentInfo>,
}

/// Start the HTTP server on the given address.
///
/// The caller provides a pre-built `ApiState` so agent event streams and
/// DB pools can be registered after startup.
pub async fn start_http_server(
    bind: SocketAddr,
    state: Arc<ApiState>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/events", get(events_sse))
        .route("/agents", get(list_agents))
        .route("/channels", get(list_channels))
        .route("/channels/messages", get(channel_messages))
        .route("/channels/status", get(channel_status));

    let app = Router::new()
        .nest("/api", api_routes)
        .fallback(static_handler)
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "HTTP server listening");

    let handle = tokio::spawn(async move {
        let mut shutdown = shutdown_rx;
        if let Err(error) = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown.wait_for(|v| *v).await;
            })
            .await
        {
            tracing::error!(%error, "HTTP server exited with error");
        }
    });

    Ok(handle)
}

// -- API handlers --

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn status(State(state): State<Arc<ApiState>>) -> Json<StatusResponse> {
    let uptime = state.started_at.elapsed();
    Json(StatusResponse {
        status: "running",
        pid: std::process::id(),
        uptime_seconds: uptime.as_secs(),
    })
}

/// List all configured agents with their config summaries.
async fn list_agents(State(state): State<Arc<ApiState>>) -> Json<AgentsResponse> {
    let agents = state.agent_configs.load();
    Json(AgentsResponse { agents: agents.as_ref().clone() })
}

/// SSE endpoint streaming all agent events to connected clients.
async fn events_sse(
    State(state): State<Arc<ApiState>>,
) -> Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>> {
    let mut rx = state.event_tx.subscribe();

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(json) = serde_json::to_string(&event) {
                        let event_type = match &event {
                            ApiEvent::InboundMessage { .. } => "inbound_message",
                            ApiEvent::OutboundMessage { .. } => "outbound_message",
                            ApiEvent::TypingState { .. } => "typing_state",
                            ApiEvent::WorkerStarted { .. } => "worker_started",
                            ApiEvent::WorkerStatusUpdate { .. } => "worker_status",
                            ApiEvent::WorkerCompleted { .. } => "worker_completed",
                            ApiEvent::BranchStarted { .. } => "branch_started",
                            ApiEvent::BranchCompleted { .. } => "branch_completed",
                            ApiEvent::ToolStarted { .. } => "tool_started",
                            ApiEvent::ToolCompleted { .. } => "tool_completed",
                        };
                        yield Ok(axum::response::sse::Event::default()
                            .event(event_type)
                            .data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    tracing::debug!(count, "SSE client lagged");
                    yield Ok(axum::response::sse::Event::default()
                        .event("lagged")
                        .data(format!("{{\"skipped\":{count}}}")));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

/// List active channels across all agents.
async fn list_channels(State(state): State<Arc<ApiState>>) -> Json<ChannelsResponse> {
    let pools = state.agent_pools.load();
    let mut all_channels = Vec::new();

    for (agent_id, pool) in pools.iter() {
        let store = ChannelStore::new(pool.clone());
        match store.list_active().await {
            Ok(channels) => {
                for channel in channels {
                    all_channels.push(ChannelResponse {
                        agent_id: agent_id.clone(),
                        id: channel.id,
                        platform: channel.platform,
                        display_name: channel.display_name,
                        is_active: channel.is_active,
                        last_activity_at: channel.last_activity_at.to_rfc3339(),
                        created_at: channel.created_at.to_rfc3339(),
                    });
                }
            }
            Err(error) => {
                tracing::warn!(%error, agent_id, "failed to list channels");
            }
        }
    }

    Json(ChannelsResponse { channels: all_channels })
}

#[derive(Deserialize)]
struct MessagesQuery {
    channel_id: String,
    #[serde(default = "default_message_limit")]
    limit: i64,
}

fn default_message_limit() -> i64 {
    20
}

/// Get the unified timeline for a channel: messages, branch runs, and worker runs
/// interleaved chronologically.
async fn channel_messages(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<MessagesQuery>,
) -> Json<MessagesResponse> {
    let pools = state.agent_pools.load();
    let limit = query.limit.min(100);

    for (_agent_id, pool) in pools.iter() {
        let logger = ProcessRunLogger::new(pool.clone());
        match logger.load_channel_timeline(&query.channel_id, limit).await {
            Ok(items) if !items.is_empty() => {
                return Json(MessagesResponse { items });
            }
            Ok(_) => continue,
            Err(error) => {
                tracing::warn!(%error, channel_id = %query.channel_id, "failed to load timeline");
                continue;
            }
        }
    }

    Json(MessagesResponse { items: vec![] })
}

/// Get live status (active workers, branches, completed items) for all channels.
///
/// Returns the StatusBlock directly -- it already derives Serialize.
async fn channel_status(
    State(state): State<Arc<ApiState>>,
) -> Json<HashMap<String, serde_json::Value>> {
    // Snapshot the map under the outer lock, then release it so
    // register/unregister calls aren't blocked during serialization.
    let snapshot: Vec<_> = {
        let blocks = state.channel_status_blocks.read().await;
        blocks.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };

    let mut result = HashMap::new();
    for (channel_id, status_block) in snapshot {
        let block = status_block.read().await;
        if let Ok(value) = serde_json::to_value(&*block) {
            result.insert(channel_id, value);
        }
    }

    Json(result)
}

// -- Static file serving --

async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    if let Some(content) = InterfaceAssets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime.as_ref())],
            content.data,
        )
            .into_response();
    }

    // SPA fallback
    if let Some(content) = InterfaceAssets::get("index.html") {
        return Html(
            std::str::from_utf8(&content.data)
                .unwrap_or("")
                .to_string(),
        )
        .into_response();
    }

    (StatusCode::NOT_FOUND, "not found").into_response()
}
