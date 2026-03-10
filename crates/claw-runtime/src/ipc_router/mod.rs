//! IPC Router with A2A Protocol Support
//!
//! Routes A2A messages between agents, supporting both local (in-process)
//! and remote (IPC) message delivery.
//!
//! # Sub-modules
//!
//! - [`codec`]             — JSON serialization/deserialization of `A2AMessage`
//! - [`endpoint_registry`] — remote endpoint registration and lookup
//! - [`acceptor`]          — inbound connection listener and dispatcher
//! - [`router`]            — core routing logic (local, remote, broadcast, discovery)

mod acceptor;
mod codec;
mod endpoint_registry;
mod router;

use crate::a2a::protocol::A2AMessage;
use crate::a2a::routing::{AgentHandle, SimpleRouter};
use crate::agent_types::AgentId;
use crate::error::RuntimeError;
use crate::event_bus::EventBus;
use endpoint_registry::EndpointRegistry;
use std::sync::Arc;

// ─── IpcTransportFactory Trait ────────────────────────────────────────────────

/// Factory trait for creating IPC transport connections.
///
/// This trait abstracts over different IPC implementations (Unix sockets,
/// Named Pipes, in-memory channels, etc.) and allows dependency injection
/// for testing.
#[async_trait::async_trait]
pub trait IpcTransportFactory: Send + Sync {
    /// Create a client transport connected to the given endpoint.
    async fn create_client(&self, endpoint: &str) -> Result<Box<dyn IpcConnection>, RuntimeError>;

    /// Create a server transport listening on the given endpoint.
    async fn create_server(&self, endpoint: &str) -> Result<Box<dyn IpcConnection>, RuntimeError>;
}

/// Runtime-specific IPC transport trait for sending and receiving messages.
///
/// This is the object-safe subset of transport operations, adapted for
/// use with RuntimeError instead of IpcError.
#[async_trait::async_trait]
pub trait IpcConnection: Send + Sync {
    /// Send a message through the connection.
    async fn send(&self, msg: &[u8]) -> Result<(), RuntimeError>;

    /// Receive a message from the connection.
    async fn recv(&self) -> Result<Vec<u8>, RuntimeError>;
}

// ─── Default Interprocess Transport Factory ───────────────────────────────────

/// Default factory using `claw_pal::InterprocessTransport`.
pub struct InterprocessTransportFactory;

impl InterprocessTransportFactory {
    /// Create a new factory.
    pub fn new() -> Self {
        Self
    }
}

impl Default for InterprocessTransportFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl IpcTransportFactory for InterprocessTransportFactory {
    async fn create_client(&self, endpoint: &str) -> Result<Box<dyn IpcConnection>, RuntimeError> {
        use claw_pal::InterprocessTransport;
        let transport = InterprocessTransport::new_client(endpoint)
            .await
            .map_err(|e| RuntimeError::IpcError(e.to_string()))?;
        Ok(Box::new(transport))
    }

    async fn create_server(&self, endpoint: &str) -> Result<Box<dyn IpcConnection>, RuntimeError> {
        use claw_pal::InterprocessTransport;
        let transport = InterprocessTransport::new_server(endpoint)
            .await
            .map_err(|e| RuntimeError::IpcError(e.to_string()))?;
        Ok(Box::new(transport))
    }
}

// Adapter to make claw_pal::InterprocessTransport implement IpcConnection
#[async_trait::async_trait]
impl IpcConnection for claw_pal::InterprocessTransport {
    async fn send(&self, msg: &[u8]) -> Result<(), RuntimeError> {
        use claw_pal::IpcTransport;
        IpcTransport::send(self, msg)
            .await
            .map_err(|e| RuntimeError::IpcError(e.to_string()))
    }

    async fn recv(&self) -> Result<Vec<u8>, RuntimeError> {
        use claw_pal::IpcTransport;
        IpcTransport::recv(self)
            .await
            .map_err(|e| RuntimeError::IpcError(e.to_string()))
    }
}

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
    remote_endpoints: Arc<EndpointRegistry>,
    /// Factory for creating IPC transport connections.
    transport_factory: Arc<dyn IpcTransportFactory>,
}

impl IpcRouter {
    /// Create a new `IpcRouter` with the given endpoint, event bus, and transport factory.
    pub fn new(
        event_bus: Arc<EventBus>,
        endpoint: impl Into<String>,
        transport_factory: Arc<dyn IpcTransportFactory>,
    ) -> Self {
        Self {
            router: Arc::new(SimpleRouter::new()),
            endpoint: endpoint.into(),
            event_bus,
            remote_endpoints: Arc::new(EndpointRegistry::new()),
            transport_factory,
        }
    }

