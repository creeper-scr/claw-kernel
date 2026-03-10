//! WebhookTriggerServer — 内核级 Webhook 触发入口（GAP-F6-03）。
//!
//! 与应用层的 `WebhookChannel`（claw-channel）不同，`WebhookTriggerServer`
//! 是内核触发基础设施，提供统一的 `/hooks/{trigger_id}` 多路复用路由，
//! 将外部 HTTP 回调直接转换为 [`TriggerEvent::Webhook`] 并发布到 [`EventBus`]。
//!
//! # 架构定位
//!
//! ```text
//! WebhookChannel（应用层）       WebhookTriggerServer（内核层）
//! ─────────────────────         ─────────────────────────────
//! 层次: 应用层渠道（F1）          层次: 内核触发基础设施（F6）
//! 用途: 双向 HTTP 通信渠道        用途: 只负责接收触发信号
//! 路由: 单端口单实例              路由: /hooks/{trigger_id} 多路复用
//! 输出: ChannelMessage            输出: TriggerEvent::Webhook
//! ```
//!
//! # 用法
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use claw_runtime::{EventBus, webhook_server::{WebhookTriggerServer, WebhookTriggerConfig}};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let bus = Arc::new(EventBus::new());
//! let server = WebhookTriggerServer::new("127.0.0.1:0", Arc::clone(&bus));
//!
//! // 注册触发器（可选 HMAC secret）
//! server.register_trigger("github-push", Some("my-github-secret".to_string()), None);
//! server.register_trigger("stripe-events", None, None);
//!
//! // 启动服务器
//! let addr = server.start().await?;
//! println!("WebhookTriggerServer listening on {}", addr);
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "webhook")]
mod inner {
    use crate::{
        agent_types::AgentId,
        event_bus::EventBus,
        events::Event,
        trigger_event::TriggerEvent,
        webhook::verification::compute_hmac_sha256,
    };
    use axum::{
        body::Bytes,
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::post,
        Router,
    };
    use dashmap::DashMap;
    use std::{
        net::SocketAddr,
        sync::Arc,
        time::{Duration, Instant},
    };
    use tokio::sync::Mutex;
    use tokio::task::JoinHandle;

    // ─── Error ────────────────────────────────────────────────────────────────

    /// `WebhookTriggerServer` 相关错误。
    #[derive(Debug, thiserror::Error)]
    pub enum WebhookServerError {
        #[error("bind failed: {0}")]
        BindFailed(String),
        #[error("server already running")]
        AlreadyRunning,
        #[error("server not running")]
        NotRunning,
        #[error("invalid address: {0}")]
        InvalidAddress(String),
    }

    // ─── Trigger registration entry ───────────────────────────────────────────

    /// 单个触发器的运行时状态。
    struct TriggerEntry {
        /// 可选 HMAC-SHA256 secret（hex 格式验证签名）。
        secret: Option<String>,
        /// 可选目标 Agent ID；None = 广播到所有在线 Agent。
        target_agent: Option<AgentId>,
        /// 限流：(本窗口计数, 窗口开始时间)。
        rate_counter: std::sync::Mutex<(u32, Instant)>,
        /// 每分钟最大请求数（默认 100）。
        max_per_minute: u32,
        /// 60 秒幂等去重缓存，键为 `X-Request-Id` 值。
        dedup_cache: DashMap<String, Instant>,
    }

    impl TriggerEntry {
        fn new(secret: Option<String>, target_agent: Option<AgentId>) -> Self {
            Self {
                secret,
                target_agent,
                rate_counter: std::sync::Mutex::new((0, Instant::now())),
                max_per_minute: 100,
                dedup_cache: DashMap::new(),
            }
        }
    }

    // ─── Shared server state ──────────────────────────────────────────────────

    struct ServerState {
        triggers: DashMap<String, Arc<TriggerEntry>>,
        event_bus: Arc<EventBus>,
    }

    // ─── WebhookTriggerServer ─────────────────────────────────────────────────

    /// 内核级 Webhook 触发服务器。
    ///
    /// 监听单一端口，通过 `/hooks/{trigger_id}` 路径区分不同触发器，
    /// 验证签名后将请求转换为 [`TriggerEvent::Webhook`] 并发布到 [`EventBus`]。
    pub struct WebhookTriggerServer {
        bind_addr: String,
        state: Arc<ServerState>,
        server_handle: Mutex<Option<JoinHandle<()>>>,
        shutdown_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        local_addr: tokio::sync::RwLock<Option<SocketAddr>>,
    }

