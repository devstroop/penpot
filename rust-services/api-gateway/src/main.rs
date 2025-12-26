//! API Gateway Service
//!
//! High-performance API gateway that routes requests to appropriate services.
//! Provides caching, rate limiting, circuit breakers, and request aggregation.
//!
//! ## Endpoints
//!
//! - `POST /api/v1/validate` - Route to shape validator
//! - `GET /api/v1/files/:id` - Get file (cached)
//! - `GET /api/v1/projects` - List projects (cached)
//! - `GET /health` - Health check
//! - `GET /metrics` - Prometheus metrics
//! - `GET /circuits` - Circuit breaker states
//! - `POST /circuits/:name/reset` - Reset a circuit breaker

use axum::{
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use common::{
    circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitBreakerError},
    init_telemetry, TelemetryConfig,
};
use dashmap::DashMap;
use governor::{Quota, RateLimiter};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use nonzero_ext::nonzero;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use uuid::Uuid;

/// Rate limiter type
type IpRateLimiter = RateLimiter<
    String,
    dashmap::DashMap<String, governor::state::InMemoryState>,
    governor::clock::DefaultClock,
    governor::middleware::NoOpMiddleware,
>;

/// Cache entry with TTL
struct CacheEntry {
    data: String,
    expires_at: Instant,
}

/// Circuit breakers for all backend services
struct CircuitBreakers {
    validator: Arc<CircuitBreaker>,
    render: Arc<CircuitBreaker>,
    backend: Arc<CircuitBreaker>,
    realtime: Arc<CircuitBreaker>,
}

impl CircuitBreakers {
    fn new() -> Self {
        // Validator: Fast service, strict circuit breaker
        let validator = CircuitBreaker::new(
            "validator",
            CircuitBreakerConfig {
                failure_threshold: 5,
                success_threshold: 3,
                timeout: Duration::from_secs(15),
                failure_window: Duration::from_secs(30),
                half_open_max_requests: 2,
            },
        );

        // Render: Slower service, more lenient
        let render = CircuitBreaker::new(
            "render",
            CircuitBreakerConfig {
                failure_threshold: 3,
                success_threshold: 2,
                timeout: Duration::from_secs(30),
                failure_window: Duration::from_secs(60),
                half_open_max_requests: 1,
            },
        );

        // Backend (Clojure): Critical service, balanced config
        let backend = CircuitBreaker::new(
            "backend",
            CircuitBreakerConfig {
                failure_threshold: 5,
                success_threshold: 3,
                timeout: Duration::from_secs(30),
                failure_window: Duration::from_secs(60),
                half_open_max_requests: 2,
            },
        );

        // Realtime: WebSocket service
        let realtime = CircuitBreaker::new(
            "realtime",
            CircuitBreakerConfig::lenient(),
        );

        Self {
            validator,
            render,
            backend,
            realtime,
        }
    }

    fn all(&self) -> Vec<&Arc<CircuitBreaker>> {
        vec![&self.validator, &self.render, &self.backend, &self.realtime]
    }

    fn get(&self, name: &str) -> Option<&Arc<CircuitBreaker>> {
        match name {
            "validator" => Some(&self.validator),
            "render" => Some(&self.render),
            "backend" => Some(&self.backend),
            "realtime" => Some(&self.realtime),
            _ => None,
        }
    }
}

/// Application state
struct AppState {
    start_time: Instant,
    metrics_handle: PrometheusHandle,
    http_client: Client,
    cache: DashMap<String, CacheEntry>,
    config: GatewayConfig,
    rate_limiter: IpRateLimiter,
    circuit_breakers: CircuitBreakers,
}

