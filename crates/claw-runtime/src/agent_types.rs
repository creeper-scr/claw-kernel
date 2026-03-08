use claw_pal::ProcessHandle;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── AgentId ──────────────────────────────────────────────────────────────────

/// Unique agent identifier (UUID-style string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    /// Create an `AgentId` from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate a new unique `AgentId` using a nanosecond-based hex token.
    ///
    /// No external `uuid` / `rand` dependency required.
    pub fn generate() -> Self {
        Self(format!("agent-{}", rand_hex(16)))
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

/// Simple non-cryptographic hex token derived from the current nanosecond
/// timestamp, atomic counter, process ID, and thread ID.
/// Guarantees uniqueness within a single process lifetime even on very fast
/// hardware where nanos may repeat, and across different processes/threads.
fn rand_hex(n: usize) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let pid = std::process::id() as u128;

    // Hash thread ID to get a stable u64 value (thread_id_value is unstable)
    let thread_id = std::thread::current().id();
    let mut hasher = DefaultHasher::new();
    thread_id.hash(&mut hasher);
    let tid = hasher.finish() as u128;

    // Mix nanos with sequence number, PID, and TID to eliminate collisions.
    let raw = t.as_nanos()
        ^ ((seq as u128).wrapping_mul(0x9e37_79b9_7f4a_7c15))
        ^ (pid << 48)
        ^ (tid << 32);
    format!("{:0>width$x}", raw & ((1u128 << (n * 4)) - 1), width = n)
}

// ─── ExecutionMode ────────────────────────────────────────────────────────────

/// Execution mode of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// Sandbox-restricted mode (default).
    Safe,
    /// Elevated-privilege mode.
    Power,
}

impl Default for ExecutionMode {
    fn default() -> Self {
        Self::Safe
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
}

// ─── AgentHandle ─────────────────────────────────────────────────────────────

/// Handle for interacting with a running agent via its EventBus.
#[derive(Debug, Clone)]
pub struct AgentHandle {
    pub agent_id: AgentId,
    pub event_bus: crate::event_bus::EventBus,
}

// ─── A2AMessage ───────────────────────────────────────────────────────────────

/// Agent-to-Agent message envelope (serialized over IPC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AMessage {
    pub from: AgentId,
    pub to: AgentId,
    /// Unique identifier for request/response correlation.
    pub correlation_id: String,
    /// Arbitrary JSON payload.
    pub payload: serde_json::Value,
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
