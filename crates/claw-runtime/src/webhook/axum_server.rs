//! Axum-based implementation of the WebhookServer trait.

use super::{
    EndpointConfig, HmacConfig, HttpMethod, WebhookConfig, WebhookError, WebhookRequest,
    WebhookResponse, WebhookServer, WebhookStats,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use dashmap::DashMap;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

/// Internal endpoint state.
struct EndpointState {
    config: EndpointConfig,
    stats: RwLock<WebhookStats>,
    /// 60-second dedup cache keyed by X-Request-Id header value.
    dedup_cache: DashMap<String, Instant>,
    /// Sliding-window rate limiter: (count_in_current_window, window_start).
    rate_counter: std::sync::Mutex<(u32, Instant)>,
}

/// Axum-based webhook server implementation.
///
/// # Deprecation
///
/// This type is **deprecated** since v1.5.0.
///
/// For kernel-level trigger webhooks use [`crate::webhook_server::WebhookTriggerServer`], which:
/// - Restricts methods at the axum routing layer (`post()` only), eliminating endpoint
///   enumeration side-channels.
/// - Uses the canonical `/hooks/{trigger_id}` multi-plex path.
/// - Integrates directly with [`crate::event_bus::EventBus`] via `TriggerEvent`.
///
/// `AxumWebhookServer` remains in place while `claw-server` depends on its synchronous
/// agent-loop callback model. Migrate `handle_trigger_add_webhook` to subscribe to
/// `Event::TriggerFired` from the `EventBus` and then remove this type.
#[deprecated(
    since = "1.5.0",
    note = "Use WebhookTriggerServer (claw_runtime::webhook_server) instead. \
            See docs/gap-analysis.md G-5 for migration rationale."
)]
pub struct AxumWebhookServer {
    config: WebhookConfig,
    endpoints: Arc<DashMap<String, Arc<EndpointState>>>,
    server_handle: Mutex<Option<JoinHandle<()>>>,
    shutdown_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    local_addr: RwLock<Option<SocketAddr>>,
    running: AtomicUsize,
}

impl AxumWebhookServer {
    /// Create a new AxumWebhookServer with the given configuration.
    pub fn new(config: WebhookConfig) -> Self {
        Self {
            config,
            endpoints: Arc::new(DashMap::new()),
            server_handle: Mutex::new(None),
            shutdown_tx: Mutex::new(None),
            local_addr: RwLock::new(None),
            running: AtomicUsize::new(0),
        }
    }

    /// Build the Axum router.
    fn build_router(&self) -> Router {
        let endpoints = Arc::clone(&self.endpoints);

        Router::new()
            .route("/*path", any(Self::handle_request))
            .route("/", any(Self::handle_root))
            .with_state(endpoints)
    }

    /// Handle incoming webhook requests.
    async fn handle_request(
        State(endpoints): State<Arc<DashMap<String, Arc<EndpointState>>>>,
        Path(path): Path<String>,
        method: Method,
        headers: HeaderMap,
        Query(query): Query<HashMap<String, String>>,
        body: Bytes,
    ) -> Response {
        let full_path = format!("/{}", path);
        Self::process_webhook(endpoints, full_path, method, headers, query, body).await
    }

    /// Handle root path requests.
    async fn handle_root(
        State(endpoints): State<Arc<DashMap<String, Arc<EndpointState>>>>,
        method: Method,
        headers: HeaderMap,
        Query(query): Query<HashMap<String, String>>,
        body: Bytes,
    ) -> Response {
        Self::process_webhook(endpoints, "/".to_string(), method, headers, query, body).await
    }