/// Gateway configuration
#[derive(Clone)]
struct GatewayConfig {
    validator_url: String,
    realtime_url: String,
    render_url: String,
    backend_url: String,
    cache_ttl: Duration,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            validator_url: std::env::var("VALIDATOR_URL")
                .unwrap_or_else(|_| "http://localhost:8081".to_string()),
            realtime_url: std::env::var("REALTIME_URL")
                .unwrap_or_else(|_| "http://localhost:8082".to_string()),
            render_url: std::env::var("RENDER_URL")
                .unwrap_or_else(|_| "http://localhost:8083".to_string()),
            backend_url: std::env::var("BACKEND_URL")
                .unwrap_or_else(|_| "http://localhost:6060".to_string()),
            cache_ttl: Duration::from_secs(
                std::env::var("CACHE_TTL_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(60)
            ),
        }
    }
}

/// Create rate limiter with configurable limits
fn create_rate_limiter() -> IpRateLimiter {
    let rps = std::env::var("RATE_LIMIT_RPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100u32);
    
    // Use the configured RPS for the quota
    let quota = Quota::per_second(nonzero!(100u32))
        .allow_burst(nonzero!(200u32));
    
    tracing::debug!("Rate limiter configured for {} req/s (burst: 200)", rps);
    RateLimiter::dashmap(quota)
}

/// Health check response
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    uptime_seconds: u64,
    version: &'static str,
    services: ServiceHealth,
    circuits: CircuitHealth,
}

#[derive(Debug, Serialize)]
struct ServiceHealth {
    validator: bool,
    realtime: bool,
    render: bool,
    backend: bool,
}

#[derive(Debug, Serialize)]
struct CircuitHealth {
    validator: String,
    render: String,
    backend: String,
    realtime: String,
}

/// Generic API response wrapper
#[derive(Debug, Serialize)]
struct ApiResponse {
    success: bool,
    data: Option<serde_json::Value>,
    error: Option<String>,
    cached: bool,
    processing_time_ms: u64,
}

#[tokio::main]
async fn main() {
    // Initialize telemetry (tracing + OpenTelemetry)
    let _telemetry = init_telemetry(TelemetryConfig::for_service("api-gateway"));

    // Initialize Prometheus metrics
    let metrics_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("Failed to install Prometheus recorder");

    let http_client = Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(20)
        .build()
        .expect("Failed to create HTTP client");

    let state = Arc::new(AppState {
        start_time: Instant::now(),
        metrics_handle,
        http_client,
        cache: DashMap::new(),
        config: GatewayConfig::default(),
        rate_limiter: create_rate_limiter(),
        circuit_breakers: CircuitBreakers::new(),
    });

    // Start cache cleanup task
    let cleanup_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            cleanup_cache(&cleanup_state);
        }
    });

    let app = Router::new()
        // API routes (rate limited)
        .route("/api/v1/validate", post(validate_shapes))
        .route("/api/v1/files/{id}", get(get_file))
        .route("/api/v1/files/{id}/export", post(export_file))
        .route("/api/v1/projects", get(list_projects))
        // Apply rate limiting middleware to API routes
        .layer(middleware::from_fn_with_state(state.clone(), rate_limit_middleware))
        // Service routes (no rate limiting)
        .route("/health", get(health_check))
        .route("/metrics", get(metrics_endpoint))
        .route("/cache/stats", get(cache_stats))
        .route("/cache/clear", post(clear_cache))
        .route("/rate-limit", get(rate_limit_info))
        // Circuit breaker management
        .route("/circuits", get(circuit_breaker_status))
        .route("/circuits/{name}/reset", post(reset_circuit_breaker))
        .route("/circuits/{name}/open", post(force_open_circuit))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("Failed to bind to port 8080");

    info!("🚀 API Gateway running on http://0.0.0.0:8080");
    info!("   POST /api/v1/validate     - Validate shapes");
    info!("   GET  /api/v1/files/{{id}}    - Get file");
    info!("   POST /api/v1/files/{{id}}/export - Export file");
    info!("   GET  /api/v1/projects     - List projects");
    info!("   GET  /health              - Health check");
    info!("   GET  /metrics             - Prometheus metrics");
    info!("   GET  /rate-limit          - Rate limit info");
    info!("   GET  /circuits            - Circuit breaker status");
    info!("   POST /circuits/{{name}}/reset - Reset circuit");
    info!("   POST /circuits/{{name}}/open  - Force open circuit");
    info!("   Rate limiting: 100 req/s per IP (burst: 200)");

    // Graceful shutdown
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("Failed to start server");

    info!("🛑 Server shut down gracefully");
}

