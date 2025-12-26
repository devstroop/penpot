//! Database connection pooling for PostgreSQL
//!
//! Provides connection pooling with support for read replicas.
//! Uses deadpool-postgres for async connection management.
//!
//! # Example
//!
//! ```ignore
//! use common::database::{DatabaseConfig, DatabasePool};
//!
//! let config = DatabaseConfig::from_env();
//! let pool = DatabasePool::new(config).await?;
//!
//! // Get a connection for writes (primary)
//! let conn = pool.get().await?;
//!
//! // Get a connection for reads (replica if configured)
//! let conn = pool.get_read().await?;
//! ```

use deadpool_postgres::{Config, Pool, PoolError, Runtime};
use std::sync::Arc;
use tokio_postgres::NoTls;
use tracing::{info, warn};

/// Database configuration
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Primary database URL (for writes)
    pub primary_url: String,
    /// Read replica URLs (optional, for read scaling)
    pub replica_urls: Vec<String>,
    /// Maximum connections per pool
    pub max_connections: usize,
    /// Connection timeout in seconds
    pub connect_timeout_secs: u64,
}

impl DatabaseConfig {
    /// Create config from environment variables
    ///
    /// Environment variables:
    /// - `DATABASE_URL` or `PENPOT_DATABASE_URI` - Primary database URL
    /// - `DATABASE_REPLICA_URLS` - Comma-separated replica URLs (optional)
    /// - `DATABASE_MAX_CONNECTIONS` - Max pool size (default: 20)
    /// - `DATABASE_CONNECT_TIMEOUT` - Connection timeout in seconds (default: 30)
    pub fn from_env() -> Self {
        let primary_url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("PENPOT_DATABASE_URI"))
            .unwrap_or_else(|_| "postgresql://penpot:penpot@localhost:5432/penpot".to_string());

        let replica_urls = std::env::var("DATABASE_REPLICA_URLS")
            .ok()
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();

        let max_connections = std::env::var("DATABASE_MAX_CONNECTIONS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20);

        let connect_timeout_secs = std::env::var("DATABASE_CONNECT_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);

        Self {
            primary_url,
            replica_urls,
            max_connections,
            connect_timeout_secs,
        }
    }

    /// Create config with a specific URL (for testing)
    pub fn with_url(url: impl Into<String>) -> Self {
        Self {
            primary_url: url.into(),
            replica_urls: vec![],
            max_connections: 20,
            connect_timeout_secs: 30,
        }
    }
}

/// Database connection pool with read replica support
pub struct DatabasePool {
    /// Primary pool (for writes and reads)
    primary: Pool,
    /// Replica pools (for read scaling)
    replicas: Vec<Pool>,
    /// Round-robin counter for replica selection
    replica_counter: std::sync::atomic::AtomicUsize,
}

impl DatabasePool {
    /// Create a new database pool from configuration
    pub async fn new(config: DatabaseConfig) -> Result<Arc<Self>, DatabaseError> {
        info!(
            "Initializing database pool (max_connections: {}, replicas: {})",
            config.max_connections,
            config.replica_urls.len()
        );

        // Create primary pool
        let primary = Self::create_pool(&config.primary_url, config.max_connections)?;

        // Test primary connection
        let test_conn = primary.get().await.map_err(|e| {
            DatabaseError::Connection(format!("Failed to connect to primary: {}", e))
        })?;
        drop(test_conn);
        info!("Connected to primary database");

        // Create replica pools
        let mut replicas = Vec::new();
        for (i, url) in config.replica_urls.iter().enumerate() {
            match Self::create_pool(url, config.max_connections) {
                Ok(pool) => {
                    // Test replica connection
                    match pool.get().await {
                        Ok(_) => {
                            info!("Connected to replica {}", i + 1);
                            replicas.push(pool);
                        }
                        Err(e) => {
                            warn!("Failed to connect to replica {}: {}", i + 1, e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to create replica {} pool: {}", i + 1, e);
                }
            }
        }

        Ok(Arc::new(Self {
            primary,
            replicas,
            replica_counter: std::sync::atomic::AtomicUsize::new(0),
        }))
    }

    fn create_pool(url: &str, max_size: usize) -> Result<Pool, DatabaseError> {
        let mut cfg = Config::new();
        
        // Parse URL to extract components
        let parsed = url::Url::parse(url)
            .map_err(|e| DatabaseError::Config(format!("Invalid database URL: {}", e)))?;
        
        cfg.host = parsed.host_str().map(|s| s.to_string());
        cfg.port = parsed.port();
        cfg.user = if parsed.username().is_empty() {
            None
        } else {
            Some(parsed.username().to_string())
        };
        cfg.password = parsed.password().map(|s| s.to_string());
        cfg.dbname = Some(parsed.path().trim_start_matches('/').to_string());
        
        cfg.pool = Some(deadpool_postgres::PoolConfig {
            max_size,
            ..Default::default()
        });

        cfg.create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| DatabaseError::Config(format!("Failed to create pool: {}", e)))
    }

    /// Get a connection from the primary pool (for writes)
    pub async fn get(&self) -> Result<deadpool_postgres::Object, DatabaseError> {
        self.primary
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e))
    }

