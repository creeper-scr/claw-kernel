//! WebSocket bidirectional channel.
//!
//! Listens for incoming WebSocket connections and allows sending messages
//! to all connected clients simultaneously (fan-out).
//!
//! # Architecture
//!
//! - Each WebSocket client that connects is assigned a unique [`ConnectionId`].
//! - Incoming text frames from any client are forwarded to the shared inbound
//!   queue and can be retrieved via [`Channel::recv`].
//! - [`Channel::send`] serialises the [`ChannelMessage`] as JSON and broadcasts
//!   it to every currently-connected client. Disconnected clients are silently
//!   pruned from the connection map.
//!
//! # Usage
//!
//! ```rust,ignore
//! use claw_channels::WebSocketChannel;
//! use claw_channel::{Channel, ChannelId};
//! use std::sync::Arc;
//!
//! let ch = Arc::new(WebSocketChannel::new(ChannelId::new("ws-main")));
//! ch.connect().await?;
//!
//! // In an axum/warp handler you can call:
//! //   ch.register_connection(id, sender).await;
//! //   ch.push_inbound(msg).await;
//! ```

use async_trait::async_trait;
use claw_channel::{Channel, ChannelError, ChannelId, ChannelMessage};
use dashmap::DashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tokio::sync::{mpsc, Mutex};

/// Numeric handle that uniquely identifies one WebSocket client connection.
pub type ConnectionId = u64;

/// A bidirectional WebSocket channel that manages multiple concurrent connections.
///
/// The struct itself is cheap to clone (all interior state is reference-counted).
/// Pass it to WebSocket upgrade handlers via `Arc<WebSocketChannel>`.
pub struct WebSocketChannel {
    /// Logical channel identifier used within the claw system.
    id: ChannelId,
    /// Active connections: connection id → per-client outbound sender.
    connections: Arc<DashMap<ConnectionId, mpsc::Sender<String>>>,
    /// Monotonically increasing counter for connection IDs.
    next_id: Arc<AtomicU64>,
    /// Sender for inbound messages pushed by WebSocket handler tasks.
    incoming_tx: mpsc::Sender<ChannelMessage>,
    /// Receiver for inbound messages, consumed by `recv()`.
    incoming_rx: Arc<Mutex<mpsc::Receiver<ChannelMessage>>>,
    /// Whether the channel is in the "connected" (accepting) state.
    connected: Arc<AtomicBool>,
}

impl WebSocketChannel {
    /// Create a new [`WebSocketChannel`].
    ///
    /// The channel is initially disconnected; call [`connect`][Self::connect]
    /// to transition it to the accepting state.
    pub fn new(id: ChannelId) -> Self {
        let (incoming_tx, incoming_rx) = mpsc::channel(256);
        Self {
            id,
            connections: Arc::new(DashMap::new()),
            next_id: Arc::new(AtomicU64::new(1)),
            incoming_tx,
            incoming_rx: Arc::new(Mutex::new(incoming_rx)),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Allocate a new unique connection ID.
    ///
    /// Call this in your WebSocket upgrade handler before calling
    /// [`register_connection`][Self::register_connection].
    pub fn next_connection_id(&self) -> ConnectionId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Register a newly-connected client.
    ///
    /// `sender` is the mpsc sender whose receiver is drained by a per-client
    /// write task that forwards strings over the WebSocket.
    pub async fn register_connection(&self, id: ConnectionId, sender: mpsc::Sender<String>) {
        self.connections.insert(id, sender);
        tracing::debug!(connection_id = id, channel_id = %self.id, "WebSocket client registered");
    }

    /// Unregister a disconnected client.
    pub fn unregister_connection(&self, id: ConnectionId) {
        self.connections.remove(&id);
        tracing::debug!(connection_id = id, channel_id = %self.id, "WebSocket client unregistered");
    }

    /// Push an inbound message received from a WebSocket client into the queue.
    ///
    /// Call this from the per-client read task when a text frame arrives.
    /// Returns `Err` if the inbound queue has been dropped (channel shut down).
    pub async fn push_inbound(&self, msg: ChannelMessage) -> Result<(), ChannelError> {
        self.incoming_tx
            .send(msg)
            .await
            .map_err(|e| ChannelError::ReceiveFailed(format!("inbound queue send error: {e}")))
    }

    /// Get a clone of the shared connections map.
    ///
    /// Useful for WebSocket upgrade handlers that need to send messages
    /// directly to individual clients.
    pub fn connections(&self) -> Arc<DashMap<ConnectionId, mpsc::Sender<String>>> {
        Arc::clone(&self.connections)
    }

    /// Get a clone of the inbound message sender.
    ///
    /// WebSocket read tasks use this sender to push received frames into the
    /// shared inbound queue without holding a reference to the full channel.
    pub fn incoming_sender(&self) -> mpsc::Sender<ChannelMessage> {
        self.incoming_tx.clone()
    }
}

#[async_trait]
impl Channel for WebSocketChannel {
    fn platform(&self) -> &str {
        "websocket"
    }

    fn channel_id(&self) -> &ChannelId {
        &self.id
    }

    /// Transition the channel to the "connected" (accepting) state.
    ///
    /// This is a lightweight operation — the actual TCP listener must be
    /// created externally (e.g., via axum). `connect()` only sets the
    /// internal flag so that `send()` and `recv()` are permitted.
    ///
    /// Calling `connect()` when already connected is a no-op.
    async fn connect(&self) -> Result<(), ChannelError> {
        if self.connected.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!(channel_id = %self.id, "WebSocketChannel connected");
        Ok(())
    }

    /// Transition the channel to the disconnected state and drop all connections.
    ///
    /// All registered client senders are removed, which will cause their
    /// corresponding write tasks to observe a closed channel and exit cleanly.
    async fn disconnect(&self) -> Result<(), ChannelError> {
        self.connected.store(false, Ordering::SeqCst);
        let count = self.connections.len();
        self.connections.clear();
        tracing::info!(
            channel_id = %self.id,
            closed_connections = count,
            "WebSocketChannel disconnected"
        );
        Ok(())
    }

    /// Broadcast a message to all connected WebSocket clients.
    ///
    /// The [`ChannelMessage`] is serialised to JSON before sending. Clients
    /// whose send queues have been dropped (i.e., disconnected) are silently
    /// pruned from the connection map.
    ///
    /// Returns `Ok(())` even if there are no connected clients.
    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(ChannelError::ConnectionFailed(
                "channel is not connected".to_string(),
            ));
        }

        let text = serde_json::to_string(&message)
            .map_err(|e| ChannelError::SendFailed(format!("serialization error: {e}")))?;

        let mut failed: Vec<ConnectionId> = Vec::new();

        for entry in self.connections.iter() {
            if entry.value().send(text.clone()).await.is_err() {
                failed.push(*entry.key());
            }
        }

        // Prune disconnected clients discovered during broadcast.
        if !failed.is_empty() {
            for id in &failed {
                self.connections.remove(id);
            }
            tracing::debug!(
                channel_id = %self.id,
                pruned = failed.len(),
                "pruned disconnected WebSocket clients after send"
            );
        }

        Ok(())
    }