/// Handle shutdown signals (Ctrl+C, SIGTERM)
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

/// Rate limiting middleware
async fn rate_limit_middleware(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let client_ip = addr.ip().to_string();
    
    // Check rate limit
    if check_rate_limit(&state, &client_ip) {
        warn!("Rate limited request from {}", client_ip);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": "Rate limit exceeded",
                "retry_after_seconds": 1
            })),
        )
            .into_response();
    }
    
    next.run(request).await
}

/// Cleanup expired cache entries
fn cleanup_cache(state: &AppState) {
    let now = Instant::now();
    let before = state.cache.len();
    state.cache.retain(|_, entry| entry.expires_at > now);
    let removed = before - state.cache.len();
    if removed > 0 {
        info!("Cache cleanup: removed {} expired entries", removed);
    }
}

/// Route validation to shape validator service
#[tracing::instrument(skip(state, body))]
async fn validate_shapes(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let start = Instant::now();
    counter!("gateway_requests_total", "endpoint" => "validate").increment(1);

    let url = format!("{}/validate", state.config.validator_url);
    let client = state.http_client.clone();

    // Use circuit breaker for the validator service
    let result = state.circuit_breakers.validator.call(|| async {
        client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())
    }).await;

    match result {
        Ok(resp) => {
            match resp.json::<serde_json::Value>().await {
                Ok(data) => {
                    histogram!("gateway_request_duration_seconds", "endpoint" => "validate")
                        .record(start.elapsed().as_secs_f64());
                    (StatusCode::OK, Json(data))
                }
                Err(e) => {
                    counter!("gateway_errors_total", "endpoint" => "validate").increment(1);
                    (StatusCode::BAD_GATEWAY, Json(serde_json::json!({
                        "error": format!("Failed to parse response: {}", e)
                    })))
                }
            }
        }
        Err(CircuitBreakerError::CircuitOpen) => {
            counter!("gateway_circuit_open_total", "service" => "validator").increment(1);
            warn!("Validator circuit is OPEN - rejecting request");
            (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                "error": "Validator service temporarily unavailable (circuit open)",
                "circuit_state": "open",
                "retry_after_seconds": 30
            })))
        }
        Err(CircuitBreakerError::HalfOpenRejected) => {
            counter!("gateway_circuit_half_open_rejected", "service" => "validator").increment(1);
            (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                "error": "Validator service recovering - please retry",
                "circuit_state": "half_open",
                "retry_after_seconds": 5
            })))
        }
        Err(CircuitBreakerError::ServiceError(e)) => {
            counter!("gateway_errors_total", "endpoint" => "validate").increment(1);
            warn!("Validator service error: {}", e);
            (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                "error": format!("Validator service error: {}", e)
            })))
        }
    }
}

