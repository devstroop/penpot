//! Distributed caching with Redis/Valkey
//!
//! Provides a high-performance distributed cache for sharing state
//! across multiple service instances.
//!
//! # Example
//!
//! ```ignore
//! use common::cache::{CacheConfig, DistributedCache};
//!
//! let config = CacheConfig::from_env();
//! let cache = DistributedCache::new(config).await?;
//!
//! // Set with TTL
//! cache.set("key", "value", 60).await?;
//!
//! // Get
//! let value: Option<String> = cache.get("key").await?;
//!
//! // Delete
//! cache.delete("key").await?;
//! ```

use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client, RedisError};
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Redis/Valkey URL
    pub url: String,
    /// Key prefix for namespacing
    pub key_prefix: String,
    /// Default TTL in seconds
    pub default_ttl_secs: u64,
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Response timeout
    pub response_timeout: Duration,
}

impl CacheConfig {
    /// Create config from environment variables
    pub fn from_env() -> Self {
        let url = std::env::var("REDIS_URL")
            .or_else(|_| std::env::var("CACHE_URL"))
            .unwrap_or_else(|_| "redis://localhost:6379".to_string());

        let key_prefix = std::env::var("CACHE_KEY_PREFIX")
            .unwrap_or_else(|_| "penpot".to_string());

        let default_ttl_secs = std::env::var("CACHE_DEFAULT_TTL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);

        let connect_timeout_ms: u64 = std::env::var("CACHE_CONNECT_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5000);

        let response_timeout_ms: u64 = std::env::var("CACHE_RESPONSE_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000);

        Self {
            url,
            key_prefix,
            default_ttl_secs,
            connect_timeout: Duration::from_millis(connect_timeout_ms),
            response_timeout: Duration::from_millis(response_timeout_ms),
        }
    }

    /// Create config with a specific URL
    pub fn with_url(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            key_prefix: "penpot".to_string(),
            default_ttl_secs: 300,
            connect_timeout: Duration::from_secs(5),
            response_timeout: Duration::from_secs(1),
        }
    }
}

/// Distributed cache backed by Redis/Valkey
pub struct DistributedCache {
    conn: ConnectionManager,
    config: CacheConfig,
}

impl DistributedCache {
    /// Create a new distributed cache
    pub async fn new(config: CacheConfig) -> Result<Arc<Self>, CacheError> {
        info!("Connecting to Redis at {}", config.url);

        let client = Client::open(config.url.as_str())
            .map_err(|e| CacheError::Connection(format!("Invalid Redis URL: {}", e)))?;

        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| CacheError::Connection(format!("Failed to connect: {}", e)))?;

        // Test connection
        let mut test_conn = conn.clone();
        let _: String = redis::cmd("PING")
            .query_async(&mut test_conn)
            .await
            .map_err(|e| CacheError::Connection(format!("Ping failed: {}", e)))?;

        info!("Connected to Redis successfully");

        Ok(Arc::new(Self { conn, config }))
    }

    /// Build full key with prefix
    fn full_key(&self, key: &str) -> String {
        format!("{}:{}", self.config.key_prefix, key)
    }

    /// Get a value from cache
    #[tracing::instrument(skip(self), fields(key = %key))]
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, CacheError> {
        let full_key = self.full_key(key);
        let mut conn = self.conn.clone();

        let result: Option<String> = conn
            .get(&full_key)
            .await
            .map_err(|e| CacheError::Operation(format!("GET failed: {}", e)))?;

        match result {
            Some(data) => {
                let value: T = serde_json::from_str(&data)
                    .map_err(|e| CacheError::Serialization(format!("Deserialize failed: {}", e)))?;
                debug!("Cache HIT: {}", key);
                Ok(Some(value))
            }
            None => {
                debug!("Cache MISS: {}", key);
                Ok(None)
            }
        }
    }

    /// Get raw string from cache
    #[tracing::instrument(skip(self), fields(key = %key))]
    pub async fn get_string(&self, key: &str) -> Result<Option<String>, CacheError> {
        let full_key = self.full_key(key);
        let mut conn = self.conn.clone();

        conn.get(&full_key)
            .await
            .map_err(|e| CacheError::Operation(format!("GET failed: {}", e)))
    }

    /// Set a value in cache with TTL
    #[tracing::instrument(skip(self, value), fields(key = %key, ttl_secs = %ttl_secs))]
    pub async fn set<T: Serialize>(&self, key: &str, value: &T, ttl_secs: u64) -> Result<(), CacheError> {
        let full_key = self.full_key(key);
        let mut conn = self.conn.clone();

        let data = serde_json::to_string(value)
            .map_err(|e| CacheError::Serialization(format!("Serialize failed: {}", e)))?;

        conn.set_ex(&full_key, &data, ttl_secs)
            .await
            .map_err(|e| CacheError::Operation(format!("SET failed: {}", e)))?;

        debug!("Cache SET: {} (TTL: {}s)", key, ttl_secs);
        Ok(())
    }

    /// Set raw string in cache with TTL
    #[tracing::instrument(skip(self, value), fields(key = %key, ttl_secs = %ttl_secs))]
    pub async fn set_string(&self, key: &str, value: &str, ttl_secs: u64) -> Result<(), CacheError> {
        let full_key = self.full_key(key);
        let mut conn = self.conn.clone();

        conn.set_ex(&full_key, value, ttl_secs)
            .await
            .map_err(|e| CacheError::Operation(format!("SET failed: {}", e)))
    }

    /// Set a value with default TTL
    pub async fn set_default<T: Serialize>(&self, key: &str, value: &T) -> Result<(), CacheError> {
        self.set(key, value, self.config.default_ttl_secs).await
    }

    /// Delete a key from cache
    #[tracing::instrument(skip(self), fields(key = %key))]
    pub async fn delete(&self, key: &str) -> Result<bool, CacheError> {
        let full_key = self.full_key(key);
        let mut conn = self.conn.clone();

        let deleted: i64 = conn
            .del(&full_key)
            .await
            .map_err(|e| CacheError::Operation(format!("DEL failed: {}", e)))?;

        debug!("Cache DEL: {} (deleted: {})", key, deleted > 0);
        Ok(deleted > 0)
    }

    /// Delete multiple keys matching a pattern
    #[tracing::instrument(skip(self), fields(pattern = %pattern))]
    pub async fn delete_pattern(&self, pattern: &str) -> Result<usize, CacheError> {
        let full_pattern = self.full_key(pattern);
        let mut conn = self.conn.clone();

        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&full_pattern)
            .query_async(&mut conn)
            .await
            .map_err(|e| CacheError::Operation(format!("KEYS failed: {}", e)))?;

        if keys.is_empty() {
            return Ok(0);
        }

        let deleted: i64 = conn
            .del(&keys)
            .await
            .map_err(|e| CacheError::Operation(format!("DEL failed: {}", e)))?;

        debug!("Cache DEL pattern: {} (deleted: {})", pattern, deleted);
        Ok(deleted as usize)
    }

