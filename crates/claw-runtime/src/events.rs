use crate::agent_types::AgentId;
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

    // ── System ───────────────────────────────────────────────────────────────
    Shutdown,
}
