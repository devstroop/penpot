//! Telemetry configuration for distributed tracing
//!
//! Provides OpenTelemetry integration for all Penpot Rust services.
//! Supports both local development (console output) and production (OTLP export).

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{runtime, trace as sdktrace, Resource};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Telemetry configuration
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Service name for tracing
    pub service_name: String,
    /// OTLP endpoint (e.g., "http://localhost:4317")
    pub otlp_endpoint: Option<String>,
    /// Log level filter
    pub log_level: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "penpot-service".to_string(),
            otlp_endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok(),
            log_level: std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        }
    }
}

impl TelemetryConfig {
    /// Create config for a specific service
    pub fn for_service(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_string(),
            ..Default::default()
        }
    }
}

/// Initialize telemetry with optional OpenTelemetry tracing
///
/// Returns a guard that should be held until shutdown.
pub fn init_telemetry(config: TelemetryConfig) -> Option<TelemetryGuard> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .compact();

    // Check if OTLP endpoint is configured
    if let Some(endpoint) = &config.otlp_endpoint {
        match init_otlp_provider(&config.service_name, endpoint) {
            Ok(provider) => {
                let tracer = provider.tracer(config.service_name.clone());
                let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt_layer)
                    .with(otel_layer)
                    .init();

                tracing::info!(
                    service = %config.service_name,
                    endpoint = %endpoint,
                    "OpenTelemetry tracing initialized"
                );

                return Some(TelemetryGuard {
                    provider: Some(provider),
                });
            }
            Err(e) => {
                eprintln!(
                    "Failed to initialize OTLP tracing: {}. Using console only.",
                    e
                );
            }
        }
    }

    // Console-only mode
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    tracing::info!(
        service = %config.service_name,
        "Telemetry initialized (console mode)"
    );

    None
}

fn init_otlp_provider(
    service_name: &str,
    endpoint: &str,
) -> Result<sdktrace::TracerProvider, opentelemetry::trace::TraceError> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;

    let provider = sdktrace::TracerProvider::builder()
        .with_batch_exporter(exporter, runtime::Tokio)
        .with_resource(Resource::new(vec![
            opentelemetry::KeyValue::new("service.name", service_name.to_string()),
            opentelemetry::KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
        ]))
        .build();

    Ok(provider)
}

/// Guard that shuts down telemetry on drop
pub struct TelemetryGuard {
    provider: Option<sdktrace::TracerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            if let Err(e) = provider.shutdown() {
                eprintln!("Error shutting down tracer provider: {:?}", e);
            }
        }
    }
}

/// Tracing instrumentation helpers
pub mod instrument {
    /// Instrument an async function with a span
    pub use tracing::instrument;
}
