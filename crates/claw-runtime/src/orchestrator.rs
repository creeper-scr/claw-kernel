use dashmap::DashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    agent_types::{AgentConfig, AgentHandle, AgentId, AgentInfo},
    error::RuntimeError,
    event_bus::EventBus,
    events::Event,
};

// ─── AgentOrchestrator ────────────────────────────────────────────────────────

/// Manages multiple agents' lifecycle and coordination.
///
/// Thread-safe: backed by `DashMap` for concurrent access without a global
/// lock.  All state-changing operations publish the appropriate `Event` to
/// the shared `EventBus`.
pub struct AgentOrchestrator {
    agents: Arc<DashMap<AgentId, AgentInfo>>,
    event_bus: Arc<EventBus>,
}

impl AgentOrchestrator {
    /// Create a new orchestrator that publishes to `event_bus`.
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            agents: Arc::new(DashMap::new()),
            event_bus,
        }
    }

    /// Register a new agent and publish an `AgentStarted` event.
    ///
    /// Returns `Err(AgentAlreadyExists)` if an agent with the same ID is
    /// already registered.
    pub fn register(&self, config: AgentConfig) -> Result<AgentHandle, RuntimeError> {
        let agent_id = config.agent_id.clone();

        if self.agents.contains_key(&agent_id) {
            return Err(RuntimeError::AgentAlreadyExists(agent_id.0.clone()));
        }

        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let info = AgentInfo {
            config,
            started_at,
        };

        self.agents.insert(agent_id.clone(), info);

        // Publish AgentStarted event (best-effort; ignore send errors when
        // there are no subscribers).
        let _ = self.event_bus.publish(Event::AgentStarted {
            agent_id: agent_id.clone(),
        });

        Ok(AgentHandle {
            agent_id,
            event_bus: (*self.event_bus).clone(),
        })
    }

    /// Unregister an agent and publish an `AgentStopped` event.
    ///
    /// Returns `Err(AgentNotFound)` if the agent is not registered.
    pub fn unregister(
        &self,
        agent_id: &AgentId,
        reason: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        if self.agents.remove(agent_id).is_none() {
            return Err(RuntimeError::AgentNotFound(agent_id.0.clone()));
        }

        let _ = self.event_bus.publish(Event::AgentStopped {
            agent_id: agent_id.clone(),
            reason: reason.into(),
        });

        Ok(())
    }

    /// Return a snapshot of the `AgentInfo` for `agent_id`, or `None` if not
    /// found.
    pub fn agent_info(&self, agent_id: &AgentId) -> Option<AgentInfo> {
        self.agents.get(agent_id).map(|r| r.value().clone())
    }

    /// Return a snapshot of all registered agent IDs.
    pub fn agent_ids(&self) -> Vec<AgentId> {
        self.agents.iter().map(|r| r.key().clone()).collect()
    }

    /// Return the number of currently registered agents.
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_types::AgentConfig;

    fn make_orchestrator() -> AgentOrchestrator {
        let bus = Arc::new(EventBus::new());
        AgentOrchestrator::new(bus)
    }

    // ── test_orchestrator_register_agent ─────────────────────────────────────
    #[test]
    fn test_orchestrator_register_agent() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("alpha");
        let id = config.agent_id.clone();

        let handle = orc.register(config).expect("register should succeed");

        assert_eq!(handle.agent_id, id);
        assert_eq!(orc.agent_count(), 1);

        let info = orc.agent_info(&id).expect("agent should be findable");
        assert_eq!(info.config.name, "alpha");
        assert!(info.started_at > 0);
    }

    // ── test_orchestrator_duplicate_agent_fails ───────────────────────────────
    #[test]
    fn test_orchestrator_duplicate_agent_fails() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("beta");
        let id = config.agent_id.clone();

        orc.register(config.clone()).expect("first register ok");

        // Try to register another config with the same agent_id.
        let dup = AgentConfig {
            agent_id: id.clone(),
            name: "beta-dup".to_string(),
            mode: config.mode,
            metadata: Default::default(),
        };
        let result = orc.register(dup);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RuntimeError::AgentAlreadyExists(_)));
    }

    // ── test_orchestrator_unregister_agent ────────────────────────────────────
    #[test]
    fn test_orchestrator_unregister_agent() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("gamma");
        let id = config.agent_id.clone();

        orc.register(config).unwrap();
        assert_eq!(orc.agent_count(), 1);

        orc.unregister(&id, "test done").expect("unregister should succeed");
        assert_eq!(orc.agent_count(), 0);
        assert!(orc.agent_info(&id).is_none());
    }

    // ── test_orchestrator_unregister_nonexistent_fails ────────────────────────
    #[test]
    fn test_orchestrator_unregister_nonexistent_fails() {
        let orc = make_orchestrator();
        let id = AgentId::new("ghost-agent");
        let result = orc.unregister(&id, "cleanup");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RuntimeError::AgentNotFound(_)));
    }

    // ── test_orchestrator_agent_count ────────────────────────────────────────
    #[test]
    fn test_orchestrator_agent_count() {
        let orc = make_orchestrator();

        assert_eq!(orc.agent_count(), 0);

        let c1 = AgentConfig::new("one");
        let c2 = AgentConfig::new("two");
        let c3 = AgentConfig::new("three");
        let id2 = c2.agent_id.clone();

        orc.register(c1).unwrap();
        orc.register(c2).unwrap();
        orc.register(c3).unwrap();
        assert_eq!(orc.agent_count(), 3);

        orc.unregister(&id2, "removed").unwrap();
        assert_eq!(orc.agent_count(), 2);
    }

    // ── test_orchestrator_agent_ids ───────────────────────────────────────────
    #[test]
    fn test_orchestrator_agent_ids() {
        let orc = make_orchestrator();
        let c1 = AgentConfig::new("a");
        let c2 = AgentConfig::new("b");
        let id1 = c1.agent_id.clone();
        let id2 = c2.agent_id.clone();

        orc.register(c1).unwrap();
        orc.register(c2).unwrap();

        let mut ids = orc.agent_ids();
        ids.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    // ── test_orchestrator_events_published ───────────────────────────────────
    #[tokio::test]
    async fn test_orchestrator_events_published() {
        use crate::events::Event;

        let bus = Arc::new(EventBus::new());
        let orc = AgentOrchestrator::new(Arc::clone(&bus));
        let mut rx = bus.subscribe();

        let config = AgentConfig::new("event-agent");
        let id = config.agent_id.clone();
        orc.register(config).unwrap();
        orc.unregister(&id, "done").unwrap();

        // Should receive AgentStarted then AgentStopped.
        let e1 = rx.recv().await.unwrap();
        assert!(matches!(e1, Event::AgentStarted { .. }));

        let e2 = rx.recv().await.unwrap();
        assert!(matches!(e2, Event::AgentStopped { .. }));
    }
}
