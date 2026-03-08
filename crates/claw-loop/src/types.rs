use claw_provider::types::{Message, TokenUsage, ToolCall};
use serde::{Deserialize, Serialize};

/// Configuration for an agent loop execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLoopConfig {
    /// Maximum number of turns (LLM + tool calls each count as one turn).
    pub max_turns: u32,
    /// Maximum total tokens across the session (0 = unlimited).
    pub token_budget: u64,
    /// The agent's persona / system prompt override (if any).
    pub system_prompt: Option<String>,
    /// Whether to enable tool use. Default: true.
    pub tool_use_enabled: bool,
    /// Tool execution timeout in seconds. Default: 30.
    pub tool_timeout_seconds: u64,
    /// Maximum number of tool calls per turn. Default: 10.
    pub max_tool_calls_per_turn: usize,
    /// Whether to enable streaming responses. Default: false.
    pub enable_streaming: bool,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_turns: 20,
            token_budget: 0,
            system_prompt: None,
            tool_use_enabled: true,
            tool_timeout_seconds: 30,
            max_tool_calls_per_turn: 10,
            enable_streaming: false,
        }
    }
}

impl AgentLoopConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_turns(mut self, n: u32) -> Self {
        self.max_turns = n;
        self
    }

    pub fn with_token_budget(mut self, budget: u64) -> Self {
        self.token_budget = budget;
        self
    }

    pub fn with_system(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_tool_timeout_seconds(mut self, seconds: u64) -> Self {
        self.tool_timeout_seconds = seconds;
        self
    }

    pub fn with_max_tool_calls_per_turn(mut self, max: usize) -> Self {
        self.max_tool_calls_per_turn = max;
        self
    }

    pub fn with_enable_streaming(mut self, enabled: bool) -> Self {
        self.enable_streaming = enabled;
        self
    }
}

/// Why the agent loop finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// LLM returned finish_reason = stop with no tool calls.
    Stop,
    /// Reached max_turns limit.
    MaxTurns,
    /// Exceeded token_budget.
    TokenBudget,
    /// LLM made a response without tool calls (stop condition).
    NoToolCall,
    /// Custom stop condition triggered.
    StopCondition,
    /// Loop stopped due to error.
    Error,
}

/// Current state of the agent loop (snapshot).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopState {
    /// Current turn number (0-indexed).
    pub turn: u32,
    /// Cumulative token usage.
    pub usage: TokenUsage,
    /// Current history length (number of messages).
    pub history_len: usize,
}

impl LoopState {
    pub fn new() -> Self {
        Self {
            turn: 0,
            usage: TokenUsage::default(),
            history_len: 0,
        }
    }
}

impl Default for LoopState {
    fn default() -> Self {
        Self::new()
    }
}

/// Final result returned by a completed agent loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    /// Why the loop finished.
    pub finish_reason: FinishReason,
    /// The last assistant message (None if loop errored before any response).
    pub last_message: Option<Message>,
    /// Total token usage.
    pub usage: TokenUsage,
    /// Total turns executed.
    pub turns: u32,
    /// Final text content (convenience for last_message.as_ref().map(|m| m.content.clone()).unwrap_or_default()).
    pub content: String,
    /// All tool calls executed across all turns.
    pub tool_calls: Vec<ToolCall>,
    /// Total execution time in milliseconds.
    pub execution_time_ms: u64,
}

/// A chunk yielded by [`AgentLoop::stream_run`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamChunk {
    /// Text delta from the LLM.
    Text { content: String, is_final: bool },
    /// Token usage update.
    UsageUpdate(TokenUsage),
    /// Loop finished.
    Finish(FinishReason),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_loop_config_default() {
        let cfg = AgentLoopConfig::default();
        assert_eq!(cfg.max_turns, 20);
        assert_eq!(cfg.token_budget, 0);
        assert!(cfg.system_prompt.is_none());
        assert!(cfg.tool_use_enabled);
        assert_eq!(cfg.tool_timeout_seconds, 30);
        assert_eq!(cfg.max_tool_calls_per_turn, 10);
        assert!(!cfg.enable_streaming);
    }

    #[test]
    fn test_agent_loop_config_builder() {
        let cfg = AgentLoopConfig::new()
            .with_max_turns(50)
            .with_token_budget(100_000)
            .with_system("You are a coding assistant.")
            .with_tool_timeout_seconds(60)
            .with_max_tool_calls_per_turn(5)
            .with_enable_streaming(true);

        assert_eq!(cfg.max_turns, 50);
        assert_eq!(cfg.token_budget, 100_000);
        assert_eq!(
            cfg.system_prompt,
            Some("You are a coding assistant.".to_string())
        );
        assert!(cfg.tool_use_enabled);
        assert_eq!(cfg.tool_timeout_seconds, 60);
        assert_eq!(cfg.max_tool_calls_per_turn, 5);
        assert!(cfg.enable_streaming);
    }

    #[test]
    fn test_agent_loop_config_new_fields() {
        let config = AgentLoopConfig::new()
            .with_max_turns(50)
            .with_tool_timeout_seconds(30)
            .with_max_tool_calls_per_turn(10)
            .with_enable_streaming(false);

        assert_eq!(config.max_turns, 50);
        assert_eq!(config.tool_timeout_seconds, 30);
        assert_eq!(config.max_tool_calls_per_turn, 10);
        assert!(!config.enable_streaming);
    }

    #[test]
    fn test_loop_state_new() {
        let state = LoopState::new();
        assert_eq!(state.turn, 0);
        assert_eq!(state.history_len, 0);
        assert_eq!(state.usage.total_tokens, 0);
        assert_eq!(state.usage.prompt_tokens, 0);
        assert_eq!(state.usage.completion_tokens, 0);

        let state2 = LoopState::default();
        assert_eq!(state2.turn, 0);
        assert_eq!(state2.history_len, 0);
    }

    #[test]
    fn test_finish_reason_variants() {
        // Verify all variants exist and PartialEq works correctly.
        assert_eq!(FinishReason::Stop, FinishReason::Stop);
        assert_ne!(FinishReason::Stop, FinishReason::Error);

        let reasons = [
            FinishReason::Stop,
            FinishReason::MaxTurns,
            FinishReason::TokenBudget,
            FinishReason::NoToolCall,
            FinishReason::StopCondition,
            FinishReason::Error,
        ];
        // Every variant must be distinct from the others.
        for (i, a) in reasons.iter().enumerate() {
            for (j, b) in reasons.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }

        // Serialization round-trip.
        let json = serde_json::to_string(&FinishReason::MaxTurns).unwrap();
        assert_eq!(json, "\"max_turns\"");
        let back: FinishReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, FinishReason::MaxTurns);
    }
}