    /// Process webhook request.
    async fn process_webhook(
        endpoints: Arc<DashMap<String, Arc<EndpointState>>>,
        path: String,
        method: Method,
        headers: HeaderMap,
        query: HashMap<String, String>,
        body: Bytes,
    ) -> Response {
        // Find matching endpoint
        let state = match endpoints.get(&path) {
            Some(s) => Arc::clone(&*s),
            None => {
                return (StatusCode::NOT_FOUND, "Endpoint not found").into_response();
            }
        };

        // Check HTTP method.
        // Security: return 404 (not 405) on method mismatch to prevent side-channel
        // enumeration of registered endpoints. A 405 at this point would reveal that
        // the path is registered, which is information an unauthenticated caller should
        // not have. WebhookTriggerServer avoids this entirely by restricting at the
        // routing layer (axum `post()` route); here we approximate that at the handler level.
        let http_method = match Self::convert_method(&method) {
            Some(m) => m,
            None => {
                return (StatusCode::NOT_FOUND, "Endpoint not found").into_response();
            }
        };
        if !state.config.methods.contains(&http_method) {
            return (StatusCode::NOT_FOUND, "Endpoint not found").into_response();
        }

        // Check body size
        if body.len() > state.config.max_body_size {
            return (StatusCode::PAYLOAD_TOO_LARGE, "Request body too large").into_response();
        }

        // Convert headers
        let header_map: HashMap<String, String> = headers
            .iter()
            .filter_map(|(k, v)| {
                v.to_str()
                    .ok()
                    .map(|v| (k.as_str().to_lowercase(), v.to_string()))
            })
            .collect();

        // Verify HMAC if configured
        if let HmacConfig::Sha256 { secret, header, prefix } = &state.config.hmac {
            // Reject immediately if the signature header is absent or empty.
            let sig_header_value = match header_map
                .get(&header.to_lowercase())
                .filter(|v| !v.is_empty())
            {
                Some(v) => v.clone(),
                None => {
                    tracing::warn!(
                        "Webhook: missing or empty HMAC signature header '{}'",
                        header
                    );
                    let mut stats = state.stats.write().await;
                    stats.hmac_failures += 1;
                    return (StatusCode::UNAUTHORIZED, "Missing HMAC signature").into_response();
                }
            };

            // Strip the expected prefix (e.g. "sha256="). If the prefix is
            // configured but absent, reject rather than falling back to the raw value.
            let sig = if let Some(p) = prefix {
                match sig_header_value.strip_prefix(p.as_str()) {
                    Some(stripped) => stripped,
                    None => {
                        tracing::warn!(
                            "Webhook: HMAC signature header '{}' is missing expected prefix '{}'",
                            header,
                            p
                        );
                        let mut stats = state.stats.write().await;
                        stats.hmac_failures += 1;
                        return (StatusCode::UNAUTHORIZED, "Invalid HMAC signature format").into_response();
                    }
                }
            } else {
                sig_header_value.as_str()
            };

            if let Err(e) = super::verification::verify_hmac_sha256(secret, &body, sig) {
                // Update HMAC failure stats
                let mut stats = state.stats.write().await;
                stats.hmac_failures += 1;
                return (StatusCode::UNAUTHORIZED, e.to_string()).into_response();
            }
        }

        // Rate limiting: sliding-window counter per endpoint.
        // The std::sync::Mutex guard is released before any .await call.
        let rate_limited = {
            let now = Instant::now();
            let mut counter = state.rate_counter.lock().unwrap_or_else(|e| e.into_inner());
            if now.duration_since(counter.1) >= std::time::Duration::from_secs(60) {
                // New window — reset and count this request.
                *counter = (1, now);
                false
            } else if counter.0 >= state.config.max_requests_per_minute {
                true
            } else {
                counter.0 += 1;
                false
            }
            // Mutex guard dropped here.
        };
        if rate_limited {
            let mut stats = state.stats.write().await;
            stats.requests_rate_limited += 1;
            return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded").into_response();
        }

        // Dedup: X-Request-Id based idempotency within a 60s window.
        // Only applies when the header is present; requests without it are always processed.
        let is_duplicate = if let Some(rid) = header_map.get("x-request-id").cloned() {
            let now = Instant::now();
            let ttl = std::time::Duration::from_secs(60);
            // Lazily evict expired entries on each request.
            state.dedup_cache.retain(|_, inserted_at| now.duration_since(*inserted_at) < ttl);
            if state.dedup_cache.contains_key(&rid) {
                true
            } else {
                state.dedup_cache.insert(rid, now);
                false
            }
        } else {
            false
        };
        if is_duplicate {
            let mut stats = state.stats.write().await;
            stats.requests_deduped += 1;
            return (
                StatusCode::OK,
                r#"{"status":"duplicate","skipped":true}"#,
            )
                .into_response();
        }

        // Build request
        let request = WebhookRequest {
            path: path.clone(),
            method: http_method,
            headers: header_map,
            body: body.to_vec(),
            remote_addr: None, // Could extract from extensions
            query,
        };

        // Execute handler
        let start = SystemTime::now();
        let handler = Arc::clone(&state.config.handler);

        let response = match handler(request).await {
            Ok(resp) => resp,
            Err(e) => WebhookResponse::error(500, e.to_string()),
        };

        // Update stats
        let elapsed = SystemTime::now()
            .duration_since(start)
            .unwrap_or_default()
            .as_millis() as u64;

        {
            let mut stats = state.stats.write().await;
            stats.requests_total += 1;
            stats.last_request = Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            );

            if response.status >= 200 && response.status < 300 {
                stats.requests_success += 1;
            } else {
                stats.requests_error += 1;
            }

            // Update average response time
            stats.avg_response_time_ms =
                (stats.avg_response_time_ms * (stats.requests_total - 1) + elapsed)
                    / stats.requests_total;
        }

