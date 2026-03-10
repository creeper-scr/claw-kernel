//! Discord channel adapter — wraps twilight-gateway (receive) and
//! twilight-http (send) to implement the [`Channel`] trait.
//!
//! # Compatibility Notice
//!
//! This module is maintained for backwards compatibility.
//! New users should prefer the `claw-channels` crate with the `discord` feature,
//! which provides the same implementation under a dedicated crate:
//!
//! ```toml
//! claw-channels = { version = "1", features = ["discord"] }
//! ```

use crate::{
    error::ChannelError,
    traits::{Channel, ChannelEvent, ChannelEventPublisher},
    types::{ChannelId, ChannelMessage, Platform},
};
use async_trait::async_trait;
use claw_pal::retry::{with_retry_mapped, RetryConfig};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{mpsc, Mutex};
use twilight_gateway::{Config, Event, EventTypeFlags, Intents, Shard, ShardId};
use twilight_http::Client as HttpClient;
use twilight_model::id::{marker::ChannelMarker, Id};

/// Discord channel adapter.
///
/// Connects to a Discord guild channel using the bot token, receives
/// `MESSAGE_CREATE` events via a twilight shard, and sends messages via
/// the twilight HTTP client.
pub struct DiscordChannel {
    /// Logical channel identifier used within the claw system.
    id: ChannelId,
    /// Agent identifier forwarded in published channel events.
    agent_id: String,
    /// Discord bot token.
    token: String,
    /// Numeric Discord channel ID.
    discord_channel_id: u64,
    /// Reusable HTTP client shared across all `send()` calls.
    http_client: Arc<HttpClient>,
    /// Sender side of the inbound message queue.
    inbound_tx: mpsc::Sender<ChannelMessage>,
    /// Receiver side of the inbound message queue (consumed by `recv()`).
    inbound_rx: Mutex<mpsc::Receiver<ChannelMessage>>,
    /// Background shard task handle, set after `connect()`.
    shard_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Whether the channel is currently connected.
    connected: Arc<AtomicBool>,
    /// Retry policy applied to outbound `send()` calls.
    retry_config: RetryConfig,
    /// Optional EventBus publisher — wires the channel into the runtime event system.
    event_publisher: Option<Arc<dyn ChannelEventPublisher>>,
}

impl DiscordChannel {
    /// Create a new [`DiscordChannel`].
    ///
    /// Does not open any network connections; call [`connect`][Self::connect]
    /// to start the shard.
    pub fn new(id: ChannelId, token: impl Into<String>, discord_channel_id: u64) -> Self {
        let (tx, rx) = mpsc::channel(128);
        let token = token.into();
        let http_client = Arc::new(HttpClient::new(token.clone()));
        Self {
            id,
            agent_id: String::new(),
            token,
            discord_channel_id,
            http_client,
            inbound_tx: tx,
            inbound_rx: Mutex::new(rx),
            shard_handle: Mutex::new(None),
            connected: Arc::new(AtomicBool::new(false)),
            retry_config: RetryConfig::default(),
            event_publisher: None,
        }
    }

    /// Return the typed Discord channel ID used by twilight.
    fn discord_id(&self) -> Id<ChannelMarker> {
        Id::new(self.discord_channel_id)
    }

    /// Inject a message directly into the inbound queue.
    ///
    /// Only available in test builds; allows unit tests to verify `recv()`
    /// without establishing a real Discord connection.
    #[cfg(test)]
    pub fn inject_for_test(&self, msg: ChannelMessage) {
        self.inbound_tx
            .try_send(msg)
            .expect("test: inbound channel full or closed");
    }

    /// Override the retry policy applied to outbound `send()` calls.
    ///
    /// The default policy retries up to 3 times with 500 ms base delay capped
    /// at 30 s, covering Discord 429 rate-limit responses and transient 5xx
    /// errors.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Attach a [`ChannelEventPublisher`] to wire this channel into the
    /// runtime EventBus.
    ///
    /// Once set, `send()` publishes [`ChannelEvent::MessageSent`] and
    /// `recv()` publishes [`ChannelEvent::MessageReceived`] on every call.
    /// `connect()` and `disconnect()` publish [`ChannelEvent::ConnectionState`].
    /// All publish calls are best-effort; failures do not affect the primary
    /// send/recv result.
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
impl Channel for DiscordChannel {
    fn platform(&self) -> &str {
        "discord"
    }

    fn channel_id(&self) -> &ChannelId {
        &self.id
    }

