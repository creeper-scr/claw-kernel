//! WebhookChannel — HTTP webhook 收发适配器。
//!
//! 启动一个 axum HTTP 服务器，在 `POST /webhook` 接收入站消息；
//! 通过 reqwest 将出站消息 POST 到配置的目标 URL。

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use claw_pal::retry::{with_retry_mapped, RetryConfig};
use tokio::sync::{mpsc, Mutex};

use crate::{
    error::ChannelError,
    traits::{Channel, ChannelEvent, ChannelEventPublisher},
    types::{ChannelId, ChannelMessage, Platform},
};

// ── Per-trigger token-bucket rate limiter ─────────────────────────────────────

/// 令牌桶速率限制器，用于 per-channel 限流。
///
/// 默认配置：容量 100 个令牌，补充速率 100/60 ≈ 1.667 token/s（即 100 req/min）。
pub(crate) struct TokenBucket {
    capacity: f64,
    tokens: f64,
    last_refill: Instant,
    refill_rate: f64, // tokens/sec
}

impl TokenBucket {
    /// 创建令牌桶。`capacity` 为最大令牌数，`per` 为补充周期。
    ///
    /// 例：`TokenBucket::new(100, Duration::from_secs(60))` → 100 req/min。
    pub fn new(capacity: u32, per: Duration) -> Self {
        let cap = capacity as f64;
        let rate = cap / per.as_secs_f64();
        Self {
            capacity: cap,
            tokens: cap,
            last_refill: Instant::now(),
            refill_rate: rate,
        }
    }

    /// 尝试消耗 1 个令牌。返回 `true` 表示允许，`false` 表示触发限流（429）。
    pub fn try_consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl Default for TokenBucket {
    fn default() -> Self {
        Self::new(100, Duration::from_secs(60))
    }
}

// ── Request deduplication ─────────────────────────────────────────────────────

const DEDUP_TTL: Duration = Duration::from_secs(60);
const DEDUP_GC_INTERVAL: usize = 64;

/// HTTP 请求去重缓存。
///
/// 以 `X-Request-Id` 或 `X-Idempotency-Key` 为 key，60s 内相同 ID 的请求只处理一次。
/// 过期条目采用惰性 GC：每隔 `gc_interval` 次检查一次，清理所有已超过 TTL 的条目。
struct RequestDeduplicator {
    seen: StdMutex<HashMap<String, Instant>>,
    ttl: Duration,
    call_count: AtomicUsize,
    gc_interval: usize,
}

impl RequestDeduplicator {
    fn new() -> Self {
        Self {
            seen: StdMutex::new(HashMap::new()),
            ttl: DEDUP_TTL,
            call_count: AtomicUsize::new(0),
            gc_interval: DEDUP_GC_INTERVAL,
        }
    }

    /// 检查 `request_id` 是否为重复请求，并将其记录到缓存中。
    ///
    /// - 返回 `true`：新请求，允许处理。
    /// - 返回 `false`：重复请求，应丢弃（返回 HTTP 200 静默 ACK，避免提供方重试）。
    fn check_and_insert(&self, request_id: &str) -> bool {
        let now = Instant::now();
        let prev = self.call_count.fetch_add(1, Ordering::Relaxed);

        let mut map = self.seen.lock().unwrap();

        // 惰性 GC：定期清理过期条目
        if prev % self.gc_interval == 0 {
            let ttl = self.ttl;
            map.retain(|_, t| now.duration_since(*t) < ttl);
        }

        if let Some(t) = map.get(request_id) {
            if now.duration_since(*t) < self.ttl {
                return false; // 重复请求
            }
        }

        map.insert(request_id.to_string(), now);
        true // 新请求
    }
}

// ── HMAC-SHA256 signature verification ───────────────────────────────────────

/// Verify a HMAC-SHA256 request signature.
///
/// `secret`    — the shared webhook secret (bytes)
/// `body`      — raw request body
/// `signature` — value from the `X-Hub-Signature-256` header; may optionally
///               be prefixed with `"sha256="`, which is stripped automatically.
///
/// Uses the `hmac` crate's constant-time comparison so that timing attacks
/// cannot leak information about the expected HMAC value.
fn verify_hmac_sha256(secret: &[u8], body: &[u8], signature: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);

    // Strip optional "sha256=" prefix.
    let sig_hex = signature.strip_prefix("sha256=").unwrap_or(signature);
    let Ok(sig_bytes) = hex::decode(sig_hex) else {
        return false;
    };

    // mac.verify_slice uses constant-time comparison internally.
    mac.verify_slice(&sig_bytes).is_ok()
}

