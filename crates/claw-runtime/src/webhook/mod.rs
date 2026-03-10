//! Webhook — External HTTP event input for claw-runtime.
//!
//! Provides HTTP webhook server capabilities with HMAC signature verification.
//! This is a Layer 1 (System Runtime) primitive, symmetric to EventBus
//! but for external events.
//!
//! # Example
//!
//! ```rust,ignore
//! use claw_runtime::webhook::{WebhookServer, WebhookConfig, WebhookRequest};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create webhook server
//! let config = WebhookConfig::new("0.0.0.0", 8080);
//! let server = WebhookServer::new(config);
//!
//! // Register a handler
//! server.register("/github", |req: WebhookRequest| async move {
//!     println!("Received GitHub webhook: {:?}", req.headers);
//!     Ok(())
//! }).await?;
//!
//! // Start the server
//! server.start().await?;
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod verification;

#[cfg(feature = "webhook")]
pub mod axum_server;

pub use error::WebhookError;
pub use verification::{verify_hmac_sha256, WebhookVerifier};

#[cfg(feature = "webhook")]
#[allow(deprecated)]
pub use axum_server::AxumWebhookServer;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Unique identifier for a webhook endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EndpointId(pub String);

impl EndpointId {
    /// Create a new EndpointId from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for EndpointId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// HTTP method for webhook endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HttpMethod {
    #[default]
    Post,
    Put,
    Patch,
    Get,
}

/// Webhook request received from external source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookRequest {
    /// The endpoint path (e.g., "/github/webhook").
    pub path: String,
    /// HTTP method.
    pub method: HttpMethod,
    /// HTTP headers.
    pub headers: HashMap<String, String>,
    /// Request body (raw bytes).
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    /// Remote address.
    pub remote_addr: Option<String>,
    /// Query parameters.
    pub query: HashMap<String, String>,
}

impl WebhookRequest {
    /// Get a header value by name (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&String> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    }

    /// Parse body as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    /// Get body as string (UTF-8).
    pub fn body_string(&self) -> Result<String, std::string::FromUtf8Error> {
        String::from_utf8(self.body.clone())
    }
}

/// Webhook response to send back to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body.
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
}

impl WebhookResponse {
    /// Create a successful (200) response.
    pub fn ok() -> Self {
        Self {
            status: 200,
            headers: HashMap::new(),
            body: vec![],
        }
    }

    /// Create a response with JSON body.
    pub fn json<T: serde::Serialize>(data: &T) -> Result<Self, serde_json::Error> {
        let body = serde_json::to_vec(data)?;
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        Ok(Self {
            status: 200,
            headers,
            body,
        })
    }

    /// Create an error response.
    pub fn error(status: u16, message: impl Into<String>) -> Self {
        let body = message.into().into_bytes();
        Self {
            status,
            headers: HashMap::new(),
            body,
        }
    }

    /// Set a header.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Set the status code.
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
}

impl Default for WebhookResponse {
    fn default() -> Self {
        Self::ok()
    }
}

/// Handler type for webhook endpoints.
pub type WebhookHandler = Arc<
    dyn Fn(WebhookRequest) -> Pin<Box<dyn Future<Output = Result<WebhookResponse, WebhookError>> + Send>>
        + Send
        + Sync,
>;

/// HMAC verification configuration.
#[derive(Debug, Clone)]
pub enum HmacConfig {
    /// No verification.
    None,
    /// Verify HMAC-SHA256 signature.
    Sha256 {
        /// Secret key for verification.
        secret: String,
        /// Header name containing the signature (e.g., "X-Hub-Signature-256").
        header: String,
        /// Signature prefix (e.g., "sha256=").
        prefix: Option<String>,
    },
}

impl Default for HmacConfig {
    fn default() -> Self {
        Self::None
    }
}

/// Configuration for a webhook endpoint.
#[derive(Clone)]
pub struct EndpointConfig {
    /// Endpoint path (e.g., "/github").
    pub path: String,
    /// Allowed HTTP methods.
    pub methods: Vec<HttpMethod>,
    /// Handler function.
    pub handler: WebhookHandler,
    /// HMAC verification configuration.
    pub hmac: HmacConfig,
    /// Maximum body size in bytes.
    pub max_body_size: usize,
    /// Whether to parse JSON automatically.
    pub parse_json: bool,
    /// Maximum requests per minute per endpoint (rate limit). Defaults to 100.
    pub max_requests_per_minute: u32,
}

