//! Router — core message routing logic (local, remote, broadcast, discovery).

use super::codec;
use super::endpoint_registry::EndpointRegistry;
use super::IpcTransportFactory;
use crate::a2a::protocol::{A2AMessage, A2AMessagePayload, A2AMessageType, AgentCapability};
use crate::a2a::routing::SimpleRouter;
use crate::agent_types::AgentId;
use crate::error::RuntimeError;
use crate::event_bus::EventBus;
use crate::events::Event;
use std::sync::Arc;

/// Route a message to its target (local or remote).
///
/// First checks if the target is a local agent, then falls back to remote.
/// Emits an `Event::A2A` when a message is successfully routed.
pub(super) async fn route_message(
    message: A2AMessage,
    router: &Arc<SimpleRouter>,
    event_bus: &Arc<EventBus>,
    remote_endpoints: &EndpointRegistry,
    transport_factory: &Arc<dyn IpcTransportFactory>,
) -> Result<(), RuntimeError> {
    // Publish A2A event before routing
    let _ = event_bus.publish(Event::A2A(message.clone()));

    // Check if target is local
    if let Some(target) = &message.target {
        if router.is_agent_registered(target).await {
            return router.route_message(message).await;
        }

        // Check if target is a known remote agent — use configured transport factory.
        if let Some(remote_ep) = remote_endpoints.get(target).await {
            let bytes = codec::encode_message(&message)?;
            let transport = transport_factory.create_client(&remote_ep).await?;
            transport.send(&bytes).await?;
            return Ok(());
        }

        // Target not found
        return Err(RuntimeError::AgentNotFound(target.0.clone()));
    }

    // Broadcast - send to all local agents
    router.route_message(message).await
}

/// Send a message directly to a specific agent (bypasses routing logic).
pub(super) async fn send_direct(
    target: &AgentId,
    message: A2AMessage,
    router: &Arc<SimpleRouter>,
) -> Result<(), RuntimeError> {
    router.send(target, message).await
}

/// Get list of all registered local agent IDs.
pub(super) async fn local_agent_ids(router: &Arc<SimpleRouter>) -> Vec<AgentId> {
    router.get_agent_ids().await
}

/// Handle a discovery request and return a response message.
pub(super) async fn handle_discovery_request(
    source: AgentId,
    query: Option<String>,
    router: &Arc<SimpleRouter>,
    endpoint: &str,
) -> A2AMessage {
    let local_agents = router.get_agent_ids().await;

    let capabilities: Vec<AgentCapability> = local_agents
        .into_iter()
        .filter(|id| {
            if let Some(q) = &query {
                id.0.to_lowercase().contains(&q.to_lowercase())
            } else {
                true
            }
        })
        .map(|id| {
            AgentCapability::new(format!("agent:{}", id.0), "1.0.0")
                .with_description(format!("Local agent: {}", id.0))
        })
        .collect();

    A2AMessage::new(
        format!("discovery-resp-{}", codec::uuid()),
        AgentId::new("ipc-router"),
        A2AMessageType::DiscoveryResponse,
        A2AMessagePayload::DiscoveryResponse {
            capabilities,
            metadata: Some(
                [("router_endpoint".to_string(), endpoint.to_string())]
                    .into_iter()
                    .collect(),
            ),
        },
    )
    .with_target(source)
    .with_correlation_id(format!("discovery-req-{}", codec::uuid()))
}
