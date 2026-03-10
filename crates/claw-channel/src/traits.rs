use crate::{
    error::ChannelError,
    types::{ChannelId, ChannelMessage, Platform},
};
use async_trait::async_trait;
use futures_util::stream::{self, Stream};
use std::sync::Arc;

/// Channel event published by [`ChannelEventPublisher::publish`].
#[derive(Debug, Clone)]
pub enum ChannelEvent {
    /// A message was received from an external platform.
    MessageReceived {
        agent_id: String,
        channel: String,
        platform: Platform,
        content_preview: String,
    },
    /// A message was sent to an external platform.
    MessageSent {
        agent_id: String,
        channel: String,
        platform: Platform,
        success: bool,
    },
    /// Channel connection state changed.
    ConnectionState {
        channel: String,
        platform: Platform,
        connected: bool,
    },
}

/// Core trait for channel adapters.
///
/// Implementors bridge external messaging platforms (Discord, webhooks, etc.)
/// to the agent system.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Platform name (e.g., "discord", "webhook").
    fn platform(&self) -> &str;

    /// Unique channel ID.
    fn channel_id(&self) -> &ChannelId;

    /// Send a message to this channel.
    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError>;

    /// Receive the next incoming message (blocking until one arrives).
    async fn recv(&self) -> Result<ChannelMessage, ChannelError>;

    /// Return an infinite [`Stream`] of inbound messages.
    ///
    /// Each item is a `ChannelMessage`; the stream ends when `recv()` returns
    /// an error (e.g. the channel is closed or disconnected), at which point
    /// the stream terminates silently.
    ///
    /// This default implementation wraps repeated `recv()` calls via
    /// [`futures_util::stream::unfold`].  Implementors may override this
    /// method to provide a more efficient stream if the underlying transport
    /// already produces one (e.g. a `tokio::sync::mpsc::Receiver`).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use futures_util::StreamExt;
    ///
    /// channel.connect().await?;
    /// let mut stream = channel.into_stream();
    /// while let Some(msg) = stream.next().await {
    ///     println!("received: {}", msg.content);
    /// }
    /// ```
    fn into_stream(&self) -> impl Stream<Item = ChannelMessage> + '_
    where
        Self: Sized,
    {
        stream::unfold(self, |ch| async move {
            match ch.recv().await {
                Ok(msg) => Some((msg, ch)),
                Err(_) => None,
            }
        })
    }

    /// Connect to the external platform and start receiving messages.
    ///
    /// # Semantics
    /// - **Idempotent**: calling `connect()` on an already-connected channel is a no-op.
    /// - **Reconnectable**: calling `connect()` after `disconnect()` re-establishes the connection.
    /// - Calling `send()` or `recv()` before `connect()` returns
    ///   `Err(ChannelError::NotConnected)`.
    async fn connect(&self) -> Result<(), ChannelError>;

    /// Disconnect from the platform.
    ///
    /// Messages already queued in the internal buffer can still be read via `recv()` after
    /// disconnection.  Calling `send()` after disconnection returns
    /// `Err(ChannelError::NotConnected)`.
    async fn disconnect(&self) -> Result<(), ChannelError>;
}

/// Create a [`Stream`] of inbound messages from any `Arc<dyn Channel>`.
///
/// This free function is the escape hatch for object-safe contexts (e.g. when
/// the channel is stored as `Arc<dyn Channel>`), where the `into_stream()`
/// method is not callable due to the `where Self: Sized` constraint.
///
/// # Example
///
/// ```rust,ignore
/// use futures_util::StreamExt;
/// use std::sync::Arc;
///
/// let ch: Arc<dyn Channel> = Arc::new(my_channel);
/// let mut stream = claw_channel::channel_into_stream(ch);
/// while let Some(msg) = stream.next().await {
///     println!("received: {}", msg.content);
/// }
/// ```
pub fn channel_into_stream(
    channel: Arc<dyn Channel>,
) -> impl Stream<Item = ChannelMessage> {
    stream::unfold(channel, |ch| async move {
        match ch.recv().await {
            Ok(msg) => Some((msg, ch)),
            Err(_) => None,
        }
    })
}