    impl WebhookTriggerServer {
        /// 创建新的 `WebhookTriggerServer`。
        ///
        /// - `bind_addr`: 监听地址，如 `"127.0.0.1:8090"` 或 `"0.0.0.0:0"`（随机端口）。
        /// - `event_bus`: 触发事件发布目标。
        pub fn new(bind_addr: impl Into<String>, event_bus: Arc<EventBus>) -> Self {
            Self {
                bind_addr: bind_addr.into(),
                state: Arc::new(ServerState {
                    triggers: DashMap::new(),
                    event_bus,
                }),
                server_handle: Mutex::new(None),
                shutdown_tx: Mutex::new(None),
                local_addr: tokio::sync::RwLock::new(None),
            }
        }

        /// 注册一个触发器。
        ///
        /// - `trigger_id`: 触发器唯一 ID，对应 URL 路径 `/hooks/{trigger_id}`。
        /// - `secret`: 可选 HMAC-SHA256 secret；`None` 表示跳过签名验证。
        /// - `target_agent`: 可选目标 Agent；`None` 表示广播。
        ///
        /// 若 `trigger_id` 已存在，则覆盖更新。
        pub fn register_trigger(
            &self,
            trigger_id: impl Into<String>,
            secret: Option<String>,
            target_agent: Option<AgentId>,
        ) {
            let id = trigger_id.into();
            tracing::debug!(
                trigger_id = %id,
                has_secret = secret.is_some(),
                "WebhookTriggerServer: registering trigger"
            );
            self.state
                .triggers
                .insert(id, Arc::new(TriggerEntry::new(secret, target_agent)));
        }

        /// 注销一个触发器。返回 `true` 表示找到并移除成功。
        pub fn unregister_trigger(&self, trigger_id: &str) -> bool {
            self.state.triggers.remove(trigger_id).is_some()
        }

        /// 返回已注册的触发器 ID 列表。
        pub fn list_triggers(&self) -> Vec<String> {
            self.state
                .triggers
                .iter()
                .map(|e| e.key().clone())
                .collect()
        }

        /// 启动 HTTP 服务器（非阻塞，在后台 tokio 任务中运行）。
        ///
        /// 返回实际绑定的 `SocketAddr`（端口为 0 时由 OS 分配）。
        pub async fn start(&self) -> Result<SocketAddr, WebhookServerError> {
            {
                let guard = self.server_handle.lock().await;
                if guard.is_some() {
                    return Err(WebhookServerError::AlreadyRunning);
                }
            }

            let addr: SocketAddr = self
                .bind_addr
                .parse()
                .map_err(|e| WebhookServerError::InvalidAddress(format!("{}", e)))?;

            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|e| WebhookServerError::BindFailed(e.to_string()))?;

            let bound_addr = listener
                .local_addr()
                .map_err(|e| WebhookServerError::BindFailed(e.to_string()))?;

            *self.local_addr.write().await = Some(bound_addr);

            let app = Router::new()
                .route("/hooks/:trigger_id", post(handle_webhook))
                .with_state(Arc::clone(&self.state));

            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
            *self.shutdown_tx.lock().await = Some(shutdown_tx);

