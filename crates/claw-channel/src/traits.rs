use crate::{
    error::ChannelError,
    types::{ChannelId, ChannelMessage, Platform},
};
use async_trait::async_trait;
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

    struct MockChannel {
        id: ChannelId,
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
            Ok(ChannelMessage::inbound(
                self.id.clone(),
                crate::types::Platform::Stdin,
                "mock",
            ))
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
        let ch = MockChannel {
            id: ChannelId::new("m"),
        };
        let msg =
            ChannelMessage::outbound(ChannelId::new("m"), crate::types::Platform::Stdin, "hi");
        ch.send(msg).await.unwrap();
    }

    #[tokio::test]
    async fn test_channel_trait_connect_disconnect() {
        let ch = MockChannel {
            id: ChannelId::new("m"),
        };
        ch.connect().await.unwrap();
        ch.disconnect().await.unwrap();
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