/// Webhook 双向通道。
///
/// - **入站**：监听 `POST /webhook`，将 JSON 体（`ChannelMessage`）推入内部队列。
/// - **出站**：若配置了 `outbound_url`，将消息 POST 到该 URL。
pub struct WebhookChannel {
    id: ChannelId,
    /// Agent identifier forwarded in published channel events.
    agent_id: String,
    bind_addr: SocketAddr,
    inbound_tx: mpsc::Sender<ChannelMessage>,
    inbound_rx: Mutex<mpsc::Receiver<ChannelMessage>>,
    outbound_url: Option<String>,
    /// Shared HMAC secret used to verify `X-Hub-Signature-256` on incoming
    /// requests.  `None` disables signature checking (development only).
    secret: Option<String>,
    client: reqwest::Client,
    server_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Retry policy for outbound send() calls.
    retry_config: RetryConfig,
    /// 实际监听地址（connect() 之后设置，用于端口 0 场景）。
    local_addr: Mutex<Option<SocketAddr>>,
    /// Guards against double-start: true after the first successful connect().
    started: AtomicBool,
    /// Optional EventBus publisher — wires the channel into the runtime event system.
    event_publisher: Option<Arc<dyn ChannelEventPublisher>>,
    /// Request-ID based deduplication cache (X-Request-Id / X-Idempotency-Key).
    deduplicator: Arc<RequestDeduplicator>,
    /// Per-channel inbound rate limiter (token bucket). Default: 100 req/min.
    rate_bucket: Arc<StdMutex<TokenBucket>>,
}

impl WebhookChannel {
    /// 创建新的 WebhookChannel。
    ///
    /// - `id`：通道唯一标识。
    /// - `bind_addr`：axum 服务器监听地址（可使用端口 0 让系统自动分配）。
    /// - `outbound_url`：出站消息目标 URL；为 `None` 时调用 `send()` 会返回错误。
    ///
    /// 默认限流：100 req/min（令牌桶容量 100，补充速率 100/60 token/s）。
    pub fn new(id: ChannelId, bind_addr: SocketAddr, outbound_url: Option<String>) -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self {
            id,
            agent_id: String::new(),
            bind_addr,
            inbound_tx: tx,
            inbound_rx: Mutex::new(rx),
            outbound_url,
            secret: None,
            client: reqwest::Client::new(),
            server_handle: Mutex::new(None),
            retry_config: RetryConfig::default(),
            local_addr: Mutex::new(None),
            started: AtomicBool::new(false),
            event_publisher: None,
            deduplicator: Arc::new(RequestDeduplicator::new()),
            rate_bucket: Arc::new(StdMutex::new(TokenBucket::default())),
        }
    }

    /// 返回服务器实际监听地址（仅在 `connect()` 之后有效）。
    pub async fn local_addr(&self) -> Option<SocketAddr> {
        *self.local_addr.lock().await
    }

    /// 设置用于验证入站 webhook 签名的 HMAC 密钥。
    ///
    /// 启用后，每个 `POST /webhook` 请求都必须携带有效的 `X-Hub-Signature-256` 头，
    /// 否则服务器将返回 `401 Unauthorized`。
    /// 密钥为 `None`（默认值）时跳过签名校验（仅限开发环境）。
    ///
    /// Returns `Err(ChannelError::InvalidConfig)` if `secret` is empty.
    pub fn with_secret(mut self, secret: impl Into<String>) -> Result<Self, ChannelError> {
        let secret = secret.into();
        if secret.is_empty() {
            return Err(ChannelError::InvalidConfig(
                "HMAC secret cannot be empty".to_string(),
            ));
        }
        self.secret = Some(secret);
        Ok(self)
    }

    /// Override the retry policy applied to outbound `send()` calls.
    ///
    /// The default policy retries up to 3 times with 500 ms base delay capped
    /// at 30 s.  Useful in tests to set sub-millisecond delays.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// 覆盖入站限流速率。
    ///
    /// - `capacity`：令牌桶容量（突发上限）。
    /// - `per`：补充 `capacity` 个令牌所需时长（即窗口大小）。
    ///
    /// 例：`with_rate_limit(100, Duration::from_secs(60))` → 100 req/min（默认值）。
    /// 例：`with_rate_limit(10, Duration::from_secs(1))` → 10 req/s。
    pub fn with_rate_limit(mut self, capacity: u32, per: Duration) -> Self {
        self.rate_bucket = Arc::new(StdMutex::new(TokenBucket::new(capacity, per)));
        self
    }

    /// Attach a [`ChannelEventPublisher`] to wire this channel into the
    /// runtime EventBus.
    ///
    /// Once set, `send()` publishes [`ChannelEvent::MessageSent`] and
    /// `recv()` publishes [`ChannelEvent::MessageReceived`] on every
    /// call.  `connect()` and `disconnect()` publish
    /// [`ChannelEvent::ConnectionState`].  All publish calls are
    /// best-effort; failures do not affect the primary send/recv result.
    pub fn with_event_publisher(
        mut self,
        agent_id: impl Into<String>,
        publisher: Arc<dyn ChannelEventPublisher>,
    ) -> Self {
        self.agent_id = agent_id.into();
        self.event_publisher = Some(publisher);
        self
    }
}

#[async_trait]
impl Channel for WebhookChannel {
    fn platform(&self) -> &str {
        "webhook"
    }

