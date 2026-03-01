use std::sync::Arc;

use crate::{agent_types::A2AMessage, error::RuntimeError, event_bus::EventBus};

// ─── IpcRouter ────────────────────────────────────────────────────────────────

/// Routes IPC byte frames to/from the `EventBus`.
///
/// Provides encode / decode helpers for `A2AMessage` (JSON-framed).
/// The actual socket-level transport is handled externally; `IpcRouter`
/// deliberately avoids coupling to `InterprocessTransport` so that different
/// transports can be plugged in at the `Runtime` layer.
pub struct IpcRouter {
    event_bus: Arc<EventBus>,
    endpoint: String,
}

impl IpcRouter {
    /// Create a new `IpcRouter` attached to `event_bus` and listening on
    /// `endpoint`.
    pub fn new(event_bus: Arc<EventBus>, endpoint: impl Into<String>) -> Self {
        Self {
            event_bus,
            endpoint: endpoint.into(),
        }
    }

    /// Serialize an `A2AMessage` to JSON bytes suitable for an IPC frame
    /// payload.
    pub fn encode_message(msg: &A2AMessage) -> Result<Vec<u8>, RuntimeError> {
        serde_json::to_vec(msg).map_err(|e| RuntimeError::IpcError(format!("encode failed: {}", e)))
    }

    /// Deserialize an IPC frame payload back into an `A2AMessage`.
    pub fn decode_message(bytes: &[u8]) -> Result<A2AMessage, RuntimeError> {
        serde_json::from_slice(bytes)
            .map_err(|e| RuntimeError::IpcError(format!("decode failed: {}", e)))
    }

    /// Return the IPC endpoint this router is associated with.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Return a reference to the shared `EventBus`.
    pub fn event_bus(&self) -> &Arc<EventBus> {
        &self.event_bus
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_types::AgentId;

    fn make_msg() -> A2AMessage {
        A2AMessage {
            from: AgentId::new("sender"),
            to: AgentId::new("receiver"),
            correlation_id: "corr-001".to_string(),
            payload: serde_json::json!({ "action": "ping", "seq": 42 }),
        }
    }

    // ── test_a2a_message_encode_decode_roundtrip ─────────────────────────────
    #[test]
    fn test_a2a_message_encode_decode_roundtrip() {
        let original = make_msg();
        let bytes = IpcRouter::encode_message(&original).expect("encode should succeed");
        assert!(!bytes.is_empty());

        let decoded = IpcRouter::decode_message(&bytes).expect("decode should succeed");

        assert_eq!(decoded.from, original.from);
        assert_eq!(decoded.to, original.to);
        assert_eq!(decoded.correlation_id, original.correlation_id);
        assert_eq!(decoded.payload, original.payload);
    }

    // ── test_ipc_router_endpoint ─────────────────────────────────────────────
    #[test]
    fn test_ipc_router_endpoint() {
        let bus = Arc::new(EventBus::new());
        let router = IpcRouter::new(Arc::clone(&bus), "/tmp/claw-test.sock");
        assert_eq!(router.endpoint(), "/tmp/claw-test.sock");
    }

    // ── test_ipc_router_decode_invalid_bytes ─────────────────────────────────
    #[test]
    fn test_ipc_router_decode_invalid_bytes() {
        let result = IpcRouter::decode_message(b"not valid json {{{");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("IPC error"),
            "expected IPC error, got: {}",
            msg
        );
    }

    // ── test_ipc_router_encode_empty_payload ─────────────────────────────────
    #[test]
    fn test_ipc_router_encode_empty_payload() {
        let msg = A2AMessage {
            from: AgentId::new("a"),
            to: AgentId::new("b"),
            correlation_id: "c".to_string(),
            payload: serde_json::Value::Null,
        };
        let bytes = IpcRouter::encode_message(&msg).unwrap();
        let decoded = IpcRouter::decode_message(&bytes).unwrap();
        assert_eq!(decoded.payload, serde_json::Value::Null);
    }
}
