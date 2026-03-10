use crate::a2a::protocol::A2AMessage;
use crate::agent_types::AgentId;
use crate::extension::ExtensionEvent;
use serde::{Deserialize, Serialize};

/// System-wide lifecycle events broadcast over the EventBus.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Event {
    // ── Agent lifecycle ──────────────────────────────────────────────────────
    AgentStarted {
        agent_id: AgentId,
    },
    AgentStopped {
        agent_id: AgentId,
        reason: String,
    },

    // ── LLM interaction ──────────────────────────────────────────────────────
    LlmRequestStarted {
        agent_id: AgentId,
        provider: String,
    },
    LlmRequestCompleted {
        agent_id: AgentId,
        prompt_tokens: u64,
        completion_tokens: u64,
    },

    // ── Message handling ────────────────────────────────────────────────────
    MessageReceived {
        agent_id: AgentId,
        channel: String,
        message_type: String,
    },

    // ── Agent-to-Agent messaging ─────────────────────────────────────────────
    /// Emitted when an A2A message is received via IPC.
    A2A(A2AMessage),

    // ── Tool usage ───────────────────────────────────────────────────────────
    ToolCalled {
        agent_id: AgentId,
        tool_name: String,
        call_id: String,
    },
    ToolResult {
        agent_id: AgentId,
        tool_name: String,
        call_id: String,
        success: bool,
    },

    // ── Memory system ────────────────────────────────────────────────────────
    /// Emitted when agent history approaches context limit.
    /// claw-loop 通过回调闭包触发，最终发布到 EventBus。
    ContextWindowApproachingLimit {
        agent_id: AgentId,
        token_count: u64,
        token_limit: u64,
    },
    /// Emitted by MemoryWorker after successfully archiving to SQLite.
    MemoryArchiveComplete {
        agent_id: AgentId,
        archived_count: usize,
    },

    // ── Security ─────────────────────────────────────────────────────────────
    ModeChanged {
        agent_id: AgentId,
        to_power_mode: bool,
    },

    // ── Extension ────────────────────────────────────────────────────────────
    /// Emitted by the extension subsystem (tool hot-loading, script reloads,
    /// provider registration).
    Extension(ExtensionEvent),

    // ── Agent restart ────────────────────────────────────────────────────────
    /// Emitted when an agent is restarted by the auto-restart mechanism.
    AgentRestarted {
        agent_id: AgentId,
        /// Which retry attempt this is (1-indexed).
        attempt: u32,
        /// Delay that was waited before this restart.
        delay_ms: u64,
    },
    /// Emitted when an agent has exhausted all restart attempts.
    AgentFailed {
        agent_id: AgentId,
        /// Total number of restart attempts that were made.
        attempts: u32,
        /// Human-readable reason for the final failure.
        reason: String,
    },

    // ── System ───────────────────────────────────────────────────────────────
    Shutdown,

    // ── Custom (from scripts) ─────────────────────────────────────────────────
    /// Emitted by scripts via the events bridge.
    Custom {
        event_type: String,
        data: serde_json::Value,
    },
}
