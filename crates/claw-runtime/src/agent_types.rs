use claw_pal::{ExecutionMode, ProcessHandle};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── AgentId ──────────────────────────────────────────────────────────────────

/// Unique agent identifier (UUID-style string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct AgentId(pub String);

impl AgentId {
    /// Create an `AgentId` from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate a new unique `AgentId` using a cryptographically secure random token.
    pub fn generate() -> Self {
        use rand::Rng;
        let random: u128 = rand::thread_rng().gen();
        Self(format!("agent-{:032x}", random))
    }

    /// Return the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ─── AgentConfig ──────────────────────────────────────────────────────────────

/// Agent configuration used when registering with the orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub agent_id: AgentId,
    pub name: String,
    pub mode: ExecutionMode,
    pub metadata: HashMap<String, String>,
}

impl AgentConfig {
    /// Create a new `AgentConfig` with auto-generated ID and safe mode.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            agent_id: AgentId::generate(),
            name: name.into(),
            mode: ExecutionMode::Safe,
            metadata: HashMap::new(),
        }
    }

    /// Builder-style method to set the execution mode.
    pub fn with_mode(mut self, mode: ExecutionMode) -> Self {
        self.mode = mode;
        self
    }

    /// Builder-style method to insert a metadata key-value pair.
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

// ─── ResourceUsage ────────────────────────────────────────────────────────────

/// Snapshot of an agent's resource consumption.
///
/// Fields are `Option` to reflect platform limitations:
/// - `cpu_ms`: cumulative CPU time is only available for out-of-process agents
///   via OS-level proc accounting; `None` for in-process tokio task agents.
/// - `memory_bytes`: resident-set size estimate from the OS; `None` when the
///   agent shares the kernel process's address space (tokio task mode), since
///   per-task isolation is not available.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceUsage {
    /// Accumulated CPU time in milliseconds.  `None` for tokio-task agents.
    pub cpu_ms: Option<u64>,
    /// Resident memory estimate in bytes.  `None` when process isolation is unavailable.
    pub memory_bytes: Option<u64>,
}

// ─── AgentInfo ────────────────────────────────────────────────────────────────

/// Lifecycle status of a registered agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    /// Agent is starting up.
    Starting,
    /// Agent is running normally.
    Running,
    /// Agent is paused.
    Paused,
    /// Agent has stopped.
    Stopped,
    /// Agent encountered an error.
    Error,
}

impl Default for AgentStatus {
    fn default() -> Self {
        Self::Running
    }
}

/// Runtime information about a registered agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub config: AgentConfig,
    /// Unix timestamp in milliseconds when the agent was registered.
    pub started_at: u64,
    /// Process handle if agent is running in a separate OS process.
    pub process_handle: Option<ProcessHandle>,
    /// Current lifecycle status.
    pub status: AgentStatus,
    /// Latest resource consumption snapshot.  `None` before the first health
    /// check completes or for agents that do not expose OS-level metrics.
    pub resource_usage: Option<ResourceUsage>,
}

// ─── AgentHandle ─────────────────────────────────────────────────────────────

/// Handle for interacting with a running agent via its EventBus.
#[derive(Debug, Clone)]
pub struct AgentHandle {
    pub agent_id: AgentId,
    pub event_bus: crate::event_bus::EventBus,
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── test_agent_id_new_and_generate ───────────────────────────────────────
    #[test]
    fn test_agent_id_new_and_generate() {
        let id1 = AgentId::new("my-agent");
        assert_eq!(id1.as_str(), "my-agent");

        let id2 = AgentId::generate();
        assert!(id2.as_str().starts_with("agent-"));
        assert!(id2.as_str().len() > "agent-".len());

        // Two generated IDs should be different (at least on typical hardware).
        let id3 = AgentId::generate();
        // We cannot guarantee uniqueness in a unit test, but we can check format.
        assert!(id3.as_str().starts_with("agent-"));
    }

    // ── test_agent_config_default_mode_is_safe ───────────────────────────────
    #[test]
    fn test_agent_config_default_mode_is_safe() {
        let config = AgentConfig::new("test-agent");
        assert_eq!(config.mode, ExecutionMode::Safe);
        assert_eq!(config.name, "test-agent");
        assert!(config.metadata.is_empty());
    }

    // ── test_agent_config_builder ─────────────────────────────────────────────
    #[test]
    fn test_agent_config_builder() {
        let config = AgentConfig::new("builder-agent")
            .with_mode(ExecutionMode::Power)
            .with_meta("region", "us-east");

        assert_eq!(config.mode, ExecutionMode::Power);
        assert_eq!(
            config.metadata.get("region").map(|s| s.as_str()),
            Some("us-east")
        );
    }

    // ── test_agent_id_equality ───────────────────────────────────────────────
    #[test]
    fn test_agent_id_equality() {
        let a = AgentId::new("abc");
        let b = AgentId::new("abc");
        let c = AgentId::new("xyz");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
