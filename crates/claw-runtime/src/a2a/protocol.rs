//! A2A (Agent-to-Agent) Protocol
//!
//! Defines the message format, message types, and related structures for
//! agent-to-agent communication.

use crate::agent_types::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Task specification for A2A messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Task ID.
    pub id: String,
    /// Task type.
    pub task_type: String,
    /// Task parameters.
    pub params: serde_json::Value,
}

/// Priority levels for A2A messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessagePriority {
    /// Critical priority - immediate processing.
    Critical = 0,
    /// High priority - process before normal.
    High = 1,
    /// Normal priority - standard processing.
    #[default]
    Normal = 2,
    /// Low priority - process when idle.
    Low = 3,
    /// Background priority - lowest priority.
    Background = 4,
}

impl MessagePriority {
    /// Convert priority to numeric value (lower = higher priority).
    pub fn as_u8(&self) -> u8 {
        *self as u8
    }

    /// Check if this is a direct (immediate) priority level.
    /// Critical and High priorities are considered direct.
    pub fn is_direct(&self) -> bool {
        matches!(self, MessagePriority::Critical | MessagePriority::High)
    }
}

/// Types of A2A messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum A2AMessageType {
    /// Request message - expects a response.
    Request,
    /// Response message - reply to a request.
    Response,
    /// Event message - one-way notification.
    Event,
    /// Discovery request - query for available capabilities.
    DiscoveryRequest,
    /// Discovery response - reply to discovery request.
    DiscoveryResponse,
    /// Heartbeat - keep-alive message.
    Heartbeat,
    /// Error message - indicates a processing error.
    Error,
}

/// Payload content for A2A messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum A2AMessagePayload {
    /// Request payload.
    Request {
        /// Action to perform.
        action: String,
        /// Additional request parameters.
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    /// Response payload.
    Response {
        /// Response status.
        status: ResponseStatus,
        /// Response data.
        result: serde_json::Value,
    },
    /// Event payload.
    Event {
        /// Event type.
        event_type: String,
        /// Event data.
        data: serde_json::Value,
    },
    /// Discovery request payload.
    DiscoveryRequest {
        /// Optional query string.
        query: Option<String>,
    },
    /// Discovery response payload.
    DiscoveryResponse {
        /// Available capabilities.
        capabilities: Vec<AgentCapability>,
        /// Additional metadata.
        metadata: Option<HashMap<String, String>>,
    },
    /// Heartbeat payload.
    Heartbeat {
        /// Agent status.
        status: AgentStatus,
    },
    /// Error payload.
    Error {
        /// Error code.
        code: String,
        /// Error message.
        message: String,
    },
}

/// Response status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    /// Success.
    Success,
    /// Partial success.
    Partial,
    /// Failure.
    Failure,
}

/// Agent status in heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Agent is active and processing.
    Active,
    /// Agent is idle.
    Idle,
    /// Agent is busy.
    Busy,
    /// Agent is shutting down.
    ShuttingDown,
}

/// A capability advertised by an agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentCapability {
    /// Capability name.
    pub name: String,
    /// Capability version.
    pub version: String,
    /// Optional description.
    pub description: Option<String>,
    /// Additional metadata.
    pub metadata: HashMap<String, String>,
}

impl AgentCapability {
    /// Create a new agent capability.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            description: None,
            metadata: HashMap::new(),
        }
    }

    /// Add a description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// An A2A message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct A2AMessage {
    /// Unique message ID.
    pub id: String,
    /// Source agent ID.
    pub source: AgentId,
    /// Target agent ID (None for broadcast).
    pub target: Option<AgentId>,
    /// Message type.
    pub message_type: A2AMessageType,
    /// Message payload.
    pub payload: A2AMessagePayload,
    /// Message priority.
    pub priority: MessagePriority,
    /// Correlation ID for request/response matching.
    pub correlation_id: Option<String>,
    /// Timestamp (Unix milliseconds).
    pub timestamp: u64,
    /// Time-to-live in seconds (None for no expiry).
    pub ttl_secs: Option<u32>,
}

impl A2AMessage {
    /// Create a new A2A message.
    pub fn new(
        id: impl Into<String>,
        source: AgentId,
        message_type: A2AMessageType,
        payload: A2AMessagePayload,
    ) -> Self {
        Self {
            id: id.into(),
            source,
            target: None,
            message_type,
            payload,
            priority: MessagePriority::Normal,
            correlation_id: None,
            timestamp: current_timestamp_ms(),
            ttl_secs: None,
        }
    }

    /// Set the target agent.
    pub fn with_target(mut self, target: AgentId) -> Self {
        self.target = Some(target);
        self
    }

    /// Set the priority.
    pub fn with_priority(mut self, priority: MessagePriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set the correlation ID.
    pub fn with_correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Set the TTL.
    pub fn with_ttl(mut self, ttl_secs: u32) -> Self {
        self.ttl_secs = Some(ttl_secs);
        self
    }

    /// Check if the message has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(ttl) = self.ttl_secs {
            let now = current_timestamp_ms();
            let age_ms = now.saturating_sub(self.timestamp);
            age_ms > (ttl as u64 * 1000)
        } else {
            false
        }
    }
}

/// Get current timestamp in milliseconds.
fn current_timestamp_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_priority_ordering() {
        assert!(MessagePriority::Critical.as_u8() < MessagePriority::Normal.as_u8());
        assert!(MessagePriority::Normal.as_u8() < MessagePriority::Background.as_u8());
    }

    #[test]
    fn test_a2a_message_builder() {
        let msg = A2AMessage::new(
            "msg-001",
            AgentId::new("sender"),
            A2AMessageType::Request,
            A2AMessagePayload::Request {
                action: "test".to_string(),
                extra: HashMap::new(),
            },
        )
        .with_target(AgentId::new("receiver"))
        .with_priority(MessagePriority::High)
        .with_correlation_id("corr-001");

        assert_eq!(msg.id, "msg-001");
        assert_eq!(msg.source.0, "sender");
        assert_eq!(msg.target.as_ref().unwrap().0, "receiver");
        assert_eq!(msg.priority, MessagePriority::High);
        assert_eq!(msg.correlation_id, Some("corr-001".to_string()));
    }

    #[test]
    fn test_agent_capability_builder() {
        let cap = AgentCapability::new("test-cap", "1.0.0")
            .with_description("A test capability")
            .with_metadata("key", "value");

        assert_eq!(cap.name, "test-cap");
        assert_eq!(cap.version, "1.0.0");
        assert_eq!(cap.description, Some("A test capability".to_string()));
        assert_eq!(cap.metadata.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn test_message_expiration() {
        let mut msg = A2AMessage::new(
            "expiring",
            AgentId::new("a"),
            A2AMessageType::Event,
            A2AMessagePayload::Event {
                event_type: "test".to_string(),
                data: serde_json::Value::Null,
            },
        )
        .with_ttl(1);

        // Set timestamp to past
        msg.timestamp = current_timestamp_ms() - 2000; // 2 seconds ago

        assert!(msg.is_expired());
    }
}
