//! WebhookChannel — HTTP webhook 收发适配器。
//!
//! 启动一个 axum HTTP 服务器，在 `POST /webhook` 接收入站消息；
//! 通过 reqwest 将出站消息 POST 到配置的目标 URL。

use std::net::SocketAddr;

use async_trait::async_trait;
use axum::{routing::post, Json, Router};
use tokio::sync::{mpsc, Mutex};

use crate::{
    error::ChannelError,
    traits::Channel,
    types::{ChannelId, ChannelMessage},
};

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
    client: reqwest::Client,
    server_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// 实际监听地址（connect() 之后设置，用于端口 0 场景）。
    local_addr: Mutex<Option<SocketAddr>>,
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
            client: reqwest::Client::new(),
            server_handle: Mutex::new(None),
            local_addr: Mutex::new(None),
        }
    }

    /// 返回服务器实际监听地址（仅在 `connect()` 之后有效）。
    pub async fn local_addr(&self) -> Option<SocketAddr> {
        *self.local_addr.lock().await
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

        let router = Router::new().route(
            "/webhook",
            post(move |Json(msg): Json<ChannelMessage>| {
                let tx = tx.clone();
                async move {
                    // 忽略满队列错误（接收方关闭时服务器也将随之关闭）
                    let _ = tx.send(msg).await;
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