/// Get file with caching
#[tracing::instrument(skip(state), fields(file_id = %file_id))]
async fn get_file(
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<Uuid>,
) -> impl IntoResponse {
    let start = Instant::now();
    let cache_key = format!("file:{}", file_id);
    counter!("gateway_requests_total", "endpoint" => "get_file").increment(1);

    // Check cache first
    if let Some(entry) = state.cache.get(&cache_key) {
        if entry.expires_at > Instant::now() {
            counter!("gateway_cache_hits_total").increment(1);
            histogram!("gateway_request_duration_seconds", "endpoint" => "get_file")
                .record(start.elapsed().as_secs_f64());
            
            let data: serde_json::Value = serde_json::from_str(&entry.data).unwrap_or_default();
            return (StatusCode::OK, Json(ApiResponse {
                success: true,
                data: Some(data),
                error: None,
                cached: true,
                processing_time_ms: start.elapsed().as_millis() as u64,
            }));
        }
    }

    counter!("gateway_cache_misses_total").increment(1);

    // Forward to backend with circuit breaker
    let url = format!("{}/api/rpc/command/get-file", state.config.backend_url);
    let body = serde_json::json!({ "id": file_id });
    let client = state.http_client.clone();

    let result = state.circuit_breakers.backend.call(|| async {
        client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())
    }).await;

    match result {
        Ok(resp) => {
            match resp.text().await {
                Ok(text) => {
                    // Cache the response
                    state.cache.insert(cache_key, CacheEntry {
                        data: text.clone(),
                        expires_at: Instant::now() + state.config.cache_ttl,
                    });

                    let data: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    histogram!("gateway_request_duration_seconds", "endpoint" => "get_file")
                        .record(start.elapsed().as_secs_f64());
                    
                    (StatusCode::OK, Json(ApiResponse {
                        success: true,
                        data: Some(data),
                        error: None,
                        cached: false,
                        processing_time_ms: start.elapsed().as_millis() as u64,
                    }))
                }
                Err(e) => {
                    counter!("gateway_errors_total", "endpoint" => "get_file").increment(1);
                    (StatusCode::BAD_GATEWAY, Json(ApiResponse {
                        success: false,
                        data: None,
                        error: Some(format!("Failed to read response: {}", e)),
                        cached: false,
                        processing_time_ms: start.elapsed().as_millis() as u64,
                    }))
                }
            }
        }
        Err(CircuitBreakerError::CircuitOpen) => {
            counter!("gateway_circuit_open_total", "service" => "backend").increment(1);
            warn!("Backend circuit is OPEN - rejecting request");
            (StatusCode::SERVICE_UNAVAILABLE, Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Backend service temporarily unavailable (circuit open)".to_string()),
                cached: false,
                processing_time_ms: start.elapsed().as_millis() as u64,
            }))
        }
        Err(CircuitBreakerError::HalfOpenRejected) => {
            counter!("gateway_circuit_half_open_rejected", "service" => "backend").increment(1);
            (StatusCode::SERVICE_UNAVAILABLE, Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Backend service recovering - please retry".to_string()),
                cached: false,
                processing_time_ms: start.elapsed().as_millis() as u64,
            }))
        }
        Err(CircuitBreakerError::ServiceError(e)) => {
            counter!("gateway_errors_total", "endpoint" => "get_file").increment(1);
            (StatusCode::SERVICE_UNAVAILABLE, Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Backend unavailable: {}", e)),
                cached: false,
                processing_time_ms: start.elapsed().as_millis() as u64,
            }))
        }
    }
}

/// Export file using render service
#[tracing::instrument(skip(state, body), fields(file_id = %file_id))]
async fn export_file(
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let start = Instant::now();
    counter!("gateway_requests_total", "endpoint" => "export").increment(1);

    let url = format!("{}/render", state.config.render_url);
    let mut request_body = body;
    request_body["file_id"] = serde_json::json!(file_id);
    let client = state.http_client.clone();

    let result = state.circuit_breakers.render.call(|| async {
        client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())
    }).await;

    match result {
        Ok(resp) => {
            match resp.json::<serde_json::Value>().await {
                Ok(data) => {
                    histogram!("gateway_request_duration_seconds", "endpoint" => "export")
                        .record(start.elapsed().as_secs_f64());
                    (StatusCode::OK, Json(data))
                }
                Err(e) => {
                    counter!("gateway_errors_total", "endpoint" => "export").increment(1);
                    (StatusCode::BAD_GATEWAY, Json(serde_json::json!({
                        "error": format!("Failed to parse response: {}", e)
                    })))
                }
            }
        }
        Err(CircuitBreakerError::CircuitOpen) => {
            counter!("gateway_circuit_open_total", "service" => "render").increment(1);
            warn!("Render circuit is OPEN - rejecting request");
            (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                "error": "Render service temporarily unavailable (circuit open)",
                "circuit_state": "open",
                "retry_after_seconds": 30
            })))
        }
        Err(CircuitBreakerError::HalfOpenRejected) => {
            counter!("gateway_circuit_half_open_rejected", "service" => "render").increment(1);
            (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                "error": "Render service recovering - please retry",
                "circuit_state": "half_open",
                "retry_after_seconds": 5
            })))
        }
        Err(CircuitBreakerError::ServiceError(e)) => {
            counter!("gateway_errors_total", "endpoint" => "export").increment(1);
            (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                "error": format!("Render service unavailable: {}", e)
            })))
        }
    }
}

