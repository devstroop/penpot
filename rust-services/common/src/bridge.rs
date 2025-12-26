//! Clojure Integration Bridge
//!
//! Provides integration with the existing Penpot Clojure backend.
//! Handles communication via HTTP/Transit and shared data formats.
//!
//! # Architecture
//!
//! ```text
//! Penpot Clojure Backend  <-->  Bridge Layer  <-->  Rust Services
//!        (6060)                    (HTTP)           (8080-8083)
//! ```
//!
//! # Example
//!
//! ```ignore
//! use common::bridge::{ClojureBridge, BridgeConfig};
//!
//! let bridge = ClojureBridge::new(BridgeConfig::from_env()).await?;
//!
//! // Get file from Clojure backend
//! let file = bridge.get_file(file_id).await?;
//!
//! // Send validation result back
//! bridge.send_validation_result(file_id, result).await?;
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Bridge configuration
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Clojure backend base URL
    pub backend_url: String,
    /// Request timeout
    pub timeout: Duration,
    /// API token for authentication (optional)
    pub api_token: Option<String>,
    /// Enable request retries
    pub retry_enabled: bool,
    /// Maximum retry attempts
    pub max_retries: u32,
}

impl BridgeConfig {
    /// Create config from environment variables
    pub fn from_env() -> Self {
        let backend_url = std::env::var("PENPOT_BACKEND_URL")
            .or_else(|_| std::env::var("BACKEND_URL"))
            .unwrap_or_else(|_| "http://localhost:6060".to_string());

        let timeout_secs: u64 = std::env::var("BRIDGE_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);

        let api_token = std::env::var("PENPOT_API_TOKEN").ok();

        let retry_enabled = std::env::var("BRIDGE_RETRY_ENABLED")
            .map(|s| s == "true" || s == "1")
            .unwrap_or(true);

        let max_retries: u32 = std::env::var("BRIDGE_MAX_RETRIES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);

        Self {
            backend_url,
            timeout: Duration::from_secs(timeout_secs),
            api_token,
            retry_enabled,
            max_retries,
        }
    }

    /// Create config with a specific URL
    pub fn with_url(url: impl Into<String>) -> Self {
        Self {
            backend_url: url.into(),
            timeout: Duration::from_secs(30),
            api_token: None,
            retry_enabled: true,
            max_retries: 3,
        }
    }
}

/// Bridge errors
#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Request failed: {0}")]
    Request(String),

    #[error("Response error: {status} - {message}")]
    Response { status: u16, message: String },

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Authentication required")]
    AuthRequired,

    #[error("Timeout")]
    Timeout,
}

/// Penpot file representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PenpotFile {
    pub id: Uuid,
    pub name: String,
    pub project_id: Uuid,
    #[serde(default)]
    pub is_shared: bool,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    pub created_at: Option<String>,
    pub modified_at: Option<String>,
}

/// Penpot project representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PenpotProject {
    pub id: Uuid,
    pub name: String,
    pub team_id: Uuid,
    #[serde(default)]
    pub is_default: bool,
    pub created_at: Option<String>,
    pub modified_at: Option<String>,
}

/// Penpot team representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PenpotTeam {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
}

/// Penpot user session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PenpotSession {
    pub id: Uuid,
    pub profile_id: Uuid,
    #[serde(default)]
    pub is_authenticated: bool,
}

/// RPC command request
#[derive(Debug, Serialize)]
struct RpcRequest<T: Serialize> {
    #[serde(flatten)]
    params: T,
}

/// RPC command response wrapper
#[derive(Debug, Deserialize)]
struct RpcResponse<T> {
    #[serde(flatten)]
    result: T,
}

/// Bridge to Clojure backend
pub struct ClojureBridge {
    config: BridgeConfig,
    client: reqwest::Client,
}

impl ClojureBridge {
    /// Create a new bridge to Clojure backend
    pub fn new(config: BridgeConfig) -> Result<Arc<Self>, BridgeError> {
        info!("Initializing Clojure bridge to {}", config.backend_url);

        let mut builder = reqwest::Client::builder()
            .timeout(config.timeout)
            .pool_max_idle_per_host(20);

        // Add default headers
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/transit+json".parse().unwrap(),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            "application/transit+json, application/json".parse().unwrap(),
        );