            let handle = tokio::spawn(async move {
                let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                });
                let _ = server.await;
            });

            *self.server_handle.lock().await = Some(handle);

            tracing::info!(
                addr = %bound_addr,
                "WebhookTriggerServer: started"
            );

            Ok(bound_addr)
        }

        /// 优雅关闭服务器。
        pub async fn stop(&self) -> Result<(), WebhookServerError> {
            let handle = self.server_handle.lock().await.take();
            if handle.is_none() {
                return Err(WebhookServerError::NotRunning);
            }

            if let Some(tx) = self.shutdown_tx.lock().await.take() {
                let _ = tx.send(());
            }

            if let Some(h) = handle {
                let _ = h.await;
            }

            *self.local_addr.write().await = None;
            tracing::info!("WebhookTriggerServer: stopped");
            Ok(())
        }

        /// 返回绑定地址（服务器启动后有效）。
        pub async fn local_addr(&self) -> Option<SocketAddr> {
            *self.local_addr.read().await
        }

        /// 返回服务器是否正在运行。
        pub async fn is_running(&self) -> bool {
            self.server_handle.lock().await.is_some()
        }
    }

    // ─── Request handler ──────────────────────────────────────────────────────

    async fn handle_webhook(
        Path(trigger_id): Path<String>,
        State(state): State<Arc<ServerState>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        // 1. 查找触发器注册记录
        let entry = match state.triggers.get(&trigger_id) {
            Some(e) => Arc::clone(&*e),
            None => {
                tracing::warn!(
                    trigger_id = %trigger_id,
                    "WebhookTriggerServer: unknown trigger_id"
                );
                return (StatusCode::NOT_FOUND, "trigger not found").into_response();
            }
        };

        // 2. 限流检查（滑动窗口，60s 重置）
        let rate_limited = {
            let now = Instant::now();
            let mut counter = entry
                .rate_counter
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if now.duration_since(counter.1) >= Duration::from_secs(60) {
                *counter = (1, now);
                false
            } else if counter.0 >= entry.max_per_minute {
                true
            } else {
                counter.0 += 1;
                false
            }
        };
        if rate_limited {
            tracing::warn!(
                trigger_id = %trigger_id,
                "WebhookTriggerServer: rate limit exceeded"
            );
            return (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response();
        }

        // 3. 幂等去重（基于 X-Request-Id，60s TTL）
        if let Some(rid) = headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
        {
            let now = Instant::now();
            let ttl = Duration::from_secs(60);
            entry
                .dedup_cache
                .retain(|_, inserted_at| now.duration_since(*inserted_at) < ttl);
            if entry.dedup_cache.contains_key(&rid) {
                tracing::debug!(
                    trigger_id = %trigger_id,
                    request_id = %rid,
                    "WebhookTriggerServer: duplicate request skipped"
                );
                return (StatusCode::OK, r#"{"status":"duplicate","skipped":true}"#).into_response();
            }
            entry.dedup_cache.insert(rid, now);
        }

        // 4. HMAC-SHA256 验证（可选）
        if let Some(ref secret) = entry.secret {
            // 从 headers 中查找签名（兼容 X-Hub-Signature-256 和 X-Signature）
            let sig_header = headers
                .get("x-hub-signature-256")
                .or_else(|| headers.get("x-signature"))
                .and_then(|v| v.to_str().ok());

            match sig_header {
                None => {
                    tracing::warn!(
                        trigger_id = %trigger_id,
                        "WebhookTriggerServer: missing HMAC signature header"
                    );
                    return (StatusCode::UNAUTHORIZED, "missing signature").into_response();
                }
                Some(sig) => {
                    // 剥离 "sha256=" 前缀后比较
                    let sig_hex = sig.strip_prefix("sha256=").unwrap_or(sig);
                    let expected = compute_hmac_sha256(secret, &body);
                    let expected_hex = expected
                        .iter()
                        .fold(String::new(), |mut s, b| {
                            use std::fmt::Write;
                            write!(s, "{:02x}", b).unwrap();
                            s
                        });
                    if sig_hex != expected_hex {
                        tracing::warn!(
                            trigger_id = %trigger_id,
                            "WebhookTriggerServer: HMAC verification failed"
                        );
                        return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
                    }
                }
            }
        }

        // 5. 解析 payload（优先尝试 JSON，失败则包装为 raw 字符串）
        let payload: serde_json::Value = if body.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&body).unwrap_or_else(|_| {
                serde_json::json!({
                    "raw": String::from_utf8_lossy(&body).to_string()
                })
            })
        };

        // 6. 发布 TriggerEvent::Webhook 到 EventBus
        let ev = TriggerEvent::webhook(
            trigger_id.clone(),
            payload,
            entry.target_agent.clone(),
        );

        tracing::debug!(
            trigger_id = %trigger_id,
            event_id = %ev.id,
            "WebhookTriggerServer: publishing TriggerEvent::Webhook"
        );

        state.event_bus.publish(Event::TriggerFired(ev));

        (StatusCode::OK, r#"{"status":"ok"}"#).into_response()
    }

    // ─── Tests ────────────────────────────────────────────────────────────────

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::{event_bus::EventBus, events::Event};

        fn make_server() -> WebhookTriggerServer {
            let bus = Arc::new(EventBus::new());
            WebhookTriggerServer::new("127.0.0.1:0", bus)
        }

        // ── 1. 注册 / 注销 / 列表 ─────────────────────────────────────────
        #[test]
        fn test_register_unregister_list() {
            let server = make_server();
            server.register_trigger("gh", None, None);
            server.register_trigger("stripe", Some("s3cr3t".into()), None);

            let mut list = server.list_triggers();
            list.sort();
            assert_eq!(list, vec!["gh", "stripe"]);

            assert!(server.unregister_trigger("gh"));
            assert!(!server.unregister_trigger("gh")); // 已不存在
            assert_eq!(server.list_triggers(), vec!["stripe"]);
        }

        // ── 2. start / stop 生命周期 ─────────────────────────────────────
        #[tokio::test]
        async fn test_start_stop() {
            let server = make_server();
            assert!(!server.is_running().await);

            let addr = server.start().await.unwrap();
            assert!(server.is_running().await);
            assert!(server.local_addr().await.is_some());
            assert_eq!(server.local_addr().await.unwrap(), addr);

            server.stop().await.unwrap();
            assert!(!server.is_running().await);
            assert!(server.local_addr().await.is_none());
        }

        // ── 3. 重复启动应返回 AlreadyRunning ─────────────────────────────
        #[tokio::test]
        async fn test_start_already_running() {
            let server = make_server();
            server.start().await.unwrap();
            let result = server.start().await;
            assert!(matches!(result, Err(WebhookServerError::AlreadyRunning)));
            server.stop().await.unwrap();
        }

        // ── 4. 未知 trigger_id 返回 404 ──────────────────────────────────
        #[tokio::test]
        async fn test_unknown_trigger_returns_404() {
            let bus = Arc::new(EventBus::new());
            let server = WebhookTriggerServer::new("127.0.0.1:0", Arc::clone(&bus));
            let addr = server.start().await.unwrap();

            let client = reqwest::Client::new();
            let resp = client
                .post(format!("http://{}/hooks/nonexistent", addr))
                .body("{}")
                .send()
                .await
                .unwrap();

            assert_eq!(resp.status(), 404);
            server.stop().await.unwrap();
        }

        // ── 5. 已注册触发器发布 TriggerFired 事件 ─────────────────────────
        #[tokio::test]
        async fn test_known_trigger_publishes_event() {
            let bus = Arc::new(EventBus::new());
            let mut rx = bus.subscribe();
            let server = WebhookTriggerServer::new("127.0.0.1:0", Arc::clone(&bus));
            server.register_trigger("gh-push", None, None);
            let addr = server.start().await.unwrap();

            let client = reqwest::Client::new();
            let resp = client
                .post(format!("http://{}/hooks/gh-push", addr))
                .header("content-type", "application/json")
                .body(r#"{"action":"push"}"#)
                .send()
                .await
                .unwrap();

            assert_eq!(resp.status(), 200);

            // 等待事件到达 EventBus
            tokio::time::timeout(tokio::time::Duration::from_millis(500), async {
                loop {
                    if let Ok(event) = rx.recv().await {
                        if let Event::TriggerFired(ev) = event {
                            assert_eq!(ev.trigger_id, "gh-push");
                            assert_eq!(
                                ev.payload.get("action").and_then(|v| v.as_str()),
                                Some("push")
                            );
                            break;
                        }
                    }
                }
            })
            .await
            .expect("TriggerFired event should arrive within 500ms");

            server.stop().await.unwrap();
        }

        // ── 6. HMAC 验证失败返回 401 ──────────────────────────────────────
        #[tokio::test]
        async fn test_hmac_verification_failure_returns_401() {
            let bus = Arc::new(EventBus::new());
            let server = WebhookTriggerServer::new("127.0.0.1:0", Arc::clone(&bus));
            server.register_trigger("secured", Some("real-secret".into()), None);
            let addr = server.start().await.unwrap();

            let client = reqwest::Client::new();
            // 错误签名
            let resp = client
                .post(format!("http://{}/hooks/secured", addr))
                .header("x-hub-signature-256", "sha256=deadbeef")
                .body(r#"{"data":"test"}"#)
                .send()
                .await
                .unwrap();

            assert_eq!(resp.status(), 401);
            server.stop().await.unwrap();
        }

        // ── 7. HMAC 验证正确时正常处理 ────────────────────────────────────
        #[tokio::test]
        async fn test_hmac_verification_success() {
            let bus = Arc::new(EventBus::new());
            let server = WebhookTriggerServer::new("127.0.0.1:0", Arc::clone(&bus));
            let secret = "my-webhook-secret";
            server.register_trigger("secured-ok", Some(secret.into()), None);
            let addr = server.start().await.unwrap();

            let body = br#"{"event":"test"}"#;
            let sig_bytes = compute_hmac_sha256(secret, body);
            let sig_hex: String = sig_bytes.iter().fold(String::new(), |mut s, b| {
                use std::fmt::Write;
                write!(s, "{:02x}", b).unwrap();
                s
            });
            let sig_header = format!("sha256={}", sig_hex);

            let client = reqwest::Client::new();
            let resp = client
                .post(format!("http://{}/hooks/secured-ok", addr))
                .header("x-hub-signature-256", sig_header)
                .body(body.as_ref())
                .send()
                .await
                .unwrap();

            assert_eq!(resp.status(), 200);
            server.stop().await.unwrap();
        }

        // ── 8. 空 body 触发器（Cron-style webhook）──────────────────────
        #[tokio::test]
        async fn test_empty_body_yields_null_payload() {
            let bus = Arc::new(EventBus::new());
            let mut rx = bus.subscribe();
            let server = WebhookTriggerServer::new("127.0.0.1:0", Arc::clone(&bus));
            server.register_trigger("ping", None, None);
            let addr = server.start().await.unwrap();

            let client = reqwest::Client::new();
            client
                .post(format!("http://{}/hooks/ping", addr))
                .send()
                .await
                .unwrap();

            tokio::time::timeout(tokio::time::Duration::from_millis(300), async {
                loop {
                    if let Ok(Event::TriggerFired(ev)) = rx.recv().await {
                        if ev.trigger_id == "ping" {
                            assert_eq!(ev.payload, serde_json::Value::Null);
                            break;
                        }
                    }
                }
            })
            .await
            .expect("TriggerFired event should arrive");

            server.stop().await.unwrap();
        }

        // ── 9. 幂等去重：相同 X-Request-Id 第二次返回 200 + skipped ─────
        #[tokio::test]
        async fn test_dedup_same_request_id() {
            let bus = Arc::new(EventBus::new());
            let server = WebhookTriggerServer::new("127.0.0.1:0", Arc::clone(&bus));
            server.register_trigger("dedup-test", None, None);
            let addr = server.start().await.unwrap();

            let client = reqwest::Client::new();
            let url = format!("http://{}/hooks/dedup-test", addr);

            // 第一次请求
            let resp1 = client
                .post(&url)
                .header("x-request-id", "unique-id-42")
                .body("{}")
                .send()
                .await
                .unwrap();
            assert_eq!(resp1.status(), 200);
            let body1 = resp1.text().await.unwrap();
            assert!(!body1.contains("skipped"), "first request should not be skipped");

            // 第二次相同 X-Request-Id
            let resp2 = client
                .post(&url)
                .header("x-request-id", "unique-id-42")
                .body("{}")
                .send()
                .await
                .unwrap();
            assert_eq!(resp2.status(), 200);
            let body2 = resp2.text().await.unwrap();
            assert!(body2.contains("skipped"), "duplicate request should be skipped");

            server.stop().await.unwrap();
        }

        // ── 10. target_agent 传递到 TriggerEvent ─────────────────────────
        #[tokio::test]
        async fn test_target_agent_propagated() {
            let bus = Arc::new(EventBus::new());
            let mut rx = bus.subscribe();
            let target = AgentId::new("my-agent");
            let server = WebhookTriggerServer::new("127.0.0.1:0", Arc::clone(&bus));
            server.register_trigger("targeted", None, Some(target.clone()));
            let addr = server.start().await.unwrap();

            let client = reqwest::Client::new();
            client
                .post(format!("http://{}/hooks/targeted", addr))
                .body("{}")
                .send()
                .await
                .unwrap();

            tokio::time::timeout(tokio::time::Duration::from_millis(300), async {
                loop {
                    if let Ok(Event::TriggerFired(ev)) = rx.recv().await {
                        if ev.trigger_id == "targeted" {
                            assert_eq!(ev.target_agent, Some(target.clone()));
                            break;
                        }
                    }
                }
            })
            .await
            .expect("TriggerFired event with target_agent should arrive");

            server.stop().await.unwrap();
        }
    }
}

// ─── Public re-exports ─────────────────────────────────────────────────────────

#[cfg(feature = "webhook")]
pub use inner::{WebhookServerError, WebhookTriggerServer};

/// Webhook 触发服务器配置（用于依赖注入场景）。
#[cfg(feature = "webhook")]
pub struct WebhookTriggerConfig {
    /// 绑定地址，如 `"0.0.0.0:8090"`。
    pub bind_addr: String,
}

#[cfg(feature = "webhook")]
impl WebhookTriggerConfig {
    /// 创建新配置。
    pub fn new(bind_addr: impl Into<String>) -> Self {
        Self {
            bind_addr: bind_addr.into(),
        }
    }
}