    /// Check if a key exists
    #[tracing::instrument(skip(self), fields(key = %key))]
    pub async fn exists(&self, key: &str) -> Result<bool, CacheError> {
        let full_key = self.full_key(key);
        let mut conn = self.conn.clone();

        let exists: bool = conn
            .exists(&full_key)
            .await
            .map_err(|e| CacheError::Operation(format!("EXISTS failed: {}", e)))?;

        Ok(exists)
    }

    /// Set TTL on existing key
    #[tracing::instrument(skip(self), fields(key = %key, ttl_secs = %ttl_secs))]
    pub async fn expire(&self, key: &str, ttl_secs: u64) -> Result<bool, CacheError> {
        let full_key = self.full_key(key);
        let mut conn = self.conn.clone();

        let set: bool = conn
            .expire(&full_key, ttl_secs as i64)
            .await
            .map_err(|e| CacheError::Operation(format!("EXPIRE failed: {}", e)))?;

        Ok(set)
    }

    /// Get remaining TTL for a key
    #[tracing::instrument(skip(self), fields(key = %key))]
    pub async fn ttl(&self, key: &str) -> Result<Option<i64>, CacheError> {
        let full_key = self.full_key(key);
        let mut conn = self.conn.clone();

        let ttl: i64 = conn
            .ttl(&full_key)
            .await
            .map_err(|e| CacheError::Operation(format!("TTL failed: {}", e)))?;

        if ttl < 0 {
            Ok(None)
        } else {
            Ok(Some(ttl))
        }
    }