        // Build response
        let mut axum_response = Response::builder().status(response.status);

        for (k, v) in response.headers {
            axum_response = axum_response.header(k, v);
        }

        axum_response
            .body(axum::body::Body::from(response.body))
            .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response())
            .into_response()
    }

    /// Convert Axum Method to our HttpMethod.
    ///
    /// Returns `None` for unrecognised methods (e.g. OPTIONS, HEAD, CONNECT)
    /// so that callers can return 405 instead of silently treating them as POST.
    fn convert_method(method: &Method) -> Option<HttpMethod> {
        match method.as_str() {
            "GET" => Some(HttpMethod::Get),
            "POST" => Some(HttpMethod::Post),
            "PUT" => Some(HttpMethod::Put),
            "PATCH" => Some(HttpMethod::Patch),
            _ => None,
        }
    }

    /// Get statistics for an endpoint.
    pub async fn stats(&self, path: &str) -> Option<WebhookStats> {
        match self.endpoints.get(path) {
            Some(e) => Some(e.stats.read().await.clone()),
            None => None,
        }
    }
}

#[async_trait::async_trait]
impl WebhookServer for AxumWebhookServer {
    async fn register(&self, config: EndpointConfig) -> Result<(), WebhookError> {
        if self.endpoints.contains_key(&config.path) {
            return Err(WebhookError::HandlerAlreadyExists(config.path));
        }

        let state = Arc::new(EndpointState {
            config,
            stats: RwLock::new(WebhookStats::default()),
            dedup_cache: DashMap::new(),
            rate_counter: std::sync::Mutex::new((0, Instant::now())),
        });

        self.endpoints.insert(state.config.path.clone(), state);
        Ok(())
    }

    async fn unregister(&self, path: &str) -> Result<(), WebhookError> {
        self.endpoints
            .remove(path)
            .ok_or_else(|| WebhookError::HandlerNotFound(path.to_string()))?;
        Ok(())
    }

    async fn is_registered(&self, path: &str) -> bool {
        self.endpoints.contains_key(path)
    }

    async fn list_endpoints(&self) -> Vec<String> {
        self.endpoints.iter().map(|e| e.key().clone()).collect()
    }

    async fn start(&self) -> Result<(), WebhookError> {
        // Check if already running
        if self.running.load(Ordering::SeqCst) != 0 {
            return Err(WebhookError::AlreadyRunning);
        }

        let app = self.build_router();
        let addr: SocketAddr = format!("{}:{}", self.config.bind_addr, self.config.port)
            .parse()
            .map_err(|e| WebhookError::InvalidConfig(format!("Invalid address: {}", e)))?;

        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| WebhookError::BindFailed(e.to_string()))?;

        let local_addr = listener.local_addr().ok();
        *self.local_addr.write().await = local_addr;

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        *self.shutdown_tx.lock().await = Some(shutdown_tx);

        let server = axum::serve(listener, app);