    /// Get a connection for reads (uses replica if available)
    pub async fn get_read(&self) -> Result<deadpool_postgres::Object, DatabaseError> {
        if self.replicas.is_empty() {
            return self.get().await;
        }

        // Round-robin replica selection
        let idx = self
            .replica_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.replicas.len();

        self.replicas[idx]
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e))
    }

    /// Get the primary pool directly
    pub fn primary(&self) -> &Pool {
        &self.primary
    }

    /// Check if replicas are available
    pub fn has_replicas(&self) -> bool {
        !self.replicas.is_empty()
    }

    /// Get pool statistics
    pub fn stats(&self) -> PoolStats {
        let primary_status = self.primary.status();
        PoolStats {
            primary_size: primary_status.size,
            primary_available: primary_status.available,
            replica_count: self.replicas.len(),
            replica_stats: self
                .replicas
                .iter()
                .map(|p| {
                    let s = p.status();
                    (s.size, s.available)
                })
                .collect(),
        }
    }
}

/// Pool statistics
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub primary_size: usize,
    pub primary_available: usize,
    pub replica_count: usize,
    pub replica_stats: Vec<(usize, usize)>,
}

/// Database errors
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Pool error: {0}")]
    Pool(#[from] PoolError),

    #[error("Query error: {0}")]
    Query(String),
}

/// Helper trait for executing queries with tracing
#[allow(async_fn_in_trait)]
pub trait QueryExt {
    /// Execute a query and return rows
    async fn query_traced(
        &self,
        query: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Vec<tokio_postgres::Row>, DatabaseError>;

    /// Execute a query and return single row
    async fn query_one_traced(
        &self,
        query: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<tokio_postgres::Row, DatabaseError>;

    /// Execute a statement (INSERT, UPDATE, DELETE)
    async fn execute_traced(
        &self,
        query: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<u64, DatabaseError>;
}

impl QueryExt for deadpool_postgres::Object {
    #[tracing::instrument(skip(self, params), fields(query = %query))]
    async fn query_traced(
        &self,
        query: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Vec<tokio_postgres::Row>, DatabaseError> {
        self.query(query, params)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))
    }

    #[tracing::instrument(skip(self, params), fields(query = %query))]
    async fn query_one_traced(
        &self,
        query: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<tokio_postgres::Row, DatabaseError> {
        self.query_one(query, params)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))
    }

    #[tracing::instrument(skip(self, params), fields(query = %query))]
    async fn execute_traced(
        &self,
        query: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<u64, DatabaseError> {
        self.execute(query, params)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env() {
        std::env::set_var("DATABASE_URL", "postgresql://test:test@localhost:5432/testdb");
        let config = DatabaseConfig::from_env();
        assert_eq!(config.primary_url, "postgresql://test:test@localhost:5432/testdb");
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn test_config_with_url() {
        let config = DatabaseConfig::with_url("postgresql://user:pass@host:5432/db");
        assert_eq!(config.primary_url, "postgresql://user:pass@host:5432/db");
        assert!(config.replica_urls.is_empty());
    }
}