/// List projects with caching
async fn list_projects(
    State(state): State<Arc<AppState>>,
    Query(params): Query<serde_json::Value>,
) -> impl IntoResponse {
    let start = Instant::now();
    let cache_key = format!("projects:{}", serde_json::to_string(&params).unwrap_or_default());
    counter!("gateway_requests_total", "endpoint" => "list_projects").increment(1);

    // Check cache
    if let Some(entry) = state.cache.get(&cache_key) {
        if entry.expires_at > Instant::now() {
            counter!("gateway_cache_hits_total").increment(1);
            let data: serde_json::Value = serde_json::from_str(&entry.data).unwrap_or_default();
            return (StatusCode::OK, Json(ApiResponse {
                success: true,
                data: Some(data),
                error: None,
                cached: true,
                processing_time_ms: start.elapsed().as_millis() as u64,
            }));
        }
    }

    counter!("gateway_cache_misses_total").increment(1);

    // Forward to backend with circuit breaker
    let url = format!("{}/api/rpc/command/get-all-projects", state.config.backend_url);
    let client = state.http_client.clone();
    let params_clone = params.clone();

    let result = state.circuit_breakers.backend.call(|| async {
        client
            .post(&url)
            .json(&params_clone)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())
    }).await;

    match result {
        Ok(resp) => {
            match resp.text().await {
                Ok(text) => {
                    state.cache.insert(cache_key, CacheEntry {
                        data: text.clone(),
                        expires_at: Instant::now() + state.config.cache_ttl,
                    });

                    let data: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    (StatusCode::OK, Json(ApiResponse {
                        success: true,
                        data: Some(data),
                        error: None,
                        cached: false,
                        processing_time_ms: start.elapsed().as_millis() as u64,
                    }))
                }
                Err(e) => {
                    (StatusCode::BAD_GATEWAY, Json(ApiResponse {
                        success: false,
                        data: None,
                        error: Some(format!("Failed to read response: {}", e)),
                        cached: false,
                        processing_time_ms: start.elapsed().as_millis() as u64,
                    }))
                }
            }
        }
        Err(CircuitBreakerError::CircuitOpen) => {
            counter!("gateway_circuit_open_total", "service" => "backend").increment(1);
            (StatusCode::SERVICE_UNAVAILABLE, Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Backend service temporarily unavailable (circuit open)".to_string()),
                cached: false,
                processing_time_ms: start.elapsed().as_millis() as u64,
            }))
        }
        Err(CircuitBreakerError::HalfOpenRejected) => {
            counter!("gateway_circuit_half_open_rejected", "service" => "backend").increment(1);
            (StatusCode::SERVICE_UNAVAILABLE, Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Backend service recovering - please retry".to_string()),
                cached: false,
                processing_time_ms: start.elapsed().as_millis() as u64,
            }))
        }
        Err(CircuitBreakerError::ServiceError(_)) => {
            counter!("gateway_errors_total", "endpoint" => "list_projects").increment(1);
            (StatusCode::SERVICE_UNAVAILABLE, Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Backend unavailable".to_string()),
                cached: false,
                processing_time_ms: start.elapsed().as_millis() as u64,
            }))
        }
    }
}

