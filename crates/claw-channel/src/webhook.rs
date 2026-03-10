//! WebhookChannel — HTTP webhook 收发适配器。
//!
//! 启动一个 axum HTTP 服务器，在 `POST /webhook` 接收入站消息；
//! 通过 reqwest 将出站消息 POST 到配置的目标 URL。

use std::{
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
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
    traits::Channel,
    types::{ChannelId, ChannelMessage},
};

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
}

impl WebhookChannel {
    /// 创建新的 WebhookChannel。
    ///
    /// - `id`：通道唯一标识。
    /// - `bind_addr`：axum 服务器监听地址（可使用端口 0 让系统自动分配）。
    /// - `outbound_url`：出站消息目标 URL；为 `None` 时调用 `send()` 会返回错误。
    pub fn new(id: ChannelId, bind_addr: SocketAddr, outbound_url: Option<String>) -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self {
            id,
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
        let router = Router::new().route(
            "/webhook",
            post(move |headers: HeaderMap, body: Bytes| {
                let tx = tx.clone();
                let secret = secret.clone();
                async move {
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
                    // Parse JSON and enqueue message.
                    match serde_json::from_slice::<ChannelMessage>(&body) {
                        Ok(msg) => {
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

        match result {
            Ok(()) => Ok(()),
            Err(SendErr::Transient(msg)) => Err(ChannelError::SendFailed(msg)),
            Err(SendErr::Permanent(e)) => Err(e),
        }
    }

    /// 从内部队列取出下一条入站消息（阻塞直到消息到达）。
    async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
        self.inbound_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| ChannelError::ReceiveFailed("channel closed".to_string()))
    }

    /// 终止后台 axum 服务器任务。
    async fn disconnect(&self) -> Result<(), ChannelError> {
        if let Some(handle) = self.server_handle.lock().await.take() {
            handle.abort();
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

    #[tokio::test]
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
        use std::sync::Arc;
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
        use std::sync::Arc;
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
}