impl std::fmt::Debug for EndpointConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointConfig")
            .field("path", &self.path)
            .field("methods", &self.methods)
            .field("hmac", &self.hmac)
            .field("max_body_size", &self.max_body_size)
            .field("parse_json", &self.parse_json)
            .finish_non_exhaustive()
    }
}

impl EndpointConfig {
    /// Create a new endpoint configuration.
    pub fn new<F, Fut>(path: impl Into<String>, handler: F) -> Self
    where
        F: Fn(WebhookRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<WebhookResponse, WebhookError>> + Send + 'static,
    {
        Self {
            path: path.into(),
            methods: vec![HttpMethod::Post],
            handler: Arc::new(move |req| Box::pin(handler(req))),
            hmac: HmacConfig::None,
            max_body_size: 10 * 1024 * 1024, // 10MB default
            parse_json: false,
            max_requests_per_minute: 100,
        }
    }

    /// Allow additional HTTP methods.
    pub fn with_methods(mut self, methods: Vec<HttpMethod>) -> Self {
        self.methods = methods;
        self
    }

    /// Enable HMAC-SHA256 verification.
    pub fn with_hmac_sha256(
        mut self,
        secret: impl Into<String>,
        header: impl Into<String>,
    ) -> Self {
        self.hmac = HmacConfig::Sha256 {
            secret: secret.into(),
            header: header.into(),
            prefix: Some("sha256=".to_string()),
        };
        self
    }

    /// Set maximum body size.
    pub fn with_max_body_size(mut self, size: usize) -> Self {
        self.max_body_size = size;
        self
    }

    /// Enable automatic JSON parsing.
    pub fn with_json_parsing(mut self) -> Self {
        self.parse_json = true;
        self
    }

    /// Set the maximum requests per minute for rate limiting.
    pub fn with_rate_limit(mut self, max: u32) -> Self {
        self.max_requests_per_minute = max;
        self
    }
}

/// Webhook server configuration.
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    /// Bind address (e.g., "0.0.0.0" or "127.0.0.1").
    pub bind_addr: String,
    /// Port number.
    pub port: u16,
    /// Global request timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum concurrent connections.
    pub max_connections: usize,
}

impl WebhookConfig {
    /// Create a new configuration with default values.
    pub fn new(bind_addr: impl Into<String>, port: u16) -> Self {
        Self {
            bind_addr: bind_addr.into(),
            port,
            timeout_secs: 30,
            max_connections: 100,
        }
    }

    /// Set the request timeout.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Set maximum concurrent connections.
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = max;
        self
    }
}

/// Core trait for webhook servers.
///
/// Implementations must be thread-safe and support concurrent request handling.
#[async_trait::async_trait]
pub trait WebhookServer: Send + Sync {
    /// Register a new endpoint.
    ///
    /// Returns an error if an endpoint with the same path already exists.
    async fn register(&self, config: EndpointConfig) -> Result<(), WebhookError>;

    /// Unregister an endpoint.
    async fn unregister(&self, path: &str) -> Result<(), WebhookError>;

    /// Check if an endpoint is registered.
    async fn is_registered(&self, path: &str) -> bool;

    /// Get list of registered endpoint paths.
    async fn list_endpoints(&self) -> Vec<String>;

    /// Start the webhook server.
    async fn start(&self) -> Result<(), WebhookError>;

    /// Stop the webhook server gracefully.
    async fn stop(&self) -> Result<(), WebhookError>;

    /// Check if the server is running.
    async fn is_running(&self) -> bool;

    /// Get the server bind address (available after start).
    async fn local_addr(&self) -> Result<String, WebhookError>;
}

/// Statistics for a webhook endpoint.
#[derive(Debug, Clone, Default)]
pub struct WebhookStats {
    /// Total requests received.
    pub requests_total: u64,
    /// Successful requests (2xx responses).
    pub requests_success: u64,
    /// Failed requests (4xx/5xx responses).
    pub requests_error: u64,
    /// HMAC verification failures.
    pub hmac_failures: u64,
    /// Last request timestamp (Unix milliseconds).
    pub last_request: Option<u64>,
    /// Average response time in milliseconds.
    pub avg_response_time_ms: u64,
    /// Requests skipped due to duplicate X-Request-Id within 60s dedup window.
    pub requests_deduped: u64,
    /// Requests rejected due to per-endpoint rate limit (100 req/min default).
    pub requests_rate_limited: u64,
}

