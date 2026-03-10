//! Endpoint registry — stores and queries remote agent endpoint mappings.

use crate::agent_types::AgentId;
use std::collections::HashMap;
use tokio::sync::RwLock;

/// Holds the mapping of `AgentId` → remote IPC endpoint string.
pub(super) struct EndpointRegistry {
    inner: RwLock<HashMap<AgentId, String>>,
}

impl EndpointRegistry {
    /// Create an empty registry.
    pub(super) fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Register a remote agent endpoint.
    pub(super) async fn register(&self, agent_id: AgentId, endpoint: impl Into<String>) {
        let mut map = self.inner.write().await;
        map.insert(agent_id, endpoint.into());
    }

    /// Unregister a remote agent endpoint.
    pub(super) async fn unregister(&self, agent_id: &AgentId) {
        let mut map = self.inner.write().await;
        map.remove(agent_id);
    }

    /// Look up the endpoint for a remote agent.
    pub(super) async fn get(&self, agent_id: &AgentId) -> Option<String> {
        let map = self.inner.read().await;
        map.get(agent_id).cloned()
    }
}
