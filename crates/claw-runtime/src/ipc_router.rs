//! IPC Router with A2A Protocol Support
//!
//! Routes A2A messages between agents, supporting both local (in-process)
//! and remote (IPC) message delivery.

use claw_pal::{InterprocessTransport, IpcTransport};

use crate::a2a::protocol::{A2AMessage, A2AMessagePayload, A2AMessageType};
use crate::a2a::routing::{AgentHandle, SimpleRouter};
use crate::agent_types::AgentId;
use crate::error::RuntimeError;
use crate::event_bus::EventBus;
use crate::events::Event;
use std::sync::Arc;
use tokio::sync::RwLock;

// ─── IpcRouter ────────────────────────────────────────────────────────────────

/// Routes A2A messages to local and remote agents.
///
/// The `IpcRouter` maintains a registry of local agents and routes
/// messages directly to target agents.
pub struct IpcRouter {
    /// The underlying simple router for message delivery.
    pub router: Arc<SimpleRouter>,
    /// The endpoint this router listens on.
    endpoint: String,
    /// Event bus for system-wide notifications.
    event_bus: Arc<EventBus>,
    /// Remote agent endpoints (agent_id -> endpoint).
    remote_endpoints: Arc<RwLock<std::collections::HashMap<AgentId, String>>>,
}

impl IpcRouter {
    /// Create a new `IpcRouter` with the given endpoint and event bus.
    pub fn new(event_bus: Arc<EventBus>, endpoint: impl Into<String>) -> Self {
        Self {
            router: Arc::new(SimpleRouter::new()),
            endpoint: endpoint.into(),
            event_bus,
            remote_endpoints: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Return the IPC endpoint this router is associated with.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Return a reference to the shared `EventBus`.
    pub fn event_bus(&self) -> &Arc<EventBus> {
        &self.event_bus
    }

    /// Return a reference to the router.
    pub fn router(&self) -> &Arc<SimpleRouter> {
        &self.router
    }

    /// Serialize an `A2AMessage` to JSON bytes.
    pub fn encode_message(msg: &A2AMessage) -> Result<Vec<u8>, RuntimeError> {
        serde_json::to_vec(msg).map_err(|e| RuntimeError::IpcError(format!("encode failed: {}", e)))
    }

    /// Deserialize an IPC frame payload back into an `A2AMessage`.
    pub fn decode_message(bytes: &[u8]) -> Result<A2AMessage, RuntimeError> {
        serde_json::from_slice(bytes)
            .map_err(|e| RuntimeError::IpcError(format!("decode failed: {}", e)))
    }

    /// Register a local agent with the router.
    ///
    /// Returns an `AgentHandle` that can be used to send messages to the agent.
    pub async fn register_agent(&self, agent_id: AgentId, buffer_size: usize) -> AgentHandle {
        self.router.register_agent(agent_id, buffer_size).await
    }

    /// Unregister a local agent from the router.
    pub async fn unregister_agent(&self, agent_id: &AgentId) -> Result<(), RuntimeError> {
        self.router.unregister_agent(agent_id).await
    }

    /// Register a remote agent endpoint.
    pub async fn register_remote_endpoint(&self, agent_id: AgentId, endpoint: impl Into<String>) {
        let mut remotes = self.remote_endpoints.write().await;
        remotes.insert(agent_id, endpoint.into());
    }

    /// Unregister a remote agent endpoint.
    pub async fn unregister_remote_endpoint(&self, agent_id: &AgentId) {
        let mut remotes = self.remote_endpoints.write().await;
        remotes.remove(agent_id);
    }

    /// Get the endpoint for a remote agent.
    pub async fn get_remote_endpoint(&self, agent_id: &AgentId) -> Option<String> {
        let remotes = self.remote_endpoints.read().await;
        remotes.get(agent_id).cloned()
    }

    /// Route a message to its target (local or remote).
    ///
    /// First checks if the target is a local agent, then falls back to remote.
    pub async fn route_message(&self, message: A2AMessage) -> Result<(), RuntimeError> {
        // Check if target is local
        if let Some(target) = &message.target {
            if self.router.is_agent_registered(target).await {
                return self.router.route_message(message).await;
            }

            // Check if target is a known remote agent — use PAL IPC transport.
            if let Some(remote_endpoint) = self.get_remote_endpoint(target).await {
                let bytes = Self::encode_message(&message)?;
                let transport = InterprocessTransport::new_client(&remote_endpoint)
                    .await
                    .map_err(|e| RuntimeError::IpcError(e.to_string()))?;
                transport
                    .send(&bytes)
                    .await
                    .map_err(|e| RuntimeError::IpcError(e.to_string()))?;
                return Ok(());
            }

            // Target not found
            return Err(RuntimeError::AgentNotFound(target.0.clone()));
        }

        // Broadcast - send to all local agents
        self.router.route_message(message).await
    }

    /// Send a message to a specific agent.
    pub async fn send(&self, target: &AgentId, message: A2AMessage) -> Result<(), RuntimeError> {
        self.router.send(target, message).await
    }

    /// Get list of all registered local agent IDs.
    pub async fn local_agent_ids(&self) -> Vec<AgentId> {
        self.router.get_agent_ids().await
    }

    /// Start accepting incoming IPC connections in a background task.
    ///
    /// Each accepted connection is handled in a dedicated `tokio::spawn` task
    /// that reads frames, decodes them as `A2AMessage`, and routes them to
    /// local agents.
    ///
    /// **Note:** Uses `InterprocessTransport::new_server` which binds, accepts
    /// one connection, then rebinds for the next. On Unix the stale socket file
    /// is cleaned up before each rebind. On Windows this returns immediately
    /// with an `IpcError` (Named Pipe support is planned for v0.2.0).
    pub async fn start_accepting(&self) -> Result<(), RuntimeError> {
        let endpoint = self.endpoint.clone();
        let router = Arc::clone(&self.router);
        let event_bus = Arc::clone(&self.event_bus);

        tokio::spawn(async move {
            loop {
                // Remove any stale socket file before rebinding (Unix only).
                #[cfg(unix)]
                let _ = std::fs::remove_file(&endpoint);

                let transport =
                    match InterprocessTransport::new_server(&endpoint).await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::error!(
                                "IpcRouter: failed to bind on {}: {}",
                                endpoint,
                                e
                            );
                            break;
                        }
                    };

                let router_clone = Arc::clone(&router);
                let event_bus_clone = Arc::clone(&event_bus);
                tokio::spawn(async move {
                    Self::handle_transport(transport, router_clone, event_bus_clone).await;
                });
            }
        });

        Ok(())
    }

