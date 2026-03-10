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
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
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

    /// 将消息 POST 到 `outbound_url`。未配置时返回 `SendFailed` 错误。
    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError> {
        let url = self
            .outbound_url
            .as_ref()
            .ok_or_else(|| ChannelError::SendFailed("no outbound_url configured".to_string()))?;
        self.client
            .post(url)
            .json(&message)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
        Ok(())
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
}