        let handle = tokio::spawn(async move {
            let server = server.with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            let _ = server.await;
        });

        *self.server_handle.lock().await = Some(handle);
        self.running.store(1, Ordering::SeqCst);

        Ok(())
    }

    async fn stop(&self) -> Result<(), WebhookError> {
        if self.running.load(Ordering::SeqCst) == 0 {
            return Err(WebhookError::NotRunning);
        }

        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
        }

        // Wait for server to stop
        if let Some(handle) = self.server_handle.lock().await.take() {
            let _ = handle.await;
        }

        self.running.store(0, Ordering::SeqCst);
        *self.local_addr.write().await = None;

        Ok(())
    }

    async fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst) != 0
    }

    async fn local_addr(&self) -> Result<String, WebhookError> {
        match *self.local_addr.read().await {
            Some(addr) => Ok(addr.to_string()),
            None => Err(WebhookError::NotRunning),
        }
    }
}

impl Default for AxumWebhookServer {
    fn default() -> Self {
        Self::new(WebhookConfig::new("127.0.0.1", 8080))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_axum_webhook_server_new() {
        let config = WebhookConfig::new("127.0.0.1", 0);
        let server = AxumWebhookServer::new(config);
        assert!(!server.is_running().await);
        assert!(server.list_endpoints().await.is_empty());
    }

    #[tokio::test]
    async fn test_register_endpoint() {
        let config = WebhookConfig::new("127.0.0.1", 0);
        let server = AxumWebhookServer::new(config);

        server
            .register(EndpointConfig::new("/test", |_req| async {
                Ok(WebhookResponse::ok())
            }))
            .await
            .unwrap();

        assert!(server.is_registered("/test").await);
        assert!(!server.is_registered("/other").await);
    }

    #[tokio::test]
    async fn test_register_duplicate_fails() {
        let config = WebhookConfig::new("127.0.0.1", 0);
        let server = AxumWebhookServer::new(config);

        server
            .register(EndpointConfig::new("/dup", |_req| async { Ok(WebhookResponse::ok()) }))
            .await
            .unwrap();

        let result = server
            .register(EndpointConfig::new("/dup", |_req| async { Ok(WebhookResponse::ok()) }))
            .await;

        assert!(matches!(result, Err(WebhookError::HandlerAlreadyExists(_))));
    }

    #[tokio::test]
    async fn test_unregister_endpoint() {
        let config = WebhookConfig::new("127.0.0.1", 0);
        let server = AxumWebhookServer::new(config);

        server
            .register(EndpointConfig::new("/unreg", |_req| async { Ok(WebhookResponse::ok()) }))
            .await
            .unwrap();

        assert!(server.is_registered("/unreg").await);

        server.unregister("/unreg").await.unwrap();

        assert!(!server.is_registered("/unreg").await);
    }

    #[tokio::test]
    async fn test_list_endpoints() {
        let config = WebhookConfig::new("127.0.0.1", 0);
        let server = AxumWebhookServer::new(config);

        for i in 0..3 {
            server
                .register(EndpointConfig::new(
                    format!("/ep{}", i),
                    |_req| async { Ok(WebhookResponse::ok()) },
                ))
                .await
                .unwrap();
        }

        let endpoints = server.list_endpoints().await;
        assert_eq!(endpoints.len(), 3);
    }

    #[tokio::test]
    async fn test_start_stop_server() {
        let config = WebhookConfig::new("127.0.0.1", 0);
        let server = AxumWebhookServer::new(config);

        // Start
        server.start().await.unwrap();
        assert!(server.is_running().await);

        // Get address
        let addr = server.local_addr().await;
        assert!(addr.is_ok());

        // Stop
        server.stop().await.unwrap();
        assert!(!server.is_running().await);
    }

    #[tokio::test]
    async fn test_start_already_running_fails() {
        let config = WebhookConfig::new("127.0.0.1", 0);
        let server = AxumWebhookServer::new(config);

        server.start().await.unwrap();

        let result = server.start().await;
        assert!(matches!(result, Err(WebhookError::AlreadyRunning)));

        server.stop().await.unwrap();
    }
}