    /// Drive a single accepted IPC connection until it closes.
    ///
    /// Reads length-prefixed frames, decodes each as an `A2AMessage`,
    /// publishes a `MessageReceived` event, and routes the message locally.
    async fn handle_transport(
        transport: InterprocessTransport,
        router: Arc<SimpleRouter>,
        event_bus: Arc<EventBus>,
    ) {
        while let Ok(bytes) = transport.recv().await {
            match Self::decode_message(&bytes) {
                Ok(message) => {
                    let _ = event_bus.publish(Event::MessageReceived {
                        agent_id: message.source.clone(),
                        channel: "ipc".to_string(),
                        message_type: format!("{:?}", message.message_type),
                    });
                    let _ = router.route_message(message).await;
                }
                Err(e) => tracing::warn!("IpcRouter: decode error: {e}"),
            }
        }
    }

    /// Handle a discovery request.
    pub async fn handle_discovery_request(
        &self,
        source: AgentId,
        query: Option<String>,
    ) -> A2AMessage {
        let local_agents = self.router.get_agent_ids().await;

        let capabilities: Vec<crate::a2a::protocol::AgentCapability> = local_agents
            .into_iter()
            .filter(|id| {
                if let Some(q) = &query {
                    id.0.to_lowercase().contains(&q.to_lowercase())
                } else {
                    true
                }
            })
            .map(|id| {
                crate::a2a::protocol::AgentCapability::new(format!("agent:{}", id.0), "1.0.0")
                    .with_description(format!("Local agent: {}", id.0))
            })
            .collect();

        A2AMessage::new(
            format!("discovery-resp-{}", uuid()),
            AgentId::new("ipc-router"),
            A2AMessageType::DiscoveryResponse,
            A2AMessagePayload::DiscoveryResponse {
                capabilities,
                metadata: Some(
                    [("router_endpoint".to_string(), self.endpoint.clone())]
                        .into_iter()
                        .collect(),
                ),
            },
        )
        .with_target(source)
        .with_correlation_id(format!("discovery-req-{}", uuid()))
    }
}

