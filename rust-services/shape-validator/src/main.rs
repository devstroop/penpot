//! Shape Validator Microservice
//!
//! High-performance shape validation service for Penpot.
//! Replaces Malli schema validation with compiled Rust validation.
//!
//! ## Endpoints
//!
//! - `POST /validate` - Validate a batch of shapes
//! - `GET /health` - Health check
//! - `GET /metrics` - Prometheus metrics

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use common::{init_telemetry, validation, Shape, TelemetryConfig};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::signal;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Application state
#[derive(Clone)]
struct AppState {
    start_time: Instant,
    metrics_handle: PrometheusHandle,
}

/// Request body for validation endpoint
#[derive(Debug, Deserialize)]
struct ValidateRequest {
    shapes: Vec<Shape>,
}

/// Response for validation endpoint
#[derive(Debug, Serialize)]
struct ValidateResponse {
    valid: bool,
    total_shapes: usize,
    valid_shapes: usize,
    invalid_shapes: usize,
    total_errors: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    results: Option<Vec<validation::ShapeValidationResult>>,
    processing_time_us: u64,
}

/// Health check response
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    uptime_seconds: u64,
    version: &'static str,
}

#[tokio::main]
async fn main() {
    // Initialize telemetry (tracing + OpenTelemetry)
    let _telemetry = init_telemetry(TelemetryConfig::for_service("shape-validator"));

    // Initialize Prometheus metrics
    let metrics_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("Failed to install Prometheus recorder");

    let state = AppState {
        start_time: Instant::now(),
        metrics_handle,
    };

    let app = Router::new()
        .route("/validate", post(validate_shapes))
        .route("/health", get(health_check))
        .route("/metrics", get(metrics_endpoint))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(Arc::new(state));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8081")
        .await
        .expect("Failed to bind to port 8081");

    info!("🚀 Shape Validator running on http://0.0.0.0:8081");
    info!("   POST /validate - Validate shapes");
    info!("   GET  /health   - Health check");
    info!("   GET  /metrics  - Prometheus metrics");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Failed to start server");

    info!("🛑 Shape Validator shut down gracefully");
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

/// Validate a batch of shapes
#[tracing::instrument(skip(request), fields(shape_count = request.shapes.len()))]
async fn validate_shapes(
    Json(request): Json<ValidateRequest>,
) -> impl IntoResponse {
    let start = Instant::now();
    let shape_count = request.shapes.len();

    // Record metrics
    counter!("validator_requests_total").increment(1);
    counter!("validator_shapes_total").increment(shape_count as u64);

    let result = validation::validate_shapes_batch(&request.shapes);
    let processing_time = start.elapsed();
    let processing_time_us = processing_time.as_micros() as u64;

    // Record timing and results
    histogram!("validator_processing_duration_seconds").record(processing_time.as_secs_f64());
    histogram!("validator_shapes_per_request").record(shape_count as f64);
    
    if result.valid {
        counter!("validator_valid_requests_total").increment(1);
    } else {
        counter!("validator_invalid_requests_total").increment(1);
        counter!("validator_errors_total").increment(result.total_errors as u64);
    }

    let response = ValidateResponse {
        valid: result.valid,
        total_shapes: result.total_shapes,
        valid_shapes: result.valid_shapes,
        invalid_shapes: result.invalid_shapes,
        total_errors: result.total_errors,
        results: if result.valid {
            None
        } else {
            Some(result.results)
        },
        processing_time_us,
    };

    let status = if result.valid {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };

    (status, Json(response))
}

/// Health check endpoint
#[tracing::instrument(skip(state))]
async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();
    gauge!("validator_uptime_seconds").set(uptime as f64);

    Json(HealthResponse {
        status: "healthy",
        uptime_seconds: uptime,
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Prometheus metrics endpoint
async fn metrics_endpoint(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics_handle.render()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn create_test_app() -> Router {
        // Initialize metrics for testing
        let metrics_handle = PrometheusBuilder::new()
            .install_recorder()
            .expect("Failed to install Prometheus recorder");

        let state = AppState {
            start_time: Instant::now(),
            metrics_handle,
        };

        Router::new()
            .route("/validate", post(validate_shapes))
            .route("/health", get(health_check))
            .with_state(Arc::new(state))
    }

    #[tokio::test]
    async fn test_health_check() {
        let app = create_test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