    fn channel_id(&self) -> &ChannelId {
        &self.id
    }

    /// 启动 axum 服务器，开始接收入站 webhook 消息。
    async fn connect(&self) -> Result<(), ChannelError> {
        // Idempotent: if already started, return immediately.
        if self
            .started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }

        if self.secret.is_none() {
            tracing::error!(
                "⚠️  SECURITY: WebhookChannel is running WITHOUT HMAC signature verification. \
                 Any incoming request will be accepted. Call .with_secret() for production use."
            );
        }

        let tx = self.inbound_tx.clone();
        // 在同步上下文中绑定端口，可立即获得实际监听地址（端口 0 时尤其有用）。
        let std_listener = std::net::TcpListener::bind(self.bind_addr)
            .map_err(|e| ChannelError::ConnectionFailed(format!("bind failed: {e}")))?;
        std_listener
            .set_nonblocking(true)
            .map_err(|e| ChannelError::ConnectionFailed(format!("set_nonblocking: {e}")))?;
        let listener = tokio::net::TcpListener::from_std(std_listener)
            .map_err(|e| ChannelError::ConnectionFailed(format!("from_std: {e}")))?;

        // 记录实际监听地址
        let addr = listener
            .local_addr()
            .map_err(|e| ChannelError::ConnectionFailed(format!("local_addr: {e}")))?;
        *self.local_addr.lock().await = Some(addr);

        let secret = self.secret.clone();
        let deduplicator = Arc::clone(&self.deduplicator);
        let rate_bucket = Arc::clone(&self.rate_bucket);

        let router = Router::new().route(
            "/webhook",
            post(move |headers: HeaderMap, body: Bytes| {
                let tx = tx.clone();
                let secret = secret.clone();
                let deduplicator = Arc::clone(&deduplicator);
                let rate_bucket = Arc::clone(&rate_bucket);
                async move {
                    // GAP-F6-05: per-channel token-bucket rate limiting (100 req/min default).
                    if !rate_bucket.lock().unwrap().try_consume() {
                        return (StatusCode::TOO_MANY_REQUESTS, "Too Many Requests")
                            .into_response();
                    }

                    // FIX-01: verify HMAC-SHA256 when a shared secret is configured.
                    if let Some(ref s) = secret {
                        let sig = headers
                            .get("x-hub-signature-256")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        if !verify_hmac_sha256(s.as_bytes(), &body, sig) {
                            return (StatusCode::UNAUTHORIZED, "Invalid signature")
                                .into_response();
                        }
                    }

                    // Request deduplication via X-Request-Id / X-Idempotency-Key.
                    let request_id = headers
                        .get("x-request-id")
                        .or_else(|| headers.get("x-idempotency-key"))
                        .and_then(|v| v.to_str().ok());
                    if let Some(rid) = request_id {
                        if !deduplicator.check_and_insert(rid) {
                            // Duplicate — silent ACK so provider doesn't retry.
                            return StatusCode::OK.into_response();
                        }
                    }

                    // Parse JSON and enqueue message.
                    match serde_json::from_slice::<ChannelMessage>(&body) {
                        Ok(mut msg) => {
                            // G-13: bridge the HTTP-layer dedup key into msg.id so
                            // that DeduplicatingRouter uses the same canonical
                            // identifier.  This gives defense-in-depth: even if a
                            // duplicate slips past the RequestDeduplicator (e.g.
                            // after a server restart), the router layer will still
                            // suppress it because both layers share the same key.
                            if let Some(rid) = request_id {
                                msg.id = rid.to_string();
                            }
                            // 忽略满队列错误（接收方关闭时服务器也将随之关闭）
                            let _ = tx.send(msg).await;
                            StatusCode::OK.into_response()
                        }
                        Err(_) => (StatusCode::BAD_REQUEST, "Invalid JSON").into_response(),
                    }
                }
            }),
        );

        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });

        *self.server_handle.lock().await = Some(handle);

        if let Some(pub_) = &self.event_publisher {
            let _ = pub_
                .publish(ChannelEvent::ConnectionState {
                    channel: self.id.to_string(),
                    platform: Platform::Webhook,
                    connected: true,
                })
                .await;
        }
        Ok(())
    }

    /// 将消息 POST 到 `outbound_url`，失败时自动指数退避重试（最多 3 次）。
    ///
    /// **重试条件**：网络层错误（连接超时、TCP 重置等）以及 HTTP 429 / 5xx 状态码。
    /// 4xx 非限流错误（如 400、401、403）视为永久性错误，直接返回，不再重试。
    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError> {
        let url = match &self.outbound_url {
            Some(u) => u.clone(),
            None => {
                return Err(ChannelError::SendFailed(
                    "no outbound_url configured".to_string(),
                ))
            }
        };

        let client = self.client.clone();

        // Local enum lets `is_retryable` distinguish transient from permanent
        // errors without adding new variants to the public `ChannelError` type.
        enum SendErr {
            Transient(String),
            Permanent(ChannelError),
        }

        let result = with_retry_mapped(
            || {
                let client = client.clone();
                let url = url.clone();
                let message = message.clone();
                async move {
                    match client.post(&url).json(&message).send().await {
                        Err(e) => {
                            // Network-level error (timeout, connection refused, …)
                            Err(SendErr::Transient(e.to_string()))
                        }
                        Ok(resp) => {
                            let status = resp.status().as_u16();
                            if status < 300 {
                                Ok(())
                            } else if status == 429 || status >= 500 {
                                // Rate-limited or server-side fault — retryable.
                                Err(SendErr::Transient(format!("HTTP {status}")))
                            } else {
                                // Permanent 4xx error — don't retry.
                                Err(SendErr::Permanent(ChannelError::SendFailed(format!(
                                    "HTTP {status}"
                                ))))
                            }
                        }
                    }
                }
            },
            &self.retry_config,
            |e| matches!(e, SendErr::Transient(_)),
        )
        .await;

        let channel_result = match result {
            Ok(()) => Ok(()),
            Err(SendErr::Transient(msg)) => Err(ChannelError::SendFailed(msg)),
            Err(SendErr::Permanent(e)) => Err(e),
        };

        if let Some(pub_) = &self.event_publisher {
            let _ = pub_
                .publish(ChannelEvent::MessageSent {
                    agent_id: self.agent_id.clone(),
                    channel: self.id.to_string(),
                    platform: Platform::Webhook,
                    success: channel_result.is_ok(),
                })
                .await;
        }
        channel_result
    }

    /// 从内部队列取出下一条入站消息（阻塞直到消息到达）。
    async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
        let result = self
            .inbound_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| ChannelError::ReceiveFailed("channel closed".to_string()));

        if let (Ok(msg), Some(pub_)) = (&result, &self.event_publisher) {
            let _ = pub_
                .publish(ChannelEvent::MessageReceived {
                    agent_id: self.agent_id.clone(),
                    channel: self.id.to_string(),
                    platform: Platform::Webhook,
                    content_preview: msg.content.chars().take(64).collect(),
                })
                .await;
        }
        result
    }

    /// 终止后台 axum 服务器任务。
    async fn disconnect(&self) -> Result<(), ChannelError> {
        if let Some(handle) = self.server_handle.lock().await.take() {
            handle.abort();
        }

        if let Some(pub_) = &self.event_publisher {
            let _ = pub_
                .publish(ChannelEvent::ConnectionState {
                    channel: self.id.to_string(),
                    platform: Platform::Webhook,
                    connected: false,
                })
                .await;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "webhook")]
