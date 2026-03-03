//! Simple A2A Message Routing
//!
//! Provides basic message routing without complex priority handling.

use crate::a2a::protocol::A2AMessage;
use crate::agent_types::AgentId;
use crate::error::RuntimeError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

// ─── AgentHandle ──────────────────────────────────────────────────────────────

/// Handle for communicating with a registered agent.
#[derive(Debug, Clone)]
pub struct AgentHandle {
    /// The agent's unique identifier.
    pub agent_id: AgentId,
    /// Channel sender for delivering messages to the agent.
    pub sender: mpsc::Sender<A2AMessage>,
}

impl AgentHandle {
    /// Send a message to this agent.
    pub async fn send(&self, message: A2AMessage) -> Result<(), RuntimeError> {
        self.sender
            .send(message)
            .await
            .map_err(|_| RuntimeError::IpcError("agent channel closed".to_string()))
    }

    /// Try to send without blocking.
    pub fn try_send(&self, message: A2AMessage) -> Result<(), RuntimeError> {
        self.sender
            .try_send(message)
            .map_err(|_| RuntimeError::IpcError("agent channel full or closed".to_string()))
    }
}

// ─── SimpleRouter ───────────────────────────────────────────────────────────

/// Simple A2A message router.
///
/// Maintains a registry of local agents with their communication channels.
/// Messages are routed directly to target agents without complex queuing.
#[derive(Debug)]
pub struct SimpleRouter {
    /// Registered local agents.
    agents: Arc<RwLock<HashMap<AgentId, AgentHandle>>>,
}

impl SimpleRouter {
    /// Create a new simple router.
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new agent with the router.
    pub async fn register_agent(&self, agent_id: AgentId, buffer_size: usize) -> AgentHandle {
        let (tx, mut rx) = mpsc::channel(buffer_size);

        let handle = AgentHandle {
            agent_id: agent_id.clone(),
            sender: tx,
        };

        let mut agents = self.agents.write().await;
        agents.insert(agent_id.clone(), handle.clone());

        // Spawn a task to handle the receiver side (in real usage, agent would own rx)
        tokio::spawn(async move {
            // In a real implementation, the agent would own this receiver
            // This dummy handler just drains the channel to prevent blocking
            while let Some(_msg) = rx.recv().await {
                tracing::debug!("Agent {} received message", agent_id);
            }
        });

        handle
    }

    /// Unregister an agent from the router.
    pub async fn unregister_agent(&self, agent_id: &AgentId) -> Result<(), RuntimeError> {
        let mut agents = self.agents.write().await;
        if agents.remove(agent_id).is_none() {
            return Err(RuntimeError::AgentNotFound(agent_id.0.clone()));
        }
        Ok(())
    }

    /// Check if an agent is registered.
    pub async fn is_agent_registered(&self, agent_id: &AgentId) -> bool {
        let agents = self.agents.read().await;
        agents.contains_key(agent_id)
    }

    /// Get a handle to a registered agent.
    pub async fn get_agent(&self, agent_id: &AgentId) -> Option<AgentHandle> {
        let agents = self.agents.read().await;
        agents.get(agent_id).cloned()
    }

    /// Get list of all registered agent IDs.
    pub async fn get_agent_ids(&self) -> Vec<AgentId> {
        let agents = self.agents.read().await;
        agents.keys().cloned().collect()
    }

    /// Route a message to its target.
    ///
    /// For messages with a target, attempts direct delivery.
    /// For messages without a target, broadcasts to all agents.
    pub async fn route_message(&self, message: A2AMessage) -> Result<(), RuntimeError> {
        let target = match &message.target {
            Some(t) => t.clone(),
            None => {
                // Broadcast to all agents
                return self.broadcast_message(message).await;
            }
        };

        // Try direct delivery
        if let Some(handle) = self.get_agent(&target).await {
            handle.send(message).await?;
            Ok(())
        } else {
            Err(RuntimeError::AgentNotFound(target.0.clone()))
        }
    }

    /// Send a message to a specific agent.
    pub async fn send(&self, target: &AgentId, message: A2AMessage) -> Result<(), RuntimeError> {
        if let Some(handle) = self.get_agent(target).await {
            handle.send(message).await
        } else {
            Err(RuntimeError::AgentNotFound(target.0.clone()))
        }
    }

    /// Broadcast a message to all registered agents.
    async fn broadcast_message(&self, message: A2AMessage) -> Result<(), RuntimeError> {
        let agents = self.agents.read().await;

        for (agent_id, handle) in agents.iter() {
            // Don't send to self if source is in the list
            if message.source == *agent_id {
                continue;
            }

            let _ = handle.try_send(message.clone());
        }

        Ok(())
    }
}

impl Default for SimpleRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a2a::protocol::{A2AMessagePayload, A2AMessageType, MessagePriority};

    fn create_test_message(
        id: &str,
        source: &str,
        target: Option<&str>,
        priority: MessagePriority,
    ) -> A2AMessage {
        let mut msg = A2AMessage::new(
            id,
            AgentId::new(source),
            A2AMessageType::Event,
            A2AMessagePayload::Event {
                event_type: "test".to_string(),
                data: Default::default(),
            },
        )
        .with_priority(priority);

        if let Some(t) = target {
            msg = msg.with_target(AgentId::new(t));
        }

        msg
    }

    #[tokio::test]
    async fn test_simple_router_register_unregister() {
        let router = SimpleRouter::new();

        let agent_id = AgentId::new("test-agent");

        // Register
        let _handle = router.register_agent(agent_id.clone(), 100).await;

        assert!(router.is_agent_registered(&agent_id).await);

        // Unregister
        router.unregister_agent(&agent_id).await.unwrap();
        assert!(!router.is_agent_registered(&agent_id).await);
    }

    #[tokio::test]
    async fn test_simple_router_route_to_registered() {
        let router = SimpleRouter::new();

        let target_id = AgentId::new("target");
        let _handle = router.register_agent(target_id.clone(), 100).await;

        let msg = create_test_message("msg-1", "source", Some("target"), MessagePriority::Normal);

        // Should succeed because target is registered
        router.route_message(msg).await.unwrap();
    }

    #[tokio::test]
    async fn test_simple_router_route_to_unregistered_fails() {
        let router = SimpleRouter::new();

        let msg = create_test_message(
            "msg-1",
            "source",
            Some("unregistered"),
            MessagePriority::Normal,
        );

        // Should fail because target is not registered
        let result = router.route_message(msg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_simple_router_broadcast() {
        let router = SimpleRouter::new();

        // Register multiple agents
        let _ = router.register_agent(AgentId::new("agent-1"), 100).await;
        let _ = router.register_agent(AgentId::new("agent-2"), 100).await;

        // Broadcast message (no target)
        let msg = create_test_message("broadcast", "source", None, MessagePriority::Normal);

        router.route_message(msg).await.unwrap();
    }

    #[tokio::test]
    async fn test_simple_router_send_direct() {
        let router = SimpleRouter::new();

        let target_id = AgentId::new("target");
        let _handle = router.register_agent(target_id.clone(), 100).await;

        let msg = create_test_message("msg-1", "source", Some("target"), MessagePriority::Normal);

        router.send(&target_id, msg).await.unwrap();
    }

    #[tokio::test]
    async fn test_simple_router_get_agent_ids() {
        let router = SimpleRouter::new();

        router.register_agent(AgentId::new("agent-1"), 100).await;
        router.register_agent(AgentId::new("agent-2"), 100).await;

        let ids = router.get_agent_ids().await;
        assert_eq!(ids.len(), 2);
    }
}