/// Generate a short unique ID string.
fn uuid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let raw = t.as_nanos() ^ ((seq as u128).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    format!("{:08x}", raw & 0xFFFFFFFF)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a2a::protocol::{A2AMessage, A2AMessagePayload, A2AMessageType, MessagePriority};

    fn make_msg() -> A2AMessage {
        A2AMessage::new(
            "msg-001",
            AgentId::new("sender"),
            A2AMessageType::Event,
            A2AMessagePayload::Event {
                event_type: "test".to_string(),
                data: Default::default(),
            },
        )
        .with_target(AgentId::new("receiver"))
    }

    // ── test_a2a_message_encode_decode_roundtrip ─────────────────────────────
    #[test]
    fn test_a2a_message_encode_decode_roundtrip() {
        let original = make_msg();
        let bytes = IpcRouter::encode_message(&original).expect("encode should succeed");
        assert!(!bytes.is_empty());

        let decoded = IpcRouter::decode_message(&bytes).expect("decode should succeed");

        assert_eq!(decoded.source, original.source);
        assert_eq!(decoded.target, original.target);
        assert_eq!(decoded.id, original.id);
        assert_eq!(decoded.message_type, original.message_type);
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
        let msg = A2AMessage::new(
            "msg-empty",
            AgentId::new("a"),
            A2AMessageType::Event,
            A2AMessagePayload::Event {
                event_type: "empty".to_string(),
                data: Default::default(),
            },
        );
        let bytes = IpcRouter::encode_message(&msg).unwrap();
        let decoded = IpcRouter::decode_message(&bytes).unwrap();
        assert_eq!(decoded.id, "msg-empty");
    }

    // ── test_ipc_router_register_local_agent ─────────────────────────────────
    #[tokio::test]
    async fn test_ipc_router_register_local_agent() {
        let bus = Arc::new(EventBus::new());
        let router = IpcRouter::new(Arc::clone(&bus), "/tmp/claw-test.sock");

        let agent_id = AgentId::new("local-agent");
        let _handle = router.register_agent(agent_id.clone(), 100).await;

        assert!(router.router.is_agent_registered(&agent_id).await);

        let ids = router.local_agent_ids().await;
        assert!(ids.contains(&agent_id));
    }

    // ── test_ipc_router_register_remote_endpoint ─────────────────────────────
    #[tokio::test]
    async fn test_ipc_router_register_remote_endpoint() {
        let bus = Arc::new(EventBus::new());
        let router = IpcRouter::new(Arc::clone(&bus), "/tmp/claw-test.sock");

        let agent_id = AgentId::new("remote-agent");
        router
            .register_remote_endpoint(agent_id.clone(), "tcp://192.168.1.1:8080")
            .await;

        let endpoint = router.get_remote_endpoint(&agent_id).await;
        assert_eq!(endpoint, Some("tcp://192.168.1.1:8080".to_string()));
    }

    // ── test_ipc_router_route_to_local_agent ─────────────────────────────────
    #[tokio::test]
    async fn test_ipc_router_route_to_local_agent() {
        let bus = Arc::new(EventBus::new());
        let router = IpcRouter::new(Arc::clone(&bus), "/tmp/claw-test.sock");

        let target_id = AgentId::new("target-agent");
        let _handle = router.register_agent(target_id.clone(), 100).await;

        let msg = A2AMessage::new(
            "routed-msg",
            AgentId::new("sender"),
            A2AMessageType::Event,
            A2AMessagePayload::Event {
                event_type: "test".to_string(),
                data: Default::default(),
            },
        )
        .with_target(target_id)
        .with_priority(MessagePriority::High);

        // Should succeed (delivered)
        let result = router.route_message(msg).await;
        assert!(result.is_ok());
    }

    // ── test_ipc_router_route_to_unknown_agent ───────────────────────────────
    #[tokio::test]
    async fn test_ipc_router_route_to_unknown_agent() {
        let bus = Arc::new(EventBus::new());
        let router = IpcRouter::new(Arc::clone(&bus), "/tmp/claw-test.sock");

        let msg = A2AMessage::new(
            "lost-msg",
            AgentId::new("sender"),
            A2AMessageType::Event,
            A2AMessagePayload::Event {
                event_type: "test".to_string(),
                data: Default::default(),
            },
        )
        .with_target(AgentId::new("unknown-agent"));

        let result = router.route_message(msg).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RuntimeError::AgentNotFound(_)
        ));
    }

    // ── test_ipc_router_handle_discovery_request ─────────────────────────────
    #[tokio::test]
    async fn test_ipc_router_handle_discovery_request() {
        let bus = Arc::new(EventBus::new());
        let router = IpcRouter::new(Arc::clone(&bus), "/tmp/claw-test.sock");

        // Register some agents
        let _ = router
            .register_agent(AgentId::new("agent-alpha"), 100)
            .await;
        let _ = router.register_agent(AgentId::new("agent-beta"), 100).await;

        let response = router
            .handle_discovery_request(AgentId::new("requester"), None)
            .await;

        assert_eq!(response.message_type, A2AMessageType::DiscoveryResponse);
        assert_eq!(response.target, Some(AgentId::new("requester")));

        match response.payload {
            A2AMessagePayload::DiscoveryResponse { capabilities, .. } => {
                assert_eq!(capabilities.len(), 2);
            }
            _ => panic!("Expected DiscoveryResponse payload"),
        }
    }

    // ── test_ipc_router_send ───────────────────────────────────
    #[tokio::test]
    async fn test_ipc_router_send() {
        let bus = Arc::new(EventBus::new());
        let router = IpcRouter::new(Arc::clone(&bus), "/tmp/claw-test.sock");

        let target_id = AgentId::new("target");
        let _handle = router.register_agent(target_id.clone(), 100).await;

        let msg = A2AMessage::new(
            "direct-msg",
            AgentId::new("sender"),
            A2AMessageType::Request,
            A2AMessagePayload::Request {
                action: "ping".to_string(),
                extra: Default::default(),
            },
        )
        .with_target(target_id.clone())
        .with_priority(MessagePriority::Critical);

        let result = router.send(&target_id, msg).await;
        assert!(result.is_ok());
    }
}