    /// Connect to the Discord gateway.
    ///
    /// Spawns a background task that receives `MESSAGE_CREATE` events and
    /// forwards matching messages to the inbound queue.  Calling `connect()`
    /// again while already connected is a no-op.
    async fn connect(&self) -> Result<(), ChannelError> {
        if self.connected.load(Ordering::SeqCst) {
            return Ok(());
        }

        let token = self.token.clone();
        let discord_channel_id = self.discord_channel_id;
        let tx = self.inbound_tx.clone();
        let channel_id = self.id.clone();
        let connected = Arc::clone(&self.connected);

        // Build a shard config that filters events to MESSAGE_CREATE only.
        // This avoids deserializing unneeded payloads.
        let intents = Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT;
        let event_types = EventTypeFlags::MESSAGE_CREATE;
        let config = Config::builder(token, intents)
            .event_types(event_types)
            .build();

        let mut shard = Shard::with_config(ShardId::ONE, config);

        self.connected.store(true, Ordering::SeqCst);

        let shard_task = tokio::spawn(async move {
            loop {
                // next_event() drives the WebSocket connection internally and
                // returns fully-deserialized gateway events matching the
                // EventTypeFlags set in Config.
                let event = match shard.next_event().await {
                    Ok(ev) => ev,
                    Err(_) => {
                        // Stop if disconnect() has been called, otherwise keep
                        // trying (reconnect is handled internally by twilight).
                        if !connected.load(Ordering::SeqCst) {
                            break;
                        }
                        continue;
                    }
                };

                if let Event::MessageCreate(msg) = event {
                    // Only forward messages from the configured channel.
                    if msg.channel_id.get() == discord_channel_id {
                        let mut channel_msg = ChannelMessage::inbound(
                            channel_id.clone(),
                            Platform::Discord,
                            msg.content.clone(),
                        );
                        channel_msg.sender_id = Some(msg.author.id.get().to_string());
                        if tx.send(channel_msg).await.is_err() {
                            // Receiver dropped — shut down.
                            break;
                        }
                    }
                }
            }
        });

        *self.shard_handle.lock().await = Some(shard_task);

        if let Some(pub_) = &self.event_publisher {
            let _ = pub_
                .publish(ChannelEvent::ConnectionState {
                    channel: self.id.to_string(),
                    platform: Platform::Discord,
                    connected: true,
                })
                .await;
        }
        Ok(())
    }

    /// Send a message to the Discord channel, retrying on transient errors.
    ///
    /// Returns `Err(ChannelError::SendFailed)` immediately (without retrying)
    /// if the message exceeds Discord's 2 000-character limit.  All other
    /// failures (HTTP 429 rate-limit, 5xx server errors, network faults) are
    /// retried with exponential back-off according to `retry_config`.
    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError> {
        const DISCORD_MAX_MSG_LEN: usize = 2000;
        if message.content.len() > DISCORD_MAX_MSG_LEN {
            let err = Err(ChannelError::SendFailed(format!(
                "message exceeds Discord 2000-character limit ({} chars)",
                message.content.len()
            )));
            if let Some(pub_) = &self.event_publisher {
                let _ = pub_
                    .publish(ChannelEvent::MessageSent {
                        agent_id: self.agent_id.clone(),
                        channel: self.id.to_string(),
                        platform: Platform::Discord,
                        success: false,
                    })
                    .await;
            }
            return err;
        }

        let client = Arc::clone(&self.http_client);
        let discord_id = self.discord_id();
        let content = message.content.clone();

        let result = with_retry_mapped(
            || {
                let client = Arc::clone(&client);
                let content = content.clone();
                async move {
                    client
                        .create_message(discord_id)
                        .content(&content)
                        .map_err(|e| ChannelError::SendFailed(e.to_string()))?
                        .await
                        .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
                    Ok(())
                }
            },
            &self.retry_config,
            // Retry on all HTTP / network errors; the 2000-char guard above
            // prevents the most common permanent 400 error before we get here.
            |_| true,
        )
        .await;

        if let Some(pub_) = &self.event_publisher {
            let _ = pub_
                .publish(ChannelEvent::MessageSent {
                    agent_id: self.agent_id.clone(),
                    channel: self.id.to_string(),
                    platform: Platform::Discord,
                    success: result.is_ok(),
                })
                .await;
        }
        result
    }

    /// Receive the next inbound message.
    ///
    /// Blocks until a message arrives or the inbound channel is closed.
    async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
        let result = self
            .inbound_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| ChannelError::ReceiveFailed("inbound channel closed".to_string()));

