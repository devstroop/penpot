//! Render Service
//!
//! High-performance server-side rendering for exports and thumbnails.
//! Uses resvg for SVG rendering and tiny-skia for rasterization.
//!
//! ## Endpoints
//!
//! - `POST /render` - Render SVG to PNG/PDF
//! - `POST /thumbnail` - Generate thumbnail
//! - `POST /render-svg` - Render raw SVG string
//! - `GET /health` - Health check
//! - `GET /metrics` - Prometheus metrics

use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use common::{init_telemetry, TelemetryConfig};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tiny_skia::Pixmap;
use tokio::signal;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use uuid::Uuid;

/// Application state
struct AppState {
    start_time: Instant,
    metrics_handle: PrometheusHandle,
    font_db: usvg::fontdb::Database,
}

/// Render request
#[derive(Debug, Deserialize)]
struct RenderRequest {
    file_id: Option<Uuid>,
    page_id: Option<Uuid>,
    svg: Option<String>,
    format: RenderFormat,
    width: Option<u32>,
    height: Option<u32>,
    scale: Option<f32>,
    background: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum RenderFormat {
    Png,
    Svg,
    Pdf,
}

/// Render response (JSON)
#[derive(Debug, Serialize)]
struct RenderJsonResponse {
    success: bool,
    format: String,
    width: u32,
    height: u32,
    data: Option<String>, // Base64 encoded
    error: Option<String>,
    processing_time_ms: u64,
}

/// Thumbnail request
#[derive(Debug, Deserialize)]
struct ThumbnailRequest {
    svg: String,
    max_width: Option<u32>,
    max_height: Option<u32>,
}

/// Health response
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    uptime_seconds: u64,
    version: &'static str,
    capabilities: Vec<&'static str>,
}

#[tokio::main]
async fn main() {
    // Initialize telemetry (tracing + OpenTelemetry)
    let _telemetry = init_telemetry(TelemetryConfig::for_service("render-service"));

    // Initialize Prometheus metrics
    let metrics_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("Failed to install Prometheus recorder");

    // Initialize font database
    let mut font_db = usvg::fontdb::Database::new();
    font_db.load_system_fonts();
    info!("Loaded {} system fonts", font_db.len());

    let state = Arc::new(AppState {
        start_time: Instant::now(),
        metrics_handle,
        font_db,
    });

    let app = Router::new()
        .route("/render", post(render_handler))
        .route("/render-svg", post(render_svg_handler))
        .route("/thumbnail", post(thumbnail_handler))
        .route("/health", get(health_check))
        .route("/metrics", get(metrics_endpoint))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8083")
        .await
        .expect("Failed to bind to port 8083");

    info!("🚀 Render Service running on http://0.0.0.0:8083");
    info!("   POST /render      - Render SVG to image");
    info!("   POST /render-svg  - Render raw SVG string");
    info!("   POST /thumbnail   - Generate thumbnail");
    info!("   GET  /health      - Health check");
    info!("   GET  /metrics     - Prometheus metrics");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Failed to start server");

    info!("🛑 Render Service shut down gracefully");
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

/// Main render handler
#[tracing::instrument(skip(state, request), fields(format = ?request.format))]
async fn render_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RenderRequest>,
) -> impl IntoResponse {
    let start = Instant::now();
    counter!("render_requests_total").increment(1);

    // For now, we need an SVG string
    let svg_data = match &request.svg {
        Some(svg) => svg.clone(),
        None => {
            // In production, this would fetch SVG from file storage
            return (
                StatusCode::BAD_REQUEST,
                Json(RenderJsonResponse {
                    success: false,
                    format: format!("{:?}", request.format).to_lowercase(),
                    width: 0,
                    height: 0,
                    data: None,
                    error: Some("SVG data required (file_id/page_id rendering not yet implemented)".to_string()),
                    processing_time_ms: start.elapsed().as_millis() as u64,
                }),
            );
        }
    };

    match render_svg_to_format(&state, &svg_data, &request) {
        Ok((data, width, height)) => {
            histogram!("render_processing_seconds").record(start.elapsed().as_secs_f64());
            counter!("render_success_total").increment(1);

            (
                StatusCode::OK,
                Json(RenderJsonResponse {
                    success: true,
                    format: format!("{:?}", request.format).to_lowercase(),
                    width,
                    height,
                    data: Some(BASE64.encode(&data)),
                    error: None,
                    processing_time_ms: start.elapsed().as_millis() as u64,
                }),
            )
        }
        Err(e) => {
            counter!("render_errors_total").increment(1);
            warn!("Render error: {}", e);

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RenderJsonResponse {
                    success: false,
                    format: format!("{:?}", request.format).to_lowercase(),
                    width: 0,
                    height: 0,
                    data: None,
                    error: Some(e.to_string()),
                    processing_time_ms: start.elapsed().as_millis() as u64,
                }),
            )
        }
    }
}

