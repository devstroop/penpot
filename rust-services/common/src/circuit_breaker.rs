//! Circuit Breaker Pattern Implementation
//!
//! Provides resilience patterns for handling failures in distributed systems.
//! Prevents cascading failures by stopping requests to failing services.
//!
//! # States
//!
//! - **Closed**: Normal operation, requests pass through
//! - **Open**: Service failing, requests are rejected immediately
//! - **HalfOpen**: Testing if service recovered, limited requests allowed
//!
//! # Example
//!
//! ```ignore
//! use common::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
//!
//! let cb = CircuitBreaker::new("backend-service", CircuitBreakerConfig::default());
//!
//! // Wrap calls with circuit breaker
//! let result = cb.call(|| async {
//!     client.get("http://backend/api").await
//! }).await;
//! ```

use metrics::{counter, gauge};
use serde::Serialize;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, info, warn};

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CircuitState {
    /// Normal operation - requests pass through
    Closed,
    /// Service failing - requests rejected immediately
    Open,
    /// Testing recovery - limited requests allowed
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitState::Closed => write!(f, "closed"),
            CircuitState::Open => write!(f, "open"),
            CircuitState::HalfOpen => write!(f, "half_open"),
        }
    }
}

/// Circuit breaker configuration
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening circuit
    pub failure_threshold: u32,
    /// Number of successes in half-open to close circuit
    pub success_threshold: u32,
    /// Time to wait before transitioning from open to half-open
    pub timeout: Duration,
    /// Time window for counting failures
    pub failure_window: Duration,
    /// Maximum concurrent requests in half-open state
    pub half_open_max_requests: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 3,
            timeout: Duration::from_secs(30),
            failure_window: Duration::from_secs(60),
            half_open_max_requests: 3,
        }
    }
}

impl CircuitBreakerConfig {
    /// Create a strict config for critical services
    pub fn strict() -> Self {
        Self {
            failure_threshold: 3,
            success_threshold: 5,
            timeout: Duration::from_secs(60),
            failure_window: Duration::from_secs(30),
            half_open_max_requests: 1,
        }
    }

    /// Create a lenient config for non-critical services
    pub fn lenient() -> Self {
        Self {
            failure_threshold: 10,
            success_threshold: 2,
            timeout: Duration::from_secs(15),
            failure_window: Duration::from_secs(120),
            half_open_max_requests: 5,
        }
    }
}

/// Circuit breaker errors
#[derive(Debug, Error)]
pub enum CircuitBreakerError {
    #[error("Circuit is open - service unavailable")]
    CircuitOpen,

    #[error("Request rejected - circuit half-open, max requests reached")]
    HalfOpenRejected,

    #[error("Service error: {0}")]
    ServiceError(String),
}

/// Internal state for atomic operations
struct InternalState {
    /// Current state (0=Closed, 1=Open, 2=HalfOpen)
    state: AtomicU32,
    /// Failure count in current window
    failure_count: AtomicU32,
    /// Success count (used in half-open)
    success_count: AtomicU32,
    /// Timestamp when circuit opened (unix millis)
    opened_at: AtomicU64,
    /// Window start timestamp (unix millis)
    window_start: AtomicU64,
    /// Current half-open requests
    half_open_requests: AtomicU32,
}

/// Circuit breaker for resilient service calls
pub struct CircuitBreaker {
    name: String,
    config: CircuitBreakerConfig,
    state: Arc<InternalState>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker
    pub fn new(name: impl Into<String>, config: CircuitBreakerConfig) -> Arc<Self> {
        let name = name.into();
        info!("Creating circuit breaker: {} (threshold: {}, timeout: {:?})",
            name, config.failure_threshold, config.timeout);

        let now = Self::now_millis();

        Arc::new(Self {
            name,
            config,
            state: Arc::new(InternalState {
                state: AtomicU32::new(0), // Closed
                failure_count: AtomicU32::new(0),
                success_count: AtomicU32::new(0),
                opened_at: AtomicU64::new(0),
                window_start: AtomicU64::new(now),
                half_open_requests: AtomicU32::new(0),
            }),
        })
    }

    fn now_millis() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    /// Get current circuit state
    pub fn state(&self) -> CircuitState {
        match self.state.state.load(Ordering::SeqCst) {
            0 => CircuitState::Closed,
            1 => CircuitState::Open,
            _ => CircuitState::HalfOpen,
        }
    }

    /// Check if circuit should transition from open to half-open
    fn check_timeout(&self) -> bool {
        let opened_at = self.state.opened_at.load(Ordering::SeqCst);
        if opened_at == 0 {
            return false;
        }
        let elapsed = Self::now_millis() - opened_at;
        elapsed >= self.config.timeout.as_millis() as u64
    }