/// Event publisher for channel-related events.
///
/// This trait allows Layer 1 (claw-runtime) to inject EventBus capabilities
/// into Layer 2 (claw-channel) without creating a circular dependency.
///
/// Channel adapters use this trait to publish events when:
/// - A message is received from an external platform
/// - A message is sent to an external platform
/// - Connection state changes
///
/// # Example
///
/// ```rust,ignore
/// use claw_channel::{ChannelEventPublisher, ChannelEvent};
/// use claw_runtime::{EventBus, events::Event, agent_types::AgentId};
/// use std::sync::Arc;
///
/// struct RuntimeChannelEventPublisher {
///     event_bus: Arc<EventBus>,
/// }
///
/// #[async_trait::async_trait]
/// impl ChannelEventPublisher for RuntimeChannelEventPublisher {
///     async fn publish(&self, event: ChannelEvent) -> Result<(), claw_channel::ChannelError> {
///         // Forward to the runtime event bus
///         let _ = self.event_bus.publish(Event::ChannelEvent);
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait ChannelEventPublisher: Send + Sync {
    /// Publish a channel event (best-effort; failures must not affect the main flow).
    async fn publish(&self, event: ChannelEvent) -> Result<(), ChannelError>;
}

/// No-op event publisher for testing or when event publishing is not needed.
pub struct NoopChannelEventPublisher;

#[async_trait]
impl ChannelEventPublisher for NoopChannelEventPublisher {
    async fn publish(&self, _event: ChannelEvent) -> Result<(), ChannelError> {
        Ok(())
    }
}

impl NoopChannelEventPublisher {
    /// Create a new no-op event publisher wrapped in Arc.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Arc<dyn ChannelEventPublisher> {
        Arc::new(NoopChannelEventPublisher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockChannel {
        id: ChannelId,
        /// Number of messages to return before returning an error.
        limit: usize,
        count: AtomicUsize,
    }

    impl MockChannel {
        fn new(id: &str, limit: usize) -> Self {
            Self {
                id: ChannelId::new(id),
                limit,
                count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl Channel for MockChannel {
        fn platform(&self) -> &str {
            "mock"
        }
        fn channel_id(&self) -> &ChannelId {
            &self.id
        }
        async fn send(&self, _msg: ChannelMessage) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
            let n = self.count.fetch_add(1, Ordering::SeqCst);
            if n < self.limit {
                Ok(ChannelMessage::inbound(
                    self.id.clone(),
                    crate::types::Platform::Stdin,
                    "mock",
                ))
            } else {
                Err(ChannelError::ReceiveFailed("closed".to_string()))
            }
        }
        async fn connect(&self) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn disconnect(&self) -> Result<(), ChannelError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_channel_trait_send() {
        let ch = MockChannel::new("m", 1);
        let msg =
            ChannelMessage::outbound(ChannelId::new("m"), crate::types::Platform::Stdin, "hi");
        ch.send(msg).await.unwrap();
    }

    #[tokio::test]
    async fn test_channel_trait_connect_disconnect() {
        let ch = MockChannel::new("m", 0);
        ch.connect().await.unwrap();
        ch.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn test_into_stream_yields_messages_then_terminates() {
        let ch = MockChannel::new("stream-test", 3);
        let msgs: Vec<_> = ch.into_stream().collect().await;
        assert_eq!(msgs.len(), 3, "stream should yield exactly 3 messages");
        for msg in &msgs {
            assert_eq!(msg.content, "mock");
        }
    }

    #[tokio::test]
    async fn test_channel_into_stream_arc_dyn() {
        let ch: Arc<dyn Channel> = Arc::new(MockChannel::new("arc-stream", 2));
        let msgs: Vec<_> = channel_into_stream(ch).collect().await;
        assert_eq!(msgs.len(), 2, "arc stream should yield 2 messages");
    }

    #[tokio::test]
    async fn test_into_stream_empty_channel() {
        // A channel that immediately errors → stream should yield nothing.
        let ch = MockChannel::new("empty", 0);
        let msgs: Vec<_> = ch.into_stream().collect().await;
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn test_noop_channel_event_publisher() {
        let publisher = NoopChannelEventPublisher;
        publisher
            .publish(ChannelEvent::MessageReceived {
                agent_id: "agent-1".to_string(),
                channel: "ch-1".to_string(),
                platform: Platform::Stdin,
                content_preview: "hello".to_string(),
            })
            .await
            .unwrap();
        publisher
            .publish(ChannelEvent::MessageSent {
                agent_id: "agent-1".to_string(),
                channel: "ch-1".to_string(),
                platform: Platform::Stdin,
                success: true,
            })
            .await
            .unwrap();
        publisher
            .publish(ChannelEvent::ConnectionState {
                channel: "ch-1".to_string(),
                platform: Platform::Stdin,
                connected: true,
            })
            .await
            .unwrap();
    }
}