    /// Increment a counter
    #[tracing::instrument(skip(self), fields(key = %key))]
    pub async fn incr(&self, key: &str) -> Result<i64, CacheError> {
        let full_key = self.full_key(key);
        let mut conn = self.conn.clone();

        conn.incr(&full_key, 1i64)
            .await
            .map_err(|e| CacheError::Operation(format!("INCR failed: {}", e)))
    }

    /// Increment with expiry (useful for rate limiting)
    #[tracing::instrument(skip(self), fields(key = %key, ttl_secs = %ttl_secs))]
    pub async fn incr_ex(&self, key: &str, ttl_secs: u64) -> Result<i64, CacheError> {
        let full_key = self.full_key(key);
        let mut conn = self.conn.clone();

        let (count,): (i64,) = redis::pipe()
            .atomic()
            .incr(&full_key, 1i64)
            .expire(&full_key, ttl_secs as i64)
            .ignore()
            .query_async(&mut conn)
            .await
            .map_err(|e| CacheError::Operation(format!("INCR_EX failed: {}", e)))?;

        Ok(count)
    }

    /// Get cache info/stats
    pub async fn info(&self) -> Result<CacheInfo, CacheError> {
        let mut conn = self.conn.clone();

        let info: String = redis::cmd("INFO")
            .arg("stats")
            .query_async(&mut conn)
            .await
            .map_err(|e| CacheError::Operation(format!("INFO failed: {}", e)))?;

        let mut hits = 0u64;
        let mut misses = 0u64;

        for line in info.lines() {
            if line.starts_with("keyspace_hits:") {
                hits = line.split(':').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            } else if line.starts_with("keyspace_misses:") {
                misses = line.split(':').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
        }

        let dbsize: i64 = redis::cmd("DBSIZE")
            .query_async(&mut conn)
            .await
            .unwrap_or(0);

        Ok(CacheInfo {
            hits,
            misses,
            keys: dbsize as u64,
        })
    }

    /// Health check
    pub async fn health_check(&self) -> Result<(), CacheError> {
        let mut conn = self.conn.clone();
        let _: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(|e| CacheError::Connection(format!("Health check failed: {}", e)))?;
        Ok(())
    }
}

/// Cache statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct CacheInfo {
    pub hits: u64,
    pub misses: u64,
    pub keys: u64,
}

impl CacheInfo {
    /// Calculate hit rate as percentage
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

/// Cache errors
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Operation error: {0}")]
    Operation(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Timeout")]
    Timeout,
}

impl From<RedisError> for CacheError {
    fn from(e: RedisError) -> Self {
        CacheError::Operation(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env_default() {
        let config = CacheConfig::from_env();
        assert!(config.url.contains("redis://"));
        assert_eq!(config.key_prefix, "penpot");
    }

    #[test]
    fn test_config_with_url() {
        let config = CacheConfig::with_url("redis://custom:6380");
        assert_eq!(config.url, "redis://custom:6380");
        assert_eq!(config.key_prefix, "penpot");
        assert_eq!(config.default_ttl_secs, 300);
    }

    #[test]
    fn test_hit_rate() {
        let info = CacheInfo {
            hits: 80,
            misses: 20,
            keys: 100,
        };
        assert!((info.hit_rate() - 80.0).abs() < 0.001);

        let empty = CacheInfo {
            hits: 0,
            misses: 0,
            keys: 0,
        };
        assert_eq!(empty.hit_rate(), 0.0);
    }
}
