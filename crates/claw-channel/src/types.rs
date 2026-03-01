use serde::{Deserialize, Serialize};

/// Unique channel identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub String);

impl ChannelId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Direction of a channel message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageDirection {
    Inbound,
    Outbound,
}

/// Supported external platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Discord,
    Webhook,
    Stdin, // for testing / CLI
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Platform::Discord => "discord",
            Platform::Webhook => "webhook",
            Platform::Stdin => "stdin",
        };
        f.write_str(s)
    }
}

/// A message exchanged over a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub id: String,
    pub channel_id: ChannelId,
    pub direction: MessageDirection,
    pub platform: Platform,
    /// Plain-text content.
    pub content: String,
    /// Optional structured metadata (author, guild, etc.).
    pub metadata: serde_json::Value,
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: u64,
}

impl ChannelMessage {
    /// Create a new inbound message.
    pub fn inbound(channel_id: ChannelId, platform: Platform, content: impl Into<String>) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            id: format!("msg-{ts}"),
            channel_id,
            direction: MessageDirection::Inbound,
            platform,
            content: content.into(),
            metadata: serde_json::Value::Null,
            timestamp_ms: ts,
        }
    }

    /// Create a new outbound message.
    pub fn outbound(channel_id: ChannelId, platform: Platform, content: impl Into<String>) -> Self {
        let mut msg = Self::inbound(channel_id, platform, content);
        msg.direction = MessageDirection::Outbound;
        msg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_id_display() {
        assert_eq!(ChannelId::new("ch-1").to_string(), "ch-1");
    }

    #[test]
    fn test_platform_display() {
        assert_eq!(Platform::Discord.to_string(), "discord");
    }

    #[test]
    fn test_inbound_message() {
        let msg = ChannelMessage::inbound(ChannelId::new("c1"), Platform::Stdin, "hello");
        assert_eq!(msg.direction, MessageDirection::Inbound);
        assert_eq!(msg.content, "hello");
        assert!(msg.timestamp_ms > 0);
    }

    #[test]
    fn test_outbound_message() {
        let msg = ChannelMessage::outbound(ChannelId::new("c1"), Platform::Webhook, "reply");
        assert_eq!(msg.direction, MessageDirection::Outbound);
    }

    #[test]
    fn test_channel_message_serde() {
        let msg = ChannelMessage::inbound(ChannelId::new("ch"), Platform::Discord, "test");
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChannelMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "test");
    }
}
