use claw_pal::{ProcessConfig, TokioProcessManager};
use claw_pal::traits::ProcessManager;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{
    agent_types::{AgentConfig, AgentHandle, AgentId, AgentInfo, AgentStatus},
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
///
/// Two registration paths exist:
/// - [`register`] — in-process agents (no OS process spawned).
/// - [`spawn`] — out-of-process agents backed by a real OS process via PAL.
pub struct AgentOrchestrator {
    agents: Arc<DashMap<AgentId, AgentInfo>>,
    event_bus: Arc<EventBus>,
    process_manager: Arc<TokioProcessManager>,
}

impl AgentOrchestrator {
    /// Create a new orchestrator with a default `TokioProcessManager`.
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self::with_process_manager(event_bus, Arc::new(TokioProcessManager::new()))
    }

    /// Create a new orchestrator with an injected `TokioProcessManager`.
    ///
    /// Use this constructor when you want `Runtime` and `AgentOrchestrator`
    /// to share the same process manager instance.
    pub fn with_process_manager(
        event_bus: Arc<EventBus>,
        process_manager: Arc<TokioProcessManager>,
    ) -> Self {
        Self {
            agents: Arc::new(DashMap::new()),
            event_bus,
            process_manager,
        }
    }

    // ── In-process registration ───────────────────────────────────────────────

    /// Register a new in-process agent and publish an `AgentStarted` event.
    ///
    /// Does **not** spawn an OS process; use [`spawn`] for that.
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
            process_handle: None,
            status: AgentStatus::Running,
        };

        self.agents.insert(agent_id.clone(), info);

        let _ = self.event_bus.publish(Event::AgentStarted {
            agent_id: agent_id.clone(),
        });

        Ok(AgentHandle {
            agent_id,
            event_bus: (*self.event_bus).clone(),
        })
    }

    /// Unregister an in-process agent and publish an `AgentStopped` event.
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

    // ── Out-of-process agent lifecycle ───────────────────────────────────────

    /// Spawn a new agent as an OS process and register it.
    ///
    /// Uses the PAL `TokioProcessManager` to start the process.
    /// Returns `Err(AgentAlreadyExists)` if the ID is already taken.
    pub async fn spawn(
        &self,
        config: AgentConfig,
        process_config: ProcessConfig,
    ) -> Result<AgentHandle, RuntimeError> {
        let agent_id = config.agent_id.clone();

        if self.agents.contains_key(&agent_id) {
            return Err(RuntimeError::AgentAlreadyExists(agent_id.0.clone()));
        }

        let process_handle = self
            .process_manager
            .spawn(process_config)
            .await
            .map_err(|e| RuntimeError::ProcessError(e.to_string()))?;

        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let info = AgentInfo {
            config,
            started_at,
            process_handle: Some(process_handle),
            status: AgentStatus::Running,
        };

        self.agents.insert(agent_id.clone(), info);

        let _ = self.event_bus.publish(Event::AgentStarted {
            agent_id: agent_id.clone(),
        });

        Ok(AgentHandle {
            agent_id,
            event_bus: (*self.event_bus).clone(),
        })
    }

    /// Gracefully terminate a spawned agent process and remove it.
    ///
    /// Sends SIGTERM and waits up to `grace_period`; falls back to SIGKILL.
    /// Also works for in-process agents (no-op on the process side).
    /// Returns `Err(AgentNotFound)` if the agent is not registered.
    pub async fn terminate(
        &self,
        agent_id: &AgentId,
        grace_period: Duration,
    ) -> Result<(), RuntimeError> {
        // Take the process handle under a short lock, then release.
        let process_handle = {
            match self.agents.get_mut(agent_id) {
                None => return Err(RuntimeError::AgentNotFound(agent_id.0.clone())),
                Some(mut entry) => {
                    entry.status = AgentStatus::Stopped;
                    entry.process_handle.take()
                }
            }
        };

        let result = if let Some(ph) = process_handle {
            self.process_manager
                .terminate(ph, grace_period)
                .await
                .map_err(|e| RuntimeError::ProcessError(e.to_string()))
        } else {
            Ok(())
        };

        // Always remove and publish, even if terminate returned an error
        // (the process may have already exited).
        self.agents.remove(agent_id);
        let _ = self.event_bus.publish(Event::AgentStopped {
            agent_id: agent_id.clone(),
            reason: "terminated".to_string(),
        });

        result
    }

    /// Kill a spawned agent process immediately (SIGKILL / TerminateProcess).
    ///
    /// Also works for in-process agents (no-op on the process side).
    /// Returns `Err(AgentNotFound)` if the agent is not registered.
    pub async fn kill(&self, agent_id: &AgentId) -> Result<(), RuntimeError> {
        let process_handle = {
            match self.agents.get_mut(agent_id) {
                None => return Err(RuntimeError::AgentNotFound(agent_id.0.clone())),
                Some(mut entry) => {
                    entry.status = AgentStatus::Error;
                    entry.process_handle.take()
                }
            }
        };

        let result = if let Some(ph) = process_handle {
            self.process_manager
                .kill(ph)
                .await
                .map_err(|e| RuntimeError::ProcessError(e.to_string()))
        } else {
            Ok(())
        };

        self.agents.remove(agent_id);
        let _ = self.event_bus.publish(Event::AgentStopped {
            agent_id: agent_id.clone(),
            reason: "killed".to_string(),
        });

        result
    }

    // ── Query methods ─────────────────────────────────────────────────────────

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
        assert!(info.process_handle.is_none());
        assert_eq!(info.status, AgentStatus::Running);
    }

    // ── test_orchestrator_duplicate_agent_fails ───────────────────────────────
    #[test]
    fn test_orchestrator_duplicate_agent_fails() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("beta");
        let id = config.agent_id.clone();

        orc.register(config.clone()).expect("first register ok");

        let dup = AgentConfig {
            agent_id: id.clone(),
            name: "beta-dup".to_string(),
            mode: config.mode,
            metadata: Default::default(),
        };
        let result = orc.register(dup);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RuntimeError::AgentAlreadyExists(_)
        ));
    }

    // ── test_orchestrator_unregister_agent ────────────────────────────────────
    #[test]
    fn test_orchestrator_unregister_agent() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("gamma");
        let id = config.agent_id.clone();

        orc.register(config).unwrap();
        assert_eq!(orc.agent_count(), 1);

        orc.unregister(&id, "test done")
            .expect("unregister should succeed");
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
        assert!(matches!(
            result.unwrap_err(),
            RuntimeError::AgentNotFound(_)
        ));
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

        let e1 = rx.recv().await.unwrap();
        assert!(matches!(e1, Event::AgentStarted { .. }));

        let e2 = rx.recv().await.unwrap();
        assert!(matches!(e2, Event::AgentStopped { .. }));
    }

    // ── test_orchestrator_spawn_real_process ──────────────────────────────────
    #[tokio::test]
    async fn test_orchestrator_spawn_real_process() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("echo-agent");
        let agent_id = config.agent_id.clone();

        let process_config = ProcessConfig::new("echo".to_string())
            .with_arg("hello from claw-runtime".to_string());

        let handle = orc
            .spawn(config, process_config)
            .await
            .expect("spawn should succeed");

        assert_eq!(handle.agent_id, agent_id);
        assert_eq!(orc.agent_count(), 1);

        let info = orc.agent_info(&agent_id).expect("agent should be present");
        assert!(info.process_handle.is_some());
        assert_eq!(info.status, AgentStatus::Running);
    }

    // ── test_orchestrator_spawn_duplicate_fails ───────────────────────────────
    #[tokio::test]
    async fn test_orchestrator_spawn_duplicate_fails() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("dup-agent");
        let agent_id = config.agent_id.clone();

        let pc = ProcessConfig::new("echo".to_string());

        // First spawn succeeds.
        orc.spawn(config.clone(), pc.clone()).await.unwrap();

        // Second spawn with same ID fails.
        let dup_config = AgentConfig {
            agent_id: agent_id.clone(),
            name: "dup-agent-2".to_string(),
            mode: config.mode,
            metadata: Default::default(),
        };
        let result = orc.spawn(dup_config, pc).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RuntimeError::AgentAlreadyExists(_)
        ));

        // Clean up.
        let _ = orc.terminate(&agent_id, Duration::from_secs(2)).await;
    }

    // ── test_orchestrator_terminate_spawned_process ───────────────────────────
    #[tokio::test]
    async fn test_orchestrator_terminate_spawned_process() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("sleep-agent");
        let agent_id = config.agent_id.clone();

        #[cfg(unix)]
        let pc = ProcessConfig::new("sleep".to_string()).with_arg("60".to_string());
        #[cfg(windows)]
        let pc = ProcessConfig::new("ping".to_string())
            .with_args(vec!["-n".to_string(), "60".to_string(), "127.0.0.1".to_string()]);

        orc.spawn(config, pc).await.expect("spawn should succeed");
        assert_eq!(orc.agent_count(), 1);

        orc.terminate(&agent_id, Duration::from_millis(100))
            .await
            .expect("terminate should succeed");

        assert_eq!(orc.agent_count(), 0);
        assert!(orc.agent_info(&agent_id).is_none());
    }

    // ── test_orchestrator_kill_spawned_process ────────────────────────────────
    #[tokio::test]
    async fn test_orchestrator_kill_spawned_process() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("kill-agent");
        let agent_id = config.agent_id.clone();

        #[cfg(unix)]
        let pc = ProcessConfig::new("sleep".to_string()).with_arg("60".to_string());
        #[cfg(windows)]
        let pc = ProcessConfig::new("ping".to_string())
            .with_args(vec!["-n".to_string(), "60".to_string(), "127.0.0.1".to_string()]);

        orc.spawn(config, pc).await.expect("spawn should succeed");

        orc.kill(&agent_id)
            .await
            .expect("kill should succeed");

        assert_eq!(orc.agent_count(), 0);
    }

    // ── test_orchestrator_terminate_inprocess_agent ───────────────────────────
    #[tokio::test]
    async fn test_orchestrator_terminate_inprocess_agent() {
        // terminate() on an in-process agent (no process handle) should
        // succeed and remove the agent.
        let orc = make_orchestrator();
        let config = AgentConfig::new("inproc");
        let id = config.agent_id.clone();
        orc.register(config).unwrap();

        orc.terminate(&id, Duration::from_secs(1))
            .await
            .expect("terminate of in-process agent should succeed");

        assert_eq!(orc.agent_count(), 0);
    }

    // ── test_orchestrator_terminate_notfound_fails ────────────────────────────
    #[tokio::test]
    async fn test_orchestrator_terminate_notfound_fails() {
        let orc = make_orchestrator();
        let id = AgentId::new("ghost");
        let result = orc.terminate(&id, Duration::from_secs(1)).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RuntimeError::AgentNotFound(_)
        ));
    }
}