mod tests {
    use super::*;
    use crate::types::{MessageDirection, Platform};

    fn make_channel(outbound_url: Option<String>) -> WebhookChannel {
        // 端口 0：系统自动分配；connect() 后通过 local_addr() 获取实际端口
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        WebhookChannel::new(ChannelId::new("wh-test"), addr, outbound_url)
    }

    #[test]
    fn test_webhook_channel_new() {
        // 构造不应 panic
        let ch = make_channel(None);
        assert_eq!(ch.id.as_str(), "wh-test");
    }

    #[test]
    fn test_webhook_channel_platform() {
        let ch = make_channel(None);
        assert_eq!(ch.platform(), "webhook");
    }

    // ── TokenBucket unit tests ────────────────────────────────────────────────

    #[test]
    fn test_token_bucket_allows_within_capacity() {
        let mut bucket = TokenBucket::new(3, Duration::from_secs(60));
        assert!(bucket.try_consume(), "1st request should be allowed");
        assert!(bucket.try_consume(), "2nd request should be allowed");
        assert!(bucket.try_consume(), "3rd request should be allowed");
    }

    #[test]
    fn test_token_bucket_rejects_when_exhausted() {
        let mut bucket = TokenBucket::new(2, Duration::from_secs(60));
        assert!(bucket.try_consume());
        assert!(bucket.try_consume());
        assert!(!bucket.try_consume(), "3rd request should be rejected (429)");
    }

    #[test]
    fn test_token_bucket_refills_over_time() {
        // 极短窗口：1 token / 1ms → 高补充速率，易于测试
        let mut bucket = TokenBucket::new(1, Duration::from_millis(1));
        assert!(bucket.try_consume(), "initial token consumed");
        assert!(!bucket.try_consume(), "no tokens left");

        // 等待超过 1ms，令牌应已补充
        std::thread::sleep(Duration::from_millis(5));
        assert!(bucket.try_consume(), "token refilled after sleep");
    }

    // ── Rate limiting HTTP integration test ──────────────────────────────────

    /// 验证超出速率限制的请求返回 HTTP 429。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_webhook_rate_limit_returns_429() {
        // 容量 2，窗口 60s → 只允许 2 次请求
        let ch = WebhookChannel::new(
            ChannelId::new("wh-rl"),
            "127.0.0.1:0".parse().unwrap(),
            None,
        )
        .with_rate_limit(2, Duration::from_secs(60));

