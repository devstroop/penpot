//! Real-time Sync Microservice
//!
//! High-performance WebSocket service for real-time collaboration.
//! Handles presence, cursor tracking, and shape updates.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use common::{init_telemetry, TelemetryConfig};
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::signal;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use uuid::Uuid;

/// Maximum number of connections per room
const MAX_ROOM_CONNECTIONS: usize = 1000;

/// Broadcast channel capacity
const BROADCAST_CAPACITY: usize = 1024;

/// Application state
struct AppState {
    /// Rooms mapped to their broadcast channels
    rooms: DashMap<Uuid, broadcast::Sender<RoomMessage>>,
    /// Active connections count per room
    connection_counts: DashMap<Uuid, usize>,
    /// Server start time
    start_time: Instant,
    /// Prometheus metrics handle
    metrics_handle: PrometheusHandle,
}

impl AppState {
    fn new(metrics_handle: PrometheusHandle) -> Self {
        Self {
            rooms: DashMap::new(),
            connection_counts: DashMap::new(),
            start_time: Instant::now(),
            metrics_handle,
        }
    }

    fn get_or_create_room(&self, room_id: Uuid) -> broadcast::Sender<RoomMessage> {
        self.rooms
            .entry(room_id)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
                tx
            })
            .clone()
    }
}

/// Messages broadcast within a room
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum RoomMessage {
    /// User joined the room
    Join { user_id: Uuid, user_name: String },
    /// User left the room
    Leave { user_id: Uuid },
    /// Cursor position update
    Cursor {
        user_id: Uuid,
        x: f64,
        y: f64,
        page_id: Uuid,
    },
    /// Selection changed
    Selection {
        user_id: Uuid,
        shape_ids: Vec<Uuid>,
    },
    /// Shape update
    ShapeUpdate {
        user_id: Uuid,
        shape_id: Uuid,
        changes: serde_json::Value,
    },
    /// Shape created
    ShapeCreate {
        user_id: Uuid,
        shape: serde_json::Value,
    },
    /// Shape deleted
    ShapeDelete { user_id: Uuid, shape_ids: Vec<Uuid> },
}

/// Health check response
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    uptime_seconds: u64,
    active_rooms: usize,
    version: &'static str,
}

/// Stats response
#[derive(Debug, Serialize)]
struct StatsResponse {
    active_rooms: usize,
    total_connections: usize,
    rooms: Vec<RoomStats>,
}

#[derive(Debug, Serialize)]
struct RoomStats {
    room_id: Uuid,
    connections: usize,
}

#[tokio::main]
async fn main() {
    // Initialize telemetry (tracing + OpenTelemetry)
    let _telemetry = init_telemetry(TelemetryConfig::for_service("realtime-sync"));

    // Initialize Prometheus metrics
    let metrics_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("Failed to install Prometheus recorder");

    let state = Arc::new(AppState::new(metrics_handle));

    let app = Router::new()
        .route("/ws/{file_id}", get(ws_handler))
        .route("/health", get(health_check))
        .route("/stats", get(stats))
        .route("/metrics", get(metrics_endpoint))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8082")
        .await
        .expect("Failed to bind to port 8082");

    info!("🚀 Real-time Sync running on http://0.0.0.0:8082");
    info!("   WS  /ws/{{file_id}} - WebSocket connection");
    info!("   GET /health      - Health check");
    info!("   GET /stats       - Connection statistics");
    info!("   GET /metrics     - Prometheus metrics");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Failed to start server");

    info!("🛑 Real-time Sync shut down gracefully");
}

/// Handle shutdown signals
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("Received Ctrl+C, shutting down..."),
        _ = terminate => info!("Received SIGTERM, shutting down..."),
    }
}

/// WebSocket upgrade handler
#[tracing::instrument(skip(ws, state), fields(room_id = %file_id))]
async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(file_id): Path<Uuid>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, file_id, state))
}

/// Handle WebSocket connection
#[tracing::instrument(skip(socket, state), fields(room_id = %file_id))]
async fn handle_socket(socket: WebSocket, file_id: Uuid, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();

    // Get or create room
    let tx = state.get_or_create_room(file_id);
    let mut rx = tx.subscribe();

    // Increment connection count and update metrics
    *state.connection_counts.entry(file_id).or_insert(0) += 1;
    counter!("ws_connections_total").increment(1);
    gauge!("ws_active_connections").set(
        state.connection_counts.iter().map(|e| *e.value()).sum::<usize>() as f64
    );
    gauge!("ws_active_rooms").set(state.rooms.len() as f64);

    info!("New connection to room {}", file_id);

    let connection_start = Instant::now();

    // Task to forward broadcast messages to this client
    let mut send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if sender.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    // Task to receive messages from this client
    let tx_clone = tx.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                counter!("ws_messages_received").increment(1);
                match serde_json::from_str::<RoomMessage>(&text) {
                    Ok(room_msg) => {
                        // Broadcast to all clients in the room
                        let _ = tx_clone.send(room_msg);
                        counter!("ws_messages_broadcast").increment(1);
                    }
                    Err(e) => {
                        counter!("ws_message_errors").increment(1);
                        warn!("Invalid message format: {}", e);
                    }
                }
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }

    // Record connection duration
    histogram!("ws_connection_duration_seconds").record(connection_start.elapsed().as_secs_f64());

    // Decrement connection count
    if let Some(mut count) = state.connection_counts.get_mut(&file_id) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            drop(count);
            state.connection_counts.remove(&file_id);
            state.rooms.remove(&file_id);
        }
    }

    counter!("ws_disconnections_total").increment(1);
    gauge!("ws_active_connections").set(
        state.connection_counts.iter().map(|e| *e.value()).sum::<usize>() as f64
    );
    gauge!("ws_active_rooms").set(state.rooms.len() as f64);

    info!("Connection closed for room {}", file_id);
}

/// Health check endpoint
async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();
    gauge!("realtime_uptime_seconds").set(uptime as f64);

    Json(HealthResponse {
        status: "healthy",
        uptime_seconds: uptime,
        active_rooms: state.rooms.len(),
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Stats endpoint
async fn stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let rooms: Vec<RoomStats> = state
        .connection_counts
        .iter()
        .map(|entry| RoomStats {
            room_id: *entry.key(),
            connections: *entry.value(),
        })
        .collect();

    let total_connections: usize = rooms.iter().map(|r| r.connections).sum();

    Json(StatsResponse {
        active_rooms: rooms.len(),
        total_connections,
        rooms,
    })
}

/// Prometheus metrics endpoint
async fn metrics_endpoint(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics_handle.render()
}