    /// Check if failure window has expired and reset if needed
    fn check_window(&self) {
        let window_start = self.state.window_start.load(Ordering::SeqCst);
        let elapsed = Self::now_millis() - window_start;
        
        if elapsed >= self.config.failure_window.as_millis() as u64 {
            // Reset window
            self.state.failure_count.store(0, Ordering::SeqCst);
            self.state.window_start.store(Self::now_millis(), Ordering::SeqCst);
            debug!("Circuit breaker '{}': failure window reset", self.name);
        }
    }

    /// Check if request is allowed
    pub fn allow_request(&self) -> Result<(), CircuitBreakerError> {
        let current_state = self.state();

        match current_state {
            CircuitState::Closed => {
                self.check_window();
                Ok(())
            }
            CircuitState::Open => {
                if self.check_timeout() {
                    // Transition to half-open
                    self.state.state.store(2, Ordering::SeqCst);
                    self.state.success_count.store(0, Ordering::SeqCst);
                    self.state.half_open_requests.store(0, Ordering::SeqCst);
                    
                    info!("Circuit breaker '{}': transitioning to half-open", self.name);
                    self.record_state_change(CircuitState::HalfOpen);
                    
                    // Allow this request
                    self.state.half_open_requests.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                } else {
                    counter!("circuit_breaker_rejected", "name" => self.name.clone(), "reason" => "open").increment(1);
                    Err(CircuitBreakerError::CircuitOpen)
                }
            }
            CircuitState::HalfOpen => {
                let current = self.state.half_open_requests.load(Ordering::SeqCst);
                if current >= self.config.half_open_max_requests {
                    counter!("circuit_breaker_rejected", "name" => self.name.clone(), "reason" => "half_open_limit").increment(1);
                    Err(CircuitBreakerError::HalfOpenRejected)
                } else {
                    self.state.half_open_requests.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            }
        }
    }

    /// Record a successful request
    pub fn record_success(&self) {
        counter!("circuit_breaker_success", "name" => self.name.clone()).increment(1);
        
        let current_state = self.state();
        
        if current_state == CircuitState::HalfOpen {
            let successes = self.state.success_count.fetch_add(1, Ordering::SeqCst) + 1;
            
            if successes >= self.config.success_threshold {
                // Transition to closed
                self.state.state.store(0, Ordering::SeqCst);
                self.state.failure_count.store(0, Ordering::SeqCst);
                self.state.window_start.store(Self::now_millis(), Ordering::SeqCst);
                
                info!("Circuit breaker '{}': recovered, transitioning to closed", self.name);
                self.record_state_change(CircuitState::Closed);
            }
        }
    }

    /// Record a failed request
    pub fn record_failure(&self) {
        counter!("circuit_breaker_failure", "name" => self.name.clone()).increment(1);
        
        let current_state = self.state();
        
        match current_state {
            CircuitState::Closed => {
                self.check_window();
                let failures = self.state.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
                
                if failures >= self.config.failure_threshold {
                    // Transition to open
                    self.state.state.store(1, Ordering::SeqCst);
                    self.state.opened_at.store(Self::now_millis(), Ordering::SeqCst);
                    
                    warn!("Circuit breaker '{}': opening after {} failures", self.name, failures);
                    self.record_state_change(CircuitState::Open);
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open immediately opens circuit
                self.state.state.store(1, Ordering::SeqCst);
                self.state.opened_at.store(Self::now_millis(), Ordering::SeqCst);
                
                warn!("Circuit breaker '{}': failure in half-open, reopening", self.name);
                self.record_state_change(CircuitState::Open);
            }
            CircuitState::Open => {
                // Already open, nothing to do
            }
        }
    }

    fn record_state_change(&self, new_state: CircuitState) {
        let state_value = match new_state {
            CircuitState::Closed => 0.0,
            CircuitState::Open => 1.0,
            CircuitState::HalfOpen => 0.5,
        };
        gauge!("circuit_breaker_state", "name" => self.name.clone()).set(state_value);
    }

    /// Execute a fallible async operation with circuit breaker protection
    pub async fn call<F, Fut, T, E>(&self, f: F) -> Result<T, CircuitBreakerError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        // Check if request is allowed
        self.allow_request()?;

        // Execute the operation
        match f().await {
            Ok(result) => {
                self.record_success();
                Ok(result)
            }
            Err(e) => {
                self.record_failure();
                Err(CircuitBreakerError::ServiceError(e.to_string()))
            }
        }
    }

    /// Get circuit breaker statistics
    pub fn stats(&self) -> CircuitBreakerStats {
        CircuitBreakerStats {
            name: self.name.clone(),
            state: self.state(),
            failure_count: self.state.failure_count.load(Ordering::SeqCst),
            success_count: self.state.success_count.load(Ordering::SeqCst),
            config: self.config.clone(),
        }
    }

    /// Manually reset the circuit breaker to closed state
    pub fn reset(&self) {
        self.state.state.store(0, Ordering::SeqCst);
        self.state.failure_count.store(0, Ordering::SeqCst);
        self.state.success_count.store(0, Ordering::SeqCst);
        self.state.opened_at.store(0, Ordering::SeqCst);
        self.state.window_start.store(Self::now_millis(), Ordering::SeqCst);
        self.state.half_open_requests.store(0, Ordering::SeqCst);
        
        info!("Circuit breaker '{}': manually reset to closed", self.name);
        self.record_state_change(CircuitState::Closed);
    }

    /// Force the circuit open (useful for maintenance)
    pub fn force_open(&self) {
        self.state.state.store(1, Ordering::SeqCst);
        self.state.opened_at.store(Self::now_millis(), Ordering::SeqCst);
        
        warn!("Circuit breaker '{}': forced open", self.name);
        self.record_state_change(CircuitState::Open);
    }
}

/// Circuit breaker statistics
#[derive(Debug, Clone, Serialize)]
pub struct CircuitBreakerStats {
    pub name: String,
    pub state: CircuitState,
    pub failure_count: u32,
    pub success_count: u32,
    #[serde(skip)]
    pub config: CircuitBreakerConfig,
}

/// Retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_attempts: u32,
    /// Initial delay between retries
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Multiplier for exponential backoff
    pub multiplier: f64,
    /// Add jitter to delays
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            multiplier: 2.0,
            jitter: true,
        }
    }
}