    /// Create a new `IpcRouter` with the default transport factory.
    pub fn with_default_transport(event_bus: Arc<EventBus>, endpoint: impl Into<String>) -> Self {
        Self::new(
            event_bus,
            endpoint,
            Arc::new(InterprocessTransportFactory::new()),
        )
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

    // ── codec delegation ──────────────────────────────────────────────────────

    /// Serialize an `A2AMessage` to JSON bytes.
    pub fn encode_message(msg: &A2AMessage) -> Result<Vec<u8>, RuntimeError> {
        codec::encode_message(msg)
    }

    /// Deserialize an IPC frame payload back into an `A2AMessage`.
    pub fn decode_message(bytes: &[u8]) -> Result<A2AMessage, RuntimeError> {
        codec::decode_message(bytes)
    }

    // ── local agent management ────────────────────────────────────────────────

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

    // ── endpoint registry delegation ──────────────────────────────────────────

    /// Register a remote agent endpoint.
    pub async fn register_remote_endpoint(&self, agent_id: AgentId, endpoint: impl Into<String>) {
        self.remote_endpoints.register(agent_id, endpoint).await;
    }

    /// Unregister a remote agent endpoint.
    pub async fn unregister_remote_endpoint(&self, agent_id: &AgentId) {
        self.remote_endpoints.unregister(agent_id).await;
    }

    /// Get the endpoint for a remote agent.
    pub async fn get_remote_endpoint(&self, agent_id: &AgentId) -> Option<String> {
        self.remote_endpoints.get(agent_id).await
    }

    // ── router delegation ─────────────────────────────────────────────────────

    /// Route a message to its target (local or remote).
    ///
    /// First checks if the target is a local agent, then falls back to remote.
    /// Emits an `Event::A2A` when a message is successfully routed.
    pub async fn route_message(&self, message: A2AMessage) -> Result<(), RuntimeError> {
        router::route_message(
            message,
            &self.router,
            &self.event_bus,
            &self.remote_endpoints,
            &self.transport_factory,
        )
        .await
    }

    /// Send a message to a specific agent.
    pub async fn send(&self, target: &AgentId, message: A2AMessage) -> Result<(), RuntimeError> {
        router::send_direct(target, message, &self.router).await
    }

    /// Get list of all registered local agent IDs.
    pub async fn local_agent_ids(&self) -> Vec<AgentId> {
        router::local_agent_ids(&self.router).await
    }

    /// Handle a discovery request.
    pub async fn handle_discovery_request(
        &self,
        source: AgentId,
        query: Option<String>,
    ) -> A2AMessage {
        router::handle_discovery_request(source, query, &self.router, &self.endpoint).await
    }

    // ── acceptor delegation ───────────────────────────────────────────────────

    /// Start accepting incoming IPC connections in a background task.
    ///
    /// Each accepted connection is handled in a dedicated `tokio::spawn` task
    /// that reads frames, decodes them as `A2AMessage`, and routes them to
    /// local agents.
    pub async fn start_accepting(&self) -> Result<(), RuntimeError> {
        acceptor::start_accepting(
            self.endpoint.clone(),
            Arc::clone(&self.router),
            Arc::clone(&self.event_bus),
            Arc::clone(&self.transport_factory),
        )
        .await
    }
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
        let router = IpcRouter::with_default_transport(Arc::clone(&bus), "/tmp/claw-test.sock");
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
        let router = IpcRouter::with_default_transport(Arc::clone(&bus), "/tmp/claw-test.sock");

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
        let router = IpcRouter::with_default_transport(Arc::clone(&bus), "/tmp/claw-test.sock");

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
        let router = IpcRouter::with_default_transport(Arc::clone(&bus), "/tmp/claw-test.sock");

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
        let router = IpcRouter::with_default_transport(Arc::clone(&bus), "/tmp/claw-test.sock");

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
        let router = IpcRouter::with_default_transport(Arc::clone(&bus), "/tmp/claw-test.sock");

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
        let router = IpcRouter::with_default_transport(Arc::clone(&bus), "/tmp/claw-test.sock");

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

    // ── test_ipc_router_emits_a2a_event ────────────────────────
    #[tokio::test]
    async fn test_ipc_router_emits_a2a_event() {
        use crate::event_bus::EventFilter;
        use crate::events::Event;

        let bus = Arc::new(EventBus::new());
        let router = IpcRouter::with_default_transport(Arc::clone(&bus), "/tmp/claw-test-a2a.sock");

        // Register a local agent
        let target_id = AgentId::new("target-agent");
        let _handle = router.register_agent(target_id.clone(), 100).await;

        // Subscribe to A2A events
        let mut rx = bus.subscribe_with_filter(EventFilter::A2A);

        // Create and route a message
        let msg = A2AMessage::new(
            "a2a-test-msg",
            AgentId::new("sender"),
            A2AMessageType::Event,
            A2AMessagePayload::Event {
                event_type: "test".to_string(),
                data: Default::default(),
            },
        )
        .with_target(target_id);

        // Route the message
        router.route_message(msg.clone()).await.unwrap();

        // Should receive A2A event
        let event = rx.recv().await.unwrap();
        assert!(matches!(event, Event::A2A(..)));

        if let Event::A2A(received_msg) = event {
            assert_eq!(received_msg.id, "a2a-test-msg");
            assert_eq!(received_msg.source.0, "sender");
        }
    }
}