/// Extension trait for webhook server utilities.
#[async_trait::async_trait]
pub trait WebhookServerExt: WebhookServer {
    /// Register a simple handler that returns 200 OK.
    async fn register_health_check(&self, path: impl Into<String> + Send) -> Result<(), WebhookError> {
        let config = EndpointConfig::new(path, |_req| async { Ok(WebhookResponse::ok()) });
        self.register(config).await
    }

    /// Register a handler with JSON response.
    async fn register_json<F, Fut, T>(
        &self,
        path: impl Into<String> + Send,
        handler: F,
    ) -> Result<(), WebhookError>
    where
        F: Fn(WebhookRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, WebhookError>> + Send + 'static,
        T: serde::Serialize + Send + 'static,
    {
        let handler = Arc::new(handler);
        let config = EndpointConfig::new(path, move |req| {
            let handler = Arc::clone(&handler);
            async move {
                let data = handler(req).await?;
                WebhookResponse::json(&data).map_err(|e| {
                    WebhookError::Internal(format!("JSON serialization failed: {}", e))
                })
            }
        })
        .with_json_parsing();

        self.register(config).await
    }
}

#[async_trait::async_trait]
impl<T: WebhookServer> WebhookServerExt for T {}

// serde_bytes helper module for Vec<u8>
mod serde_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(s.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoint_id() {
        let id = EndpointId::new("github-webhook");
        assert_eq!(id.0, "github-webhook");
        assert_eq!(id.to_string(), "github-webhook");
    }

    #[test]
    fn test_webhook_request_header() {
        let mut headers = HashMap::new();
        headers.insert("X-GitHub-Event".to_string(), "push".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let req = WebhookRequest {
            path: "/github".to_string(),
            method: HttpMethod::Post,
            headers,
            body: br#"{"ref": "main"}"#.to_vec(),
            remote_addr: Some("192.168.1.1:1234".to_string()),
            query: HashMap::new(),
        };

        assert_eq!(req.header("x-github-event"), Some(&"push".to_string()));
        assert_eq!(req.header("content-type"), Some(&"application/json".to_string()));
        assert!(req.header("x-missing").is_none());
    }

    #[test]
    fn test_webhook_request_json() {
        let req = WebhookRequest {
            path: "/test".to_string(),
            method: HttpMethod::Post,
            headers: HashMap::new(),
            body: br#"{"name": "test", "value": 42}"#.to_vec(),
            remote_addr: None,
            query: HashMap::new(),
        };

        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct TestData {
            name: String,
            value: i32,
        }

        let data: TestData = req.json().unwrap();
        assert_eq!(data.name, "test");
        assert_eq!(data.value, 42);
    }

    #[test]
    fn test_webhook_response() {
        let resp = WebhookResponse::ok();
        assert_eq!(resp.status, 200);
        assert!(resp.body.is_empty());

        let resp = WebhookResponse::error(404, "Not Found");
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body, b"Not Found");

        let resp = WebhookResponse::json(&serde_json::json!({"status": "ok"})).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(
            resp.headers.get("content-type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_endpoint_config_builder() {
        let config = EndpointConfig::new("/github", |_req| async { Ok(WebhookResponse::ok()) })
            .with_methods(vec![HttpMethod::Post, HttpMethod::Put])
            .with_hmac_sha256("secret123", "X-Hub-Signature-256")
            .with_max_body_size(1024)
            .with_json_parsing();

        assert_eq!(config.path, "/github");
        assert_eq!(config.methods.len(), 2);
        assert_eq!(config.max_body_size, 1024);
        assert!(config.parse_json);

        match config.hmac {
            HmacConfig::Sha256 { secret, header, .. } => {
                assert_eq!(secret, "secret123");
                assert_eq!(header, "X-Hub-Signature-256");
            }
            _ => panic!("Expected Sha256 HMAC config"),
        }
    }

    #[test]
    fn test_webhook_config() {
        let config = WebhookConfig::new("127.0.0.1", 8080)
            .with_timeout(60)
            .with_max_connections(200);

        assert_eq!(config.bind_addr, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert_eq!(config.timeout_secs, 60);
        assert_eq!(config.max_connections, 200);
    }

    #[test]
    fn test_webhook_stats_default() {
        let stats = WebhookStats::default();
        assert_eq!(stats.requests_total, 0);
        assert_eq!(stats.requests_success, 0);
        assert_eq!(stats.requests_error, 0);
        assert_eq!(stats.hmac_failures, 0);
        assert!(stats.last_request.is_none());
        assert_eq!(stats.avg_response_time_ms, 0);
    }
}