/// Execute with retries and exponential backoff
#[cfg(feature = "resilience")]
pub async fn retry_with_backoff<F, Fut, T, E>(
    config: &RetryConfig,
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempts = 0;
    let mut delay = config.initial_delay;

    loop {
        attempts += 1;
        
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) if attempts >= config.max_attempts => {
                warn!("All {} retry attempts exhausted: {}", attempts, e);
                return Err(e);
            }
            Err(e) => {
                debug!("Attempt {} failed: {}, retrying in {:?}", attempts, e, delay);
                
                // Add jitter if enabled
                let actual_delay = if config.jitter {
                    let jitter_range = delay.as_millis() as f64 * 0.2;
                    let jitter = (rand_simple() * jitter_range * 2.0 - jitter_range) as u64;
                    Duration::from_millis(delay.as_millis() as u64 + jitter)
                } else {
                    delay
                };

                tokio::time::sleep(actual_delay).await;

                // Calculate next delay with exponential backoff
                delay = Duration::from_millis(
                    (delay.as_millis() as f64 * config.multiplier) as u64
                );
                if delay > config.max_delay {
                    delay = config.max_delay;
                }
            }
        }
    }
}

/// Simple random number generator (no external dependency)
fn rand_simple() -> f64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    (nanos % 1000) as f64 / 1000.0
}

/// Timeout wrapper for async operations
#[cfg(feature = "resilience")]
pub async fn with_timeout<F, T>(
    duration: Duration,
    f: F,
) -> Result<T, TimeoutError>
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(duration, f)
        .await
        .map_err(|_| TimeoutError::Elapsed(duration))
}

/// Timeout error
#[derive(Debug, Error)]
pub enum TimeoutError {
    #[error("Operation timed out after {0:?}")]
    Elapsed(Duration),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_initial_state() {
        let cb = CircuitBreaker::new("test", CircuitBreakerConfig::default());
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_circuit_opens_after_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let cb = CircuitBreaker::new("test", config);

        // Record failures up to threshold
        for _ in 0..3 {
            cb.record_failure();
        }

        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn test_circuit_rejects_when_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            timeout: Duration::from_secs(60), // Long timeout
            ..Default::default()
        };
        let cb = CircuitBreaker::new("test", config);

        cb.record_failure(); // Opens circuit
        assert!(cb.allow_request().is_err());
    }

    #[test]
    fn test_manual_reset() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            ..Default::default()
        };
        let cb = CircuitBreaker::new("test", config);

        cb.record_failure(); // Opens circuit
        assert_eq!(cb.state(), CircuitState::Open);

        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request().is_ok());
    }

    #[test]
    fn test_stats() {
        let cb = CircuitBreaker::new("test-service", CircuitBreakerConfig::default());
        cb.record_failure();
        cb.record_failure();

        let stats = cb.stats();
        assert_eq!(stats.name, "test-service");
        assert_eq!(stats.failure_count, 2);
    }
}