/// Render raw SVG and return binary
async fn render_svg_handler(
    State(state): State<Arc<AppState>>,
    body: String,
) -> impl IntoResponse {
    let start = Instant::now();
    counter!("render_svg_requests_total").increment(1);

    let request = RenderRequest {
        file_id: None,
        page_id: None,
        svg: Some(body),
        format: RenderFormat::Png,
        width: None,
        height: None,
        scale: Some(1.0),
        background: None,
    };

    match render_svg_to_format(&state, request.svg.as_ref().unwrap(), &request) {
        Ok((data, _, _)) => {
            histogram!("render_processing_seconds").record(start.elapsed().as_secs_f64());
            
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .body(Body::from(data))
                .unwrap()
        }
        Err(e) => {
            counter!("render_errors_total").increment(1);
            
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(format!("Render error: {}", e)))
                .unwrap()
        }
    }
}

/// Generate thumbnail
#[tracing::instrument(skip(state, request))]
async fn thumbnail_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ThumbnailRequest>,
) -> impl IntoResponse {
    let start = Instant::now();
    counter!("thumbnail_requests_total").increment(1);

    let max_width = request.max_width.unwrap_or(256);
    let max_height = request.max_height.unwrap_or(256);

    // Parse SVG to get dimensions
    let opts = usvg::Options::default();
    let tree = match usvg::Tree::from_str(&request.svg, &opts) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RenderJsonResponse {
                    success: false,
                    format: "png".to_string(),
                    width: 0,
                    height: 0,
                    data: None,
                    error: Some(format!("Invalid SVG: {}", e)),
                    processing_time_ms: start.elapsed().as_millis() as u64,
                }),
            );
        }
    };

    let svg_size = tree.size();
    let svg_width = svg_size.width();
    let svg_height = svg_size.height();

    // Calculate scale to fit within max dimensions
    let scale_x = max_width as f32 / svg_width;
    let scale_y = max_height as f32 / svg_height;
    let scale = scale_x.min(scale_y).min(1.0); // Don't upscale

    let render_request = RenderRequest {
        file_id: None,
        page_id: None,
        svg: Some(request.svg),
        format: RenderFormat::Png,
        width: Some((svg_width * scale) as u32),
        height: Some((svg_height * scale) as u32),
        scale: Some(scale),
        background: None,
    };

    match render_svg_to_format(&state, render_request.svg.as_ref().unwrap(), &render_request) {
        Ok((data, width, height)) => {
            histogram!("thumbnail_processing_seconds").record(start.elapsed().as_secs_f64());
            
            (
                StatusCode::OK,
                Json(RenderJsonResponse {
                    success: true,
                    format: "png".to_string(),
                    width,
                    height,
                    data: Some(BASE64.encode(&data)),
                    error: None,
                    processing_time_ms: start.elapsed().as_millis() as u64,
                }),
            )
        }
        Err(e) => {
            counter!("thumbnail_errors_total").increment(1);
            
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RenderJsonResponse {
                    success: false,
                    format: "png".to_string(),
                    width: 0,
                    height: 0,
                    data: None,
                    error: Some(e.to_string()),
                    processing_time_ms: start.elapsed().as_millis() as u64,
                }),
            )
        }
    }
}

/// Core SVG rendering function
fn render_svg_to_format(
    state: &AppState,
    svg_data: &str,
    request: &RenderRequest,
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error + Send + Sync>> {
    // Parse SVG
    let opts = usvg::Options {
        fontdb: Arc::new(state.font_db.clone()),
        ..Default::default()
    };
    
    let tree = usvg::Tree::from_str(svg_data, &opts)?;
    let svg_size = tree.size();

    // Calculate dimensions
    let scale = request.scale.unwrap_or(1.0);
    let width = request.width.unwrap_or((svg_size.width() * scale) as u32);
    let height = request.height.unwrap_or((svg_size.height() * scale) as u32);

    match request.format {
        RenderFormat::Png => {
            // Create pixmap
            let mut pixmap = Pixmap::new(width, height)
                .ok_or("Failed to create pixmap")?;

            // Optional background
            if let Some(bg) = &request.background {
                if let Some(color) = parse_color(bg) {
                    pixmap.fill(color);
                }
            }

            // Render SVG
            let transform = tiny_skia::Transform::from_scale(
                width as f32 / svg_size.width(),
                height as f32 / svg_size.height(),
            );

            resvg::render(&tree, transform, &mut pixmap.as_mut());

            // Encode to PNG
            let png_data = pixmap.encode_png()?;
            
            histogram!("render_output_bytes").record(png_data.len() as f64);
            Ok((png_data, width, height))
        }
        RenderFormat::Svg => {
            // Just return the original SVG
            Ok((svg_data.as_bytes().to_vec(), width, height))
        }
        RenderFormat::Pdf => {
            // PDF rendering would require additional dependencies
            Err("PDF rendering not yet implemented".into())
        }
    }
}

/// Parse hex color to tiny_skia Color
fn parse_color(hex: &str) -> Option<tiny_skia::Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(tiny_skia::Color::from_rgba8(r, g, b, 255))
    } else if hex.len() == 8 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
        Some(tiny_skia::Color::from_rgba8(r, g, b, a))
    } else {
        None
    }
}

/// Health check
async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();
    gauge!("render_uptime_seconds").set(uptime as f64);

    Json(HealthResponse {
        status: "healthy",
        uptime_seconds: uptime,
        version: env!("CARGO_PKG_VERSION"),
        capabilities: vec!["png", "svg", "thumbnail"],
    })
}

/// Prometheus metrics endpoint
async fn metrics_endpoint(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics_handle.render()
}