        if let (Ok(msg), Some(pub_)) = (&result, &self.event_publisher) {
            let _ = pub_
                .publish(ChannelEvent::MessageReceived {
                    agent_id: self.agent_id.clone(),
                    channel: self.id.to_string(),
                    platform: Platform::Discord,
                    content_preview: msg.content.chars().take(64).collect(),
                })
                .await;
        }
        result
    }

    /// Disconnect from the Discord gateway.
    async fn disconnect(&self) -> Result<(), ChannelError> {
        self.connected.store(false, Ordering::SeqCst);
        if let Some(handle) = self.shard_handle.lock().await.take() {
            handle.abort();
        }

        if let Some(pub_) = &self.event_publisher {
            let _ = pub_
                .publish(ChannelEvent::ConnectionState {
                    channel: self.id.to_string(),
                    platform: Platform::Discord,
                    connected: false,
                })
                .await;
        }
        Ok(())
    }
}

#[cfg(feature = "discord")]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MessageDirection, Platform};
    use std::sync::Mutex as StdMutex;

    fn make_channel() -> DiscordChannel {
        DiscordChannel::new(ChannelId::new("test-discord-ch"), "fake-token", 123_456_789)
    }

    // ── helper: a publisher that records all emitted events ─────────────────

    struct CapturingPublisher {
        events: Arc<StdMutex<Vec<ChannelEvent>>>,
    }

    impl CapturingPublisher {
        fn new() -> (Arc<dyn ChannelEventPublisher>, Arc<StdMutex<Vec<ChannelEvent>>>) {
            let events = Arc::new(StdMutex::new(Vec::new()));
            let publisher = Arc::new(Self {
                events: Arc::clone(&events),
            });
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

    // ── existing tests ───────────────────────────────────────────────────────

    #[test]
    fn test_discord_channel_new() {
        // Constructor must not panic.
        let _ch = make_channel();
    }

    #[test]
    fn test_discord_channel_platform() {
        let ch = make_channel();
        assert_eq!(ch.platform(), "discord");
    }

    #[test]
    fn test_discord_channel_channel_id() {
        let ch = make_channel();
        assert_eq!(ch.channel_id().as_str(), "test-discord-ch");
    }

    #[tokio::test]
    async fn test_discord_channel_inject_and_recv() {
        let ch = make_channel();

        let msg = ChannelMessage::inbound(
            ChannelId::new("test-discord-ch"),
            Platform::Discord,
            "hello from discord",
        );
        ch.inject_for_test(msg);

        let received = ch.recv().await.expect("should receive injected message");
        assert_eq!(received.content, "hello from discord");
        assert_eq!(received.direction, MessageDirection::Inbound);
        assert_eq!(received.platform, Platform::Discord);
    }

    #[tokio::test]
    async fn test_discord_send_rejects_overlong_message() {
        let ch = make_channel();
        // Construct a message that exceeds Discord's 2000-character limit.
        let long_content = "a".repeat(2001);
        let msg = ChannelMessage::outbound(
            ChannelId::new("test-discord-ch"),
            Platform::Discord,
            long_content,
        );
        let err = ch.send(msg).await.unwrap_err();
        assert!(
            err.to_string().contains("2000-character limit"),
            "expected 2000-character limit error, got: {err}"
        );
    }

    // ── event publisher tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_recv_publishes_message_received_event() {
        let (publisher, captured) = CapturingPublisher::new();
        let ch = DiscordChannel::new(ChannelId::new("dc-ep"), "fake-token", 999)
            .with_event_publisher("agent-dc", publisher);

        ch.inject_for_test(ChannelMessage::inbound(
            ChannelId::new("dc-ep"),
            Platform::Discord,
            "discord hello",
        ));

        ch.recv().await.expect("recv ok");

        let events = captured.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ChannelEvent::MessageReceived {
                agent_id,
                channel,
                platform,
                content_preview,
            } => {
                assert_eq!(agent_id, "agent-dc");
                assert_eq!(channel, "dc-ep");
                assert_eq!(*platform, Platform::Discord);
                assert_eq!(content_preview, "discord hello");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_send_overlong_publishes_failed_event() {
        let (publisher, captured) = CapturingPublisher::new();
        let ch = DiscordChannel::new(ChannelId::new("dc-ep2"), "fake-token", 999)
            .with_event_publisher("agent-dc2", publisher);

        let long_content = "a".repeat(2001);
        let msg = ChannelMessage::outbound(
            ChannelId::new("dc-ep2"),
            Platform::Discord,
            long_content,
        );
        let _ = ch.send(msg).await;

        let events = captured.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ChannelEvent::MessageSent { success, .. } => assert!(!success),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_disconnect_publishes_connection_state() {
        let (publisher, captured) = CapturingPublisher::new();
        let ch = DiscordChannel::new(ChannelId::new("dc-ep3"), "fake-token", 999)
            .with_event_publisher("agent-dc3", publisher);

        ch.disconnect().await.expect("disconnect ok");

        let events = captured.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ChannelEvent::ConnectionState { connected, .. } => assert!(!connected),
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
