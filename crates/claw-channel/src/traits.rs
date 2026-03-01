use async_trait::async_trait;
use crate::{error::ChannelError, types::{ChannelId, ChannelMessage}};

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

    /// Connect / authenticate with the external platform.
    async fn connect(&self) -> Result<(), ChannelError>;

    /// Gracefully disconnect.
    async fn disconnect(&self) -> Result<(), ChannelError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockChannel { id: ChannelId }

    #[async_trait::async_trait]
    impl Channel for MockChannel {
        fn platform(&self) -> &str { "mock" }
        fn channel_id(&self) -> &ChannelId { &self.id }
        async fn send(&self, _msg: ChannelMessage) -> Result<(), ChannelError> { Ok(()) }
        async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
            Ok(ChannelMessage::inbound(self.id.clone(), crate::types::Platform::Stdin, "mock"))
        }
        async fn connect(&self) -> Result<(), ChannelError> { Ok(()) }
        async fn disconnect(&self) -> Result<(), ChannelError> { Ok(()) }
    }

    #[tokio::test]
    async fn test_channel_trait_send() {
        let ch = MockChannel { id: ChannelId::new("m") };
        let msg = ChannelMessage::outbound(ChannelId::new("m"), crate::types::Platform::Stdin, "hi");
        ch.send(msg).await.unwrap();
    }

    #[tokio::test]
    async fn test_channel_trait_connect_disconnect() {
        let ch = MockChannel { id: ChannelId::new("m") };
        ch.connect().await.unwrap();
        ch.disconnect().await.unwrap();
    }
}