    /// Receive the next inbound message from any connected client.
    ///
    /// Blocks until a message is available or the inbound queue is closed.
    async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
        self.incoming_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| ChannelError::ReceiveFailed("inbound queue closed".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_channel::types::{MessageDirection, Platform};

    fn make_channel() -> WebSocketChannel {
        WebSocketChannel::new(ChannelId::new("ws-test"))
    }

    #[test]
    fn test_websocket_channel_platform() {
        let ch = make_channel();
        assert_eq!(ch.platform(), "websocket");
    }

    #[test]
    fn test_websocket_channel_id() {
        let ch = make_channel();
        assert_eq!(ch.channel_id().as_str(), "ws-test");
    }

    #[tokio::test]
    async fn test_connect_is_idempotent() {
        let ch = make_channel();
        ch.connect().await.unwrap();
        ch.connect().await.unwrap(); // second call must not error
        assert!(ch.connected.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_disconnect_clears_connections() {
        let ch = make_channel();
        ch.connect().await.unwrap();

        let (tx, _rx) = mpsc::channel(4);
        let id = ch.next_connection_id();
        ch.register_connection(id, tx).await;
        assert_eq!(ch.connections.len(), 1);

        ch.disconnect().await.unwrap();
        assert_eq!(ch.connections.len(), 0);
        assert!(!ch.connected.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_send_before_connect_returns_error() {
        let ch = make_channel();
        let msg = ChannelMessage::outbound(ChannelId::new("ws-test"), Platform::Stdin, "hi");
        let err = ch.send(msg).await.unwrap_err();
        assert!(
            err.to_string().contains("not connected"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn test_push_inbound_and_recv() {
        let ch = make_channel();
        ch.connect().await.unwrap();

        let msg = ChannelMessage::inbound(ChannelId::new("ws-test"), Platform::Stdin, "ping");
        ch.push_inbound(msg).await.unwrap();

        let received = ch.recv().await.unwrap();
        assert_eq!(received.content, "ping");
        assert_eq!(received.direction, MessageDirection::Inbound);
    }

    #[tokio::test]
    async fn test_send_broadcasts_to_connected_clients() {
        let ch = make_channel();
        ch.connect().await.unwrap();

        let (tx1, mut rx1) = mpsc::channel::<String>(4);
        let (tx2, mut rx2) = mpsc::channel::<String>(4);
        let id1 = ch.next_connection_id();
        let id2 = ch.next_connection_id();
        ch.register_connection(id1, tx1).await;
        ch.register_connection(id2, tx2).await;

        let msg = ChannelMessage::outbound(ChannelId::new("ws-test"), Platform::Stdin, "broadcast");
        ch.send(msg).await.unwrap();

        let raw1 = rx1.recv().await.unwrap();
        let raw2 = rx2.recv().await.unwrap();
        assert!(raw1.contains("broadcast"), "client 1 got: {raw1}");
        assert!(raw2.contains("broadcast"), "client 2 got: {raw2}");
    }

    #[tokio::test]
    async fn test_send_prunes_disconnected_clients() {
        let ch = make_channel();
        ch.connect().await.unwrap();

        let (tx, rx) = mpsc::channel::<String>(1);
        let id = ch.next_connection_id();
        ch.register_connection(id, tx).await;
        // Drop receiver to simulate client disconnect.
        drop(rx);

        let msg = ChannelMessage::outbound(ChannelId::new("ws-test"), Platform::Stdin, "test");
        // Send must succeed even though the client is gone.
        ch.send(msg).await.unwrap();
        // The dead connection must have been pruned.
        assert_eq!(ch.connections.len(), 0);
    }

    #[tokio::test]
    async fn test_next_connection_id_is_unique() {
        let ch = make_channel();
        let id1 = ch.next_connection_id();
        let id2 = ch.next_connection_id();
        assert_ne!(id1, id2);
    }
}