        if let Some(ref token) = config.api_token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Token {}", token).parse().unwrap(),
            );
        }

        builder = builder.default_headers(headers);

        let client = builder
            .build()
            .map_err(|e| BridgeError::Connection(format!("Failed to create client: {}", e)))?;

        Ok(Arc::new(Self { config, client }))
    }

    /// Execute an RPC command against the Clojure backend
    #[tracing::instrument(skip(self, params), fields(command = %command))]
    pub async fn rpc_command<T, R>(&self, command: &str, params: T) -> Result<R, BridgeError>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        let url = format!("{}/api/rpc/command/{}", self.config.backend_url, command);
        debug!("RPC command: {} -> {}", command, url);

        let response = self
            .client
            .post(&url)
            .json(&params)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BridgeError::Timeout
                } else if e.is_connect() {
                    BridgeError::Connection(e.to_string())
                } else {
                    BridgeError::Request(e.to_string())
                }
            })?;

        let status = response.status();

        if status.is_success() {
            response
                .json::<R>()
                .await
                .map_err(|e| BridgeError::Serialization(e.to_string()))
        } else if status.as_u16() == 404 {
            Err(BridgeError::NotFound(command.to_string()))
        } else if status.as_u16() == 401 {
            Err(BridgeError::AuthRequired)
        } else {
            let message = response.text().await.unwrap_or_default();
            Err(BridgeError::Response {
                status: status.as_u16(),
                message,
            })
        }
    }

    /// Get a file by ID
    pub async fn get_file(&self, file_id: Uuid) -> Result<PenpotFile, BridgeError> {
        #[derive(Serialize)]
        struct Params {
            id: Uuid,
        }

        self.rpc_command("get-file", Params { id: file_id }).await
    }

    /// Get file data (shapes, pages, etc.)
    pub async fn get_file_data(&self, file_id: Uuid) -> Result<serde_json::Value, BridgeError> {
        #[derive(Serialize)]
        struct Params {
            id: Uuid,
        }

        self.rpc_command("get-file-data", Params { id: file_id })
            .await
    }

    /// Get project by ID
    pub async fn get_project(&self, project_id: Uuid) -> Result<PenpotProject, BridgeError> {
        #[derive(Serialize)]
        struct Params {
            id: Uuid,
        }

        self.rpc_command("get-project", Params { id: project_id })
            .await
    }

    /// Get all projects for a team
    pub async fn get_projects(&self, team_id: Uuid) -> Result<Vec<PenpotProject>, BridgeError> {
        #[derive(Serialize)]
        struct Params {
            team_id: Uuid,
        }

        self.rpc_command("get-projects", Params { team_id }).await
    }

    /// Get team by ID
    pub async fn get_team(&self, team_id: Uuid) -> Result<PenpotTeam, BridgeError> {
        #[derive(Serialize)]
        struct Params {
            id: Uuid,
        }

        self.rpc_command("get-team", Params { id: team_id }).await
    }

    /// Verify a session token
    pub async fn verify_session(&self, session_id: Uuid) -> Result<PenpotSession, BridgeError> {
        #[derive(Serialize)]
        struct Params {
            id: Uuid,
        }

        self.rpc_command("get-profile", Params { id: session_id })
            .await
    }

    /// Send validation results back to Clojure backend
    pub async fn send_validation_result(
        &self,
        file_id: Uuid,
        valid: bool,
        errors: Option<Vec<String>>,
    ) -> Result<(), BridgeError> {
        #[derive(Serialize)]
        struct Params {
            file_id: Uuid,
            valid: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            errors: Option<Vec<String>>,
        }

        let _: serde_json::Value = self
            .rpc_command(
                "submit-validation-result",
                Params {
                    file_id,
                    valid,
                    errors,
                },
            )
            .await?;

        Ok(())
    }

    /// Notify about render completion
    pub async fn notify_render_complete(
        &self,
        file_id: Uuid,
        page_id: Option<Uuid>,
        output_path: &str,
    ) -> Result<(), BridgeError> {
        #[derive(Serialize)]
        struct Params<'a> {
            file_id: Uuid,
            #[serde(skip_serializing_if = "Option::is_none")]
            page_id: Option<Uuid>,
            output_path: &'a str,
        }

        let _: serde_json::Value = self
            .rpc_command(
                "notify-render-complete",
                Params {
                    file_id,
                    page_id,
                    output_path,
                },
            )
            .await?;

        Ok(())
    }

    /// Health check for Clojure backend
    pub async fn health_check(&self) -> Result<bool, BridgeError> {
        let url = format!("{}/readyz", self.config.backend_url);

        let response = self.client.get(&url).send().await.map_err(|e| {
            if e.is_timeout() {
                BridgeError::Timeout
            } else {
                BridgeError::Connection(e.to_string())
            }
        })?;

        Ok(response.status().is_success())
    }

    /// Get backend info/version
    pub async fn get_backend_info(&self) -> Result<BackendInfo, BridgeError> {
        let url = format!("{}/api/info", self.config.backend_url);

        let response = self.client.get(&url).send().await.map_err(|e| {
            if e.is_timeout() {
                BridgeError::Timeout
            } else {
                BridgeError::Connection(e.to_string())
            }
        })?;

        if response.status().is_success() {
            response
                .json::<BackendInfo>()
                .await
                .map_err(|e| BridgeError::Serialization(e.to_string()))
        } else {
            // Return default if endpoint not available
            Ok(BackendInfo::default())
        }
    }
}