        ch.connect().await.expect("connect");
        let addr = ch.local_addr().await.expect("local_addr");
        let url = format!("http://{addr}/webhook");

        let msg = ChannelMessage {
            id: "rl-test".to_string(),
            channel_id: ChannelId::new("wh-rl"),
            direction: MessageDirection::Inbound,
            platform: Platform::Webhook,
            content: "rate limit test".to_string(),
            sender_id: None,
            thread_id: None,
            metadata: serde_json::Value::Null,
            timestamp_ms: 0,
        };

        let client = reqwest::Client::new();
        // 第 1 次：200
        let r1 = client.post(&url).json(&msg).send().await.unwrap();
        assert_eq!(r1.status(), 200, "1st request should succeed");
        // 第 2 次：200
        let r2 = client.post(&url).json(&msg).send().await.unwrap();
        assert_eq!(r2.status(), 200, "2nd request should succeed");
        // 第 3 次：429
        let r3 = client.post(&url).json(&msg).send().await.unwrap();
        assert_eq!(r3.status(), 429, "3rd request should be rate-limited");

        ch.disconnect().await.expect("disconnect");
    }

    /// 验证 with_rate_limit() 可正确覆盖默认配置。
    #[test]
    fn test_with_rate_limit_overrides_default() {
        let ch = WebhookChannel::new(
            ChannelId::new("wh-cfg"),
            "127.0.0.1:0".parse().unwrap(),
            None,
        )
        .with_rate_limit(5, Duration::from_secs(1));

        // 验证令牌桶初始容量为 5
        let mut bucket = ch.rate_bucket.lock().unwrap();
        for _ in 0..5 {
            assert!(bucket.try_consume(), "should allow 5 req/s");
        }
        assert!(!bucket.try_consume(), "6th should be rejected");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_webhook_channel_connect_and_send_receive() {
        let ch = WebhookChannel::new(ChannelId::new("wh-1"), "127.0.0.1:0".parse().unwrap(), None);
        ch.connect().await.expect("connect");

        // 通过 local_addr() 获取系统分配的实际端口（无 TOCTOU 竞争）
        let actual_addr = ch.local_addr().await.expect("local_addr after connect");

        // 向 /webhook 端点发送一条测试消息
        let msg = ChannelMessage {
            id: "test-id".to_string(),
            channel_id: ChannelId::new("wh-1"),
            direction: MessageDirection::Inbound,
            platform: Platform::Webhook,
            content: "hello webhook".to_string(),
            sender_id: None,
            thread_id: None,
            metadata: serde_json::Value::Null,
            timestamp_ms: 0,
        };

        let client = reqwest::Client::new();
        let url = format!("http://{actual_addr}/webhook");
        client
            .post(&url)
            .json(&msg)
            .send()
            .await
            .expect("POST to /webhook");

        // recv() 应当接收到刚才发送的消息
        let received = tokio::time::timeout(tokio::time::Duration::from_secs(3), ch.recv())
            .await
            .expect("recv timeout")
            .expect("recv ok");

        assert_eq!(received.content, "hello webhook");
        assert_eq!(received.channel_id.as_str(), "wh-1");

        ch.disconnect().await.expect("disconnect");
    }

    #[tokio::test]
    async fn test_webhook_send_no_outbound_url() {
        let ch = make_channel(None);
        let msg = ChannelMessage::inbound(ChannelId::new("wh-test"), Platform::Webhook, "out");
        let err = ch.send(msg).await.unwrap_err();
        assert!(err.to_string().contains("no outbound_url configured"));
    }

    /// Verify that send() retries on HTTP 5xx and eventually succeeds.
    ///
    /// A local axum server returns 503 for the first two requests, then 200.
    /// With fast retry delays the entire test completes in well under a second.
    #[tokio::test]
    async fn test_webhook_send_retries_on_transient_5xx() {
        use std::sync::atomic::{AtomicU32, Ordering as AO};
        use std::time::Duration;

        let call_count = Arc::new(AtomicU32::new(0));
        let cc = Arc::clone(&call_count);

        let app = axum::Router::new().route(
            "/target",
            axum::routing::post(move || {
                let count = cc.fetch_add(1, AO::SeqCst);
                async move {
                    if count < 2 {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::OK
                    }
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });

        let outbound_url = format!("http://{addr}/target");
        let ch = WebhookChannel::new(
            ChannelId::new("wh-retry"),
            "127.0.0.1:0".parse().unwrap(),
            Some(outbound_url),
        )
        .with_retry_config(
            RetryConfig::new()
                .with_max_retries(3)
                .with_base_delay(Duration::from_millis(1))
                .with_max_delay(Duration::from_millis(5)),
        );

        let msg = ChannelMessage::inbound(ChannelId::new("wh-retry"), Platform::Webhook, "test");
        ch.send(msg).await.expect("should succeed after retries");
        // 2 failures + 1 success = 3 calls
        assert_eq!(call_count.load(AO::SeqCst), 3);
    }

    /// Verify that a permanent 4xx error is returned immediately without retrying.
    #[tokio::test]
    async fn test_webhook_send_no_retry_on_4xx() {
        use std::sync::atomic::{AtomicU32, Ordering as AO};
        use std::time::Duration;

        let call_count = Arc::new(AtomicU32::new(0));
        let cc = Arc::clone(&call_count);

        let app = axum::Router::new().route(
            "/target",
            axum::routing::post(move || {
                cc.fetch_add(1, AO::SeqCst);
                async { StatusCode::BAD_REQUEST }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });

        let outbound_url = format!("http://{addr}/target");
        let ch = WebhookChannel::new(
            ChannelId::new("wh-perm"),
            "127.0.0.1:0".parse().unwrap(),
            Some(outbound_url),
        )
        .with_retry_config(
            RetryConfig::new()
                .with_max_retries(3)
                .with_base_delay(Duration::from_millis(1))
                .with_max_delay(Duration::from_millis(5)),
        );

        let msg = ChannelMessage::inbound(ChannelId::new("wh-perm"), Platform::Webhook, "test");
        let err = ch.send(msg).await.unwrap_err();
        assert!(err.to_string().contains("HTTP 400"));
        // No retries for permanent 4xx
        assert_eq!(call_count.load(AO::SeqCst), 1);
    }

    // ── RequestDeduplicator unit tests ───────────────────────────────────────

    #[test]
    fn test_deduplicator_allows_new_request() {
        let dedup = RequestDeduplicator::new();
        assert!(dedup.check_and_insert("req-1"), "首次请求应被允许");
    }

    #[test]
    fn test_deduplicator_rejects_duplicate_within_ttl() {
        let dedup = RequestDeduplicator::new();
        assert!(dedup.check_and_insert("req-dup"), "首次请求应被允许");
        assert!(!dedup.check_and_insert("req-dup"), "TTL 内重复请求应被拒绝");
    }

    #[test]
    fn test_deduplicator_allows_different_ids() {
        let dedup = RequestDeduplicator::new();
        assert!(dedup.check_and_insert("req-a"));
        assert!(dedup.check_and_insert("req-b"), "不同 ID 应都被允许");
    }

    #[test]
    fn test_deduplicator_allows_after_ttl_expired() {
        let dedup = RequestDeduplicator {
            seen: StdMutex::new(HashMap::new()),
            ttl: Duration::from_nanos(1),
            call_count: AtomicUsize::new(0),
            gc_interval: DEDUP_GC_INTERVAL,
        };
        assert!(dedup.check_and_insert("req-exp"), "首次请求允许");
        std::thread::sleep(Duration::from_millis(1));
        assert!(dedup.check_and_insert("req-exp"), "TTL 过期后应重新放行");
    }

    #[test]
    fn test_deduplicator_lazy_gc_removes_expired() {
        // gc_interval = 1 → 每次 check 都触发 GC
        let dedup = RequestDeduplicator {
            seen: StdMutex::new(HashMap::new()),
            ttl: Duration::from_nanos(1),
            call_count: AtomicUsize::new(0),
            gc_interval: 1,
        };
        dedup.check_and_insert("gc-a");
        dedup.check_and_insert("gc-b");
        std::thread::sleep(Duration::from_millis(1));
        // 触发 GC：清理过期条目后插入 gc-c
        dedup.check_and_insert("gc-c");
        // 过期条目已被清理，seen 中只有 gc-c
        assert_eq!(dedup.seen.lock().unwrap().len(), 1);
    }

    // ── HTTP 去重集成测试 ─────────────────────────────────────────────────────

    /// 相同 X-Request-Id 在 60s TTL 内只处理一次，第二次返回静默 200。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_webhook_dedup_x_request_id() {
        let ch = WebhookChannel::new(
            ChannelId::new("wh-dedup-rid"),
            "127.0.0.1:0".parse().unwrap(),
            None,
        );
        ch.connect().await.expect("connect");
        let addr = ch.local_addr().await.expect("local_addr");
        let url = format!("http://{addr}/webhook");

        let msg = ChannelMessage {
            id: "dedup-1".to_string(),
            channel_id: ChannelId::new("wh-dedup-rid"),
            direction: MessageDirection::Inbound,
            platform: Platform::Webhook,
            content: "dedup test".to_string(),
            sender_id: None,
            thread_id: None,
            metadata: serde_json::Value::Null,
            timestamp_ms: 0,
        };

        let client = reqwest::Client::new();
        // 第 1 次：200，消息入队
        let r1 = client
            .post(&url)
            .header("x-request-id", "unique-req-001")
            .json(&msg)
            .send()
            .await
            .unwrap();
        assert_eq!(r1.status(), 200, "首次请求应成功");

        // 接收第 1 条消息
        let received = tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            ch.recv(),
        )
        .await
        .expect("recv timeout")
        .expect("recv ok");
        assert_eq!(received.content, "dedup test");

        // 第 2 次：相同 X-Request-Id → 静默 200，但不入队
        let r2 = client
            .post(&url)
            .header("x-request-id", "unique-req-001")
            .json(&msg)
            .send()
            .await
            .unwrap();
        assert_eq!(r2.status(), 200, "重复请求应返回静默 200");

        // 队列中不应有第 2 条消息
        let no_msg = tokio::time::timeout(
            tokio::time::Duration::from_millis(100),
            ch.recv(),
        )
        .await;
        assert!(no_msg.is_err(), "重复请求不应入队");

        ch.disconnect().await.expect("disconnect");
    }

    /// X-Idempotency-Key 同样触发去重。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_webhook_dedup_x_idempotency_key() {
        let ch = WebhookChannel::new(
            ChannelId::new("wh-dedup-ik"),
            "127.0.0.1:0".parse().unwrap(),
            None,
        );
        ch.connect().await.expect("connect");
        let addr = ch.local_addr().await.expect("local_addr");
        let url = format!("http://{addr}/webhook");

        let msg = ChannelMessage {
            id: "dedup-ik".to_string(),
            channel_id: ChannelId::new("wh-dedup-ik"),
            direction: MessageDirection::Inbound,
            platform: Platform::Webhook,
            content: "idempotency test".to_string(),
            sender_id: None,
            thread_id: None,
            metadata: serde_json::Value::Null,
            timestamp_ms: 0,
        };

        let client = reqwest::Client::new();
        // 第 1 次（新 key）→ 入队
        let r1 = client
            .post(&url)
            .header("x-idempotency-key", "idem-key-abc")
            .json(&msg)
            .send()
            .await
            .unwrap();
        assert_eq!(r1.status(), 200);

        tokio::time::timeout(tokio::time::Duration::from_secs(3), ch.recv())
            .await
            .expect("recv timeout")
            .expect("recv ok");

        // 第 2 次（相同 key）→ 静默 ACK，不入队
        let r2 = client
            .post(&url)
            .header("x-idempotency-key", "idem-key-abc")
            .json(&msg)
            .send()
            .await
            .unwrap();
        assert_eq!(r2.status(), 200, "重复 idempotency key 应静默 ACK");

        let no_msg = tokio::time::timeout(
            tokio::time::Duration::from_millis(100),
            ch.recv(),
        )
        .await;
        assert!(no_msg.is_err(), "重复 idempotency key 不应入队");

        ch.disconnect().await.expect("disconnect");
    }

    /// 无 X-Request-Id / X-Idempotency-Key 时不做去重，请求正常处理。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_webhook_no_dedup_when_no_id_header() {
        let ch = WebhookChannel::new(
            ChannelId::new("wh-no-id"),
            "127.0.0.1:0".parse().unwrap(),
            None,
        );
        ch.connect().await.expect("connect");
        let addr = ch.local_addr().await.expect("local_addr");
        let url = format!("http://{addr}/webhook");

        let make_msg = |content: &str| ChannelMessage {
            id: content.to_string(),
            channel_id: ChannelId::new("wh-no-id"),
            direction: MessageDirection::Inbound,
            platform: Platform::Webhook,
            content: content.to_string(),
            sender_id: None,
            thread_id: None,
            metadata: serde_json::Value::Null,
            timestamp_ms: 0,
        };

        let client = reqwest::Client::new();
        // 无 ID header 的两条请求都应正常入队
        client.post(&url).json(&make_msg("msg-a")).send().await.unwrap();
        client.post(&url).json(&make_msg("msg-b")).send().await.unwrap();

        let m1 = tokio::time::timeout(tokio::time::Duration::from_secs(3), ch.recv())
            .await.expect("recv1 timeout").expect("recv1 ok");
        let m2 = tokio::time::timeout(tokio::time::Duration::from_secs(3), ch.recv())
            .await.expect("recv2 timeout").expect("recv2 ok");

        let contents: std::collections::HashSet<_> = [m1.content, m2.content].into_iter().collect();
        assert!(contents.contains("msg-a"));
        assert!(contents.contains("msg-b"));

        ch.disconnect().await.expect("disconnect");
    }

    // ── event publisher tests ────────────────────────────────────────────────

    use std::sync::Mutex as StdMutex;
    use crate::traits::{ChannelEvent, ChannelEventPublisher};

    struct CapturingPublisher {
        events: Arc<StdMutex<Vec<ChannelEvent>>>,
    }

    impl CapturingPublisher {
        fn new() -> (Arc<dyn ChannelEventPublisher>, Arc<StdMutex<Vec<ChannelEvent>>>) {
            let events = Arc::new(StdMutex::new(Vec::new()));
            let publisher = Arc::new(Self { events: Arc::clone(&events) });
            (publisher, events)
        }
    }

    #[async_trait::async_trait]
    impl ChannelEventPublisher for CapturingPublisher {
        async fn publish(&self, event: ChannelEvent) -> Result<(), ChannelError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_webhook_recv_publishes_message_received() {
        let (publisher, captured) = CapturingPublisher::new();
        let ch = WebhookChannel::new(
            ChannelId::new("wh-ep"),
            "127.0.0.1:0".parse().unwrap(),
            None,
        )
        .with_event_publisher("agent-99", publisher);

        ch.connect().await.expect("connect");

        // Send an inbound message directly to the HTTP endpoint.
        let actual_addr = ch.local_addr().await.expect("local_addr");
        let msg = ChannelMessage {
            id: "ev-test".to_string(),
            channel_id: ChannelId::new("wh-ep"),
            direction: crate::types::MessageDirection::Inbound,
            platform: Platform::Webhook,
            content: "event test".to_string(),
            sender_id: None,
            thread_id: None,
            metadata: serde_json::Value::Null,
            timestamp_ms: 0,
        };
        reqwest::Client::new()
            .post(format!("http://{actual_addr}/webhook"))
            .json(&msg)
            .send()
            .await
            .expect("POST ok");

        let received = tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            ch.recv(),
        )
        .await
        .expect("recv timeout")
        .expect("recv ok");

        assert_eq!(received.content, "event test");

        // recv() should have published exactly one MessageReceived event
        // (plus the ConnectionState from connect()).
        let events = captured.lock().unwrap();
        let recv_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, ChannelEvent::MessageReceived { .. }))
            .collect();
        assert_eq!(recv_events.len(), 1, "expected one MessageReceived event");
        match &recv_events[0] {
            ChannelEvent::MessageReceived {
                agent_id,
                channel,
                platform,
                content_preview,
            } => {
                assert_eq!(agent_id, "agent-99");
                assert_eq!(channel, "wh-ep");
                assert_eq!(*platform, Platform::Webhook);
                assert_eq!(content_preview, "event test");
            }
            _ => unreachable!(),
        }

        ch.disconnect().await.expect("disconnect");
    }

    // ── G-13 dedup bridge tests ───────────────────────────────────────────────

    /// G-13: When X-Request-Id is present the webhook handler must propagate
    /// that header value into msg.id.  This ensures DeduplicatingRouter shares
    /// the same canonical key as the HTTP-layer RequestDeduplicator.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_x_request_id_bridged_to_msg_id() {
        let ch = WebhookChannel::new(
            ChannelId::new("wh-bridge"),
            "127.0.0.1:0".parse().unwrap(),
            None,
        );
        ch.connect().await.expect("connect");
        let addr = ch.local_addr().await.expect("local_addr");
        let url = format!("http://{addr}/webhook");

        let msg = ChannelMessage {
            id: "original-json-id".to_string(),
            channel_id: ChannelId::new("wh-bridge"),
            direction: MessageDirection::Inbound,
            platform: Platform::Webhook,
            content: "bridge test".to_string(),
            sender_id: None,
            thread_id: None,
            metadata: serde_json::Value::Null,
            timestamp_ms: 0,
        };

        let client = reqwest::Client::new();
        client
            .post(&url)
            .header("x-request-id", "canonical-id-xyz")
            .json(&msg)
            .send()
            .await
            .unwrap();

        let received = tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            ch.recv(),
        )
        .await
        .expect("recv timeout")
        .expect("recv ok");

        // msg.id must be overwritten with the HTTP header value so that both
        // dedup layers (HTTP + router) operate on the same key.
        assert_eq!(
            received.id, "canonical-id-xyz",
            "msg.id should reflect X-Request-Id header (dedup bridge)"
        );
        assert_eq!(received.content, "bridge test");

        ch.disconnect().await.expect("disconnect");
    }

    /// G-13: When no X-Request-Id / X-Idempotency-Key is present the handler
    /// must preserve the sender's own msg.id (no overwrite).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_msg_id_preserved_when_no_header() {
        let ch = WebhookChannel::new(
            ChannelId::new("wh-nohdr"),
            "127.0.0.1:0".parse().unwrap(),
            None,
        );
        ch.connect().await.expect("connect");
        let addr = ch.local_addr().await.expect("local_addr");
        let url = format!("http://{addr}/webhook");

        let msg = ChannelMessage {
            id: "sender-chosen-id-123".to_string(),
            channel_id: ChannelId::new("wh-nohdr"),
            direction: MessageDirection::Inbound,
            platform: Platform::Webhook,
            content: "no header test".to_string(),
            sender_id: None,
            thread_id: None,
            metadata: serde_json::Value::Null,
            timestamp_ms: 0,
        };

        reqwest::Client::new()
            .post(&url)
            .json(&msg)  // no X-Request-Id header
            .send()
            .await
            .unwrap();

        let received = tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            ch.recv(),
        )
        .await
        .expect("recv timeout")
        .expect("recv ok");

        assert_eq!(
            received.id, "sender-chosen-id-123",
            "msg.id should be untouched when no HTTP dedup header is present"
        );

        ch.disconnect().await.expect("disconnect");
    }
}
