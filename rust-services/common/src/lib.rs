//! Common types and utilities for Penpot Rust services
//!
//! This crate provides shared functionality across all Rust microservices.

pub mod bridge;
#[cfg(feature = "cache")]
pub mod cache;
pub mod circuit_breaker;
#[cfg(feature = "database")]
pub mod database;
pub mod error;
pub mod telemetry;
pub mod types;
pub mod validation;

pub use bridge::{BridgeConfig, BridgeError, ClojureBridge, FeatureFlags, PenpotFile, PenpotProject};
#[cfg(feature = "cache")]
pub use cache::{CacheConfig, CacheError, CacheInfo, DistributedCache};
pub use circuit_breaker::{
    CircuitBreaker, CircuitBreakerConfig, CircuitBreakerError, CircuitBreakerStats, CircuitState,
    RetryConfig,
};
#[cfg(feature = "resilience")]
pub use circuit_breaker::{retry_with_backoff, with_timeout, TimeoutError};
#[cfg(feature = "database")]
pub use database::{DatabaseConfig, DatabaseError, DatabasePool, PoolStats, QueryExt};
pub use error::{Error, Result};
pub use telemetry::{init_telemetry, TelemetryConfig, TelemetryGuard};
pub use types::*;