/// Backend information
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackendInfo {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub flags: Vec<String>,
}

/// Service discovery for dynamic service registration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRegistration {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub health_endpoint: String,
    pub capabilities: Vec<String>,
}

impl ServiceRegistration {
    /// Create registration for shape validator
    pub fn shape_validator(host: &str, port: u16) -> Self {
        Self {
            name: "shape-validator".to_string(),
            host: host.to_string(),
            port,
            health_endpoint: "/health".to_string(),
            capabilities: vec!["validate".to_string(), "batch-validate".to_string()],
        }
    }

    /// Create registration for render service
    pub fn render_service(host: &str, port: u16) -> Self {
        Self {
            name: "render-service".to_string(),
            host: host.to_string(),
            port,
            health_endpoint: "/health".to_string(),
            capabilities: vec![
                "render-png".to_string(),
                "render-svg".to_string(),
                "thumbnail".to_string(),
            ],
        }
    }

    /// Create registration for realtime sync
    pub fn realtime_sync(host: &str, port: u16) -> Self {
        Self {
            name: "realtime-sync".to_string(),
            host: host.to_string(),
            port,
            health_endpoint: "/health".to_string(),
            capabilities: vec![
                "websocket".to_string(),
                "presence".to_string(),
                "cursor-sync".to_string(),
            ],
        }
    }
}

/// Feature flags from Clojure backend
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeatureFlags {
    #[serde(default)]
    pub rust_validation_enabled: bool,
    #[serde(default)]
    pub rust_rendering_enabled: bool,
    #[serde(default)]
    pub rust_realtime_enabled: bool,
    #[serde(default)]
    pub rust_validation_percentage: u8,
    #[serde(default)]
    pub rust_rendering_percentage: u8,
}

impl FeatureFlags {
    /// Check if Rust validation should be used for this request
    pub fn should_use_rust_validation(&self, request_id: &Uuid) -> bool {
        if !self.rust_validation_enabled {
            return false;
        }
        if self.rust_validation_percentage >= 100 {
            return true;
        }
        // Use request ID to deterministically route percentage of traffic
        let hash = request_id.as_bytes()[0] as u8;
        (hash % 100) < self.rust_validation_percentage
    }

    /// Check if Rust rendering should be used for this request
    pub fn should_use_rust_rendering(&self, request_id: &Uuid) -> bool {
        if !self.rust_rendering_enabled {
            return false;
        }
        if self.rust_rendering_percentage >= 100 {
            return true;
        }
        let hash = request_id.as_bytes()[0] as u8;
        (hash % 100) < self.rust_rendering_percentage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env_default() {
        let config = BridgeConfig::from_env();
        assert!(config.backend_url.contains("localhost"));
        assert!(config.retry_enabled);
    }

    #[test]
    fn test_config_with_url() {
        let config = BridgeConfig::with_url("http://custom:8080");
        assert_eq!(config.backend_url, "http://custom:8080");
    }

    #[test]
    fn test_feature_flags_percentage() {
        let flags = FeatureFlags {
            rust_validation_enabled: true,
            rust_validation_percentage: 50,
            ..Default::default()
        };

        // Test deterministic routing
        let uuid1 = Uuid::parse_str("00000000-0000-0000-0000-000000000000").unwrap();
        let uuid2 = Uuid::parse_str("ff000000-0000-0000-0000-000000000000").unwrap();

        // First byte 0x00 = 0, should be included (0 < 50)
        assert!(flags.should_use_rust_validation(&uuid1));
        // First byte 0xff = 255, 255 % 100 = 55, should not be included (55 >= 50)
        assert!(!flags.should_use_rust_validation(&uuid2));
    }

    #[test]
    fn test_service_registration() {
        let reg = ServiceRegistration::shape_validator("localhost", 8081);
        assert_eq!(reg.name, "shape-validator");
        assert!(reg.capabilities.contains(&"validate".to_string()));
    }
}