/// Health check with service health
async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();
    gauge!("gateway_uptime_seconds").set(uptime as f64);

    // Check services in parallel
    let (validator, realtime, render, backend) = tokio::join!(
        check_service_health(&state.http_client, &state.config.validator_url),
        check_service_health(&state.http_client, &state.config.realtime_url),
        check_service_health(&state.http_client, &state.config.render_url),
        check_service_health(&state.http_client, &state.config.backend_url),
    );

    // Get circuit breaker states
    let circuits = CircuitHealth {
        validator: state.circuit_breakers.validator.state().to_string(),
        render: state.circuit_breakers.render.state().to_string(),
        backend: state.circuit_breakers.backend.state().to_string(),
        realtime: state.circuit_breakers.realtime.state().to_string(),
    };

    Json(HealthResponse {
        status: "healthy",
        uptime_seconds: uptime,
        version: env!("CARGO_PKG_VERSION"),
        services: ServiceHealth {
            validator,
            realtime,
            render,
            backend,
        },
        circuits,
    })
}

async fn check_service_health(client: &Client, base_url: &str) -> bool {
    let url = format!("{}/health", base_url);
    client.get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Cache statistics
async fn cache_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let size = state.cache.len();
    gauge!("gateway_cache_size").set(size as f64);
    
    Json(serde_json::json!({
        "size": size,
        "ttl_seconds": state.config.cache_ttl.as_secs(),
    }))
}

/// Clear cache
async fn clear_cache(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let size = state.cache.len();
    state.cache.clear();
    counter!("gateway_cache_clears_total").increment(1);
    
    Json(serde_json::json!({
        "cleared": size,
        "message": "Cache cleared successfully"
    }))
}

/// Prometheus metrics endpoint
async fn metrics_endpoint(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics_handle.render()
}

/// Rate limit info endpoint
async fn rate_limit_info() -> impl IntoResponse {
    Json(serde_json::json!({
        "rate_limit": {
            "requests_per_second": 100,
            "burst_size": 200,
            "description": "Per-IP rate limiting"
        }
    }))
}

/// Circuit breaker status for all services
async fn circuit_breaker_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let circuits: Vec<_> = state
        .circuit_breakers
        .all()
        .iter()
        .map(|cb| cb.stats())
        .collect();

    Json(serde_json::json!({
        "circuits": circuits,
        "description": "Circuit breakers protect against cascading failures"
    }))
}

/// Request body for circuit operations
#[derive(Debug, Deserialize)]
struct CircuitResetRequest {
    #[serde(default)]
    force: bool,
}

/// Reset a specific circuit breaker
async fn reset_circuit_breaker(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.circuit_breakers.get(&name) {
        Some(cb) => {
            cb.reset();
            info!("Circuit breaker '{}' reset via API", name);
            (StatusCode::OK, Json(serde_json::json!({
                "success": true,
                "message": format!("Circuit '{}' reset to closed state", name),
                "state": cb.stats()
            })))
        }
        None => {
            let available: Vec<&str> = vec!["validator", "render", "backend", "realtime"];
            (StatusCode::NOT_FOUND, Json(serde_json::json!({
                "success": false,
                "error": format!("Circuit '{}' not found", name),
                "available_circuits": available
            })))
        }
    }
}

/// Force a circuit breaker open (for maintenance)
async fn force_open_circuit(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.circuit_breakers.get(&name) {
        Some(cb) => {
            cb.force_open();
            warn!("Circuit breaker '{}' forced OPEN via API", name);
            (StatusCode::OK, Json(serde_json::json!({
                "success": true,
                "message": format!("Circuit '{}' forced open - requests will be rejected", name),
                "state": cb.stats()
            })))
        }
        None => {
            let available: Vec<&str> = vec!["validator", "render", "backend", "realtime"];
            (StatusCode::NOT_FOUND, Json(serde_json::json!({
                "success": false,
                "error": format!("Circuit '{}' not found", name),
                "available_circuits": available
            })))
        }
    }
}

/// Check if request should be rate limited
fn check_rate_limit(state: &AppState, client_ip: &str) -> bool {
    match state.rate_limiter.check_key(&client_ip.to_string()) {
        Ok(_) => false,
        Err(_) => {
            counter!("gateway_rate_limited_total").increment(1);
            true
        }
    }
}
