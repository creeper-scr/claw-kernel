//! Core trait abstractions for the claw-runtime.
//!
//! Provides the [`Orchestrator`] trait that abstracts over agent lifecycle
//! management, enabling dependency injection and mock-based testing.

use async_trait::async_trait;
use std::time::Duration;

use crate::{
    agent_types::{AgentConfig, AgentHandle, AgentId, AgentStatus},
    error::RuntimeError,
    event_bus::EventReceiver,
    orchestrator::{HealthStatus, ResourceQuota, SteerCommand},
};

// ─── Orchestrator trait ───────────────────────────────────────────────────────

/// Core orchestration interface for managing agent lifecycles.
///
/// Implementations must be `Send + Sync` to support use across async tasks.
///
/// This trait mirrors the public API of [`crate::AgentOrchestrator`] and
/// exists to enable:
/// - **Dependency injection**: callers can accept `Arc<dyn Orchestrator>`
///   instead of a concrete `Arc<AgentOrchestrator>`.
/// - **Mock testing**: test doubles can implement this trait without standing
///   up a real process manager.
#[async_trait]
pub trait Orchestrator: Send + Sync {
    /// Register an agent configuration without spawning a process.
    fn register(&self, config: AgentConfig) -> Result<AgentHandle, RuntimeError>;

    /// Spawn an agent process using the provided process configuration.
    async fn spawn(
        &self,
        config: AgentConfig,
        process_config: claw_pal::ProcessConfig,
    ) -> Result<AgentHandle, RuntimeError>;

    /// Send a graceful termination signal, waiting up to `grace_period`.
    async fn terminate(
        &self,
        agent_id: &AgentId,
        grace_period: Duration,
    ) -> Result<(), RuntimeError>;

    /// Forcibly kill an agent.
    async fn kill(&self, agent_id: &AgentId) -> Result<(), RuntimeError>;

    /// Send a steering command to modify agent behavior.
    async fn steer(&self, agent_id: &AgentId, command: SteerCommand) -> Result<(), RuntimeError>;

    /// Update resource quota for an agent.
    async fn set_quota(&self, agent_id: &AgentId, quota: ResourceQuota) -> Result<(), RuntimeError>;

    /// Perform a health check and return the current status.
    async fn health_check(&self, agent_id: &AgentId) -> Result<HealthStatus, RuntimeError>;

    /// Unregister an in-process agent.
    fn unregister(&self, agent_id: &AgentId, reason: &str) -> Result<(), RuntimeError>;

    /// Return a snapshot of all registered agent IDs.
    fn list_agents(&self) -> Vec<AgentId>;

    /// Return the current status of an agent.
    fn agent_status(&self, agent_id: &AgentId) -> Result<AgentStatus, RuntimeError>;

    /// Return the number of currently registered agents.
    fn agent_count(&self) -> usize;

    /// Subscribe to events from this orchestrator.
    fn subscribe(&self) -> EventReceiver;
}

// ─── Blanket impl for AgentOrchestrator ──────────────────────────────────────

#[async_trait]
impl Orchestrator for crate::orchestrator::AgentOrchestrator {
    fn register(&self, config: AgentConfig) -> Result<AgentHandle, RuntimeError> {
        self.register(config)
    }

    async fn spawn(
        &self,
        config: AgentConfig,
        process_config: claw_pal::ProcessConfig,
    ) -> Result<AgentHandle, RuntimeError> {
        self.spawn(config, process_config).await
    }

    async fn terminate(
        &self,
        agent_id: &AgentId,
        grace_period: Duration,
    ) -> Result<(), RuntimeError> {
        self.terminate(agent_id, grace_period).await
    }

    async fn kill(&self, agent_id: &AgentId) -> Result<(), RuntimeError> {
        self.kill(agent_id).await
    }

    async fn steer(&self, agent_id: &AgentId, command: SteerCommand) -> Result<(), RuntimeError> {
        self.steer(agent_id, command).await
    }

    async fn set_quota(
        &self,
        agent_id: &AgentId,
        quota: ResourceQuota,
    ) -> Result<(), RuntimeError> {
        self.set_quota(agent_id, quota).await
    }

    async fn health_check(&self, agent_id: &AgentId) -> Result<HealthStatus, RuntimeError> {
        self.health_check(agent_id).await
    }

    fn unregister(&self, agent_id: &AgentId, reason: &str) -> Result<(), RuntimeError> {
        self.unregister(agent_id, reason)
    }

    fn list_agents(&self) -> Vec<AgentId> {
        self.agent_ids()
    }

    fn agent_status(&self, agent_id: &AgentId) -> Result<AgentStatus, RuntimeError> {
        self.agent_info(agent_id)
            .map(|info| info.status)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.0.clone()))
    }

    fn agent_count(&self) -> usize {
        self.agent_count()
    }

    fn subscribe(&self) -> EventReceiver {
        self.subscribe()
    }
}
