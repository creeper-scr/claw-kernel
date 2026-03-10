//! Core types for agent loop configuration, state, and results.
//!
//! This module defines the configuration and output types used by `AgentLoop`:
//! [`AgentLoopConfig`] controls runtime behaviour, [`LoopState`] captures
//! per-turn snapshots, [`AgentResult`] carries the completed-loop summary, and
//! [`StreamChunk`] is the item type for streaming mode.

use std::sync::Arc;

use claw_provider::traits::LLMProvider;
use claw_provider::types::{Message, TokenUsage, ToolCall};
use serde::{Deserialize, Serialize};

/// Policy for switching to fallback providers when the primary provider fails.
///
/// Used in [`AgentLoopConfig::failover_policy`] to control automatic failover.
///
/// # Examples
///
/// ```rust
/// use claw_loop::types::FailoverPolicy;
/// use std::time::Duration;
///
/// // Switch on any error
/// let policy = FailoverPolicy::OnError;
///
/// // Switch after 3 consecutive errors
/// let policy = FailoverPolicy::OnConsecutiveErrors(3);
///
/// // Switch when latency exceeds 5 seconds
/// let policy = FailoverPolicy::OnLatencyExceeds(Duration::from_secs(5));
/// ```
#[derive(Debug, Clone, Default)]
pub enum FailoverPolicy {
    /// Switch to the next fallback provider on any provider error.
    #[default]
    OnError,
    /// Switch after N consecutive errors from the current provider.
    OnConsecutiveErrors(u32),
    /// Switch when the provider's response latency exceeds the given threshold.
    OnLatencyExceeds(std::time::Duration),
}

/// Configuration for an agent loop execution.
#[derive(Clone, Serialize, Deserialize)]
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
    /// Ordered list of fallback providers to try when the primary fails.
    ///
    /// Providers are tried in order. If all providers fail, the last error
    /// is returned. When empty (the default), no failover occurs.
    #[serde(skip)]
    pub fallback_providers: Vec<Arc<dyn LLMProvider>>,
    /// Policy controlling when to switch to the next fallback provider.
    ///
    /// Only used when [`fallback_providers`](Self::fallback_providers) is non-empty.
    /// Defaults to [`FailoverPolicy::OnError`].
    #[serde(skip)]
    pub failover_policy: FailoverPolicy,
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
            fallback_providers: Vec::new(),
            failover_policy: FailoverPolicy::default(),
        }
    }
}

impl std::fmt::Debug for AgentLoopConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentLoopConfig")
            .field("max_turns", &self.max_turns)
            .field("token_budget", &self.token_budget)
            .field("system_prompt", &self.system_prompt)
            .field("tool_use_enabled", &self.tool_use_enabled)
            .field("tool_timeout_seconds", &self.tool_timeout_seconds)
            .field("max_tool_calls_per_turn", &self.max_tool_calls_per_turn)
            .field("enable_streaming", &self.enable_streaming)
            .field(
                "fallback_providers",
                &format!("[{} provider(s)]", self.fallback_providers.len()),
            )
            .field("failover_policy", &self.failover_policy)
            .finish()
    }
}

impl AgentLoopConfig {
    /// Create a new config with default values.
    ///
    /// # Defaults
    ///
    /// - `max_turns`: 20
    /// - `token_budget`: 0 (unlimited)
    /// - `system_prompt`: None
    /// - `tool_use_enabled`: true
    /// - `tool_timeout_seconds`: 30
    /// - `max_tool_calls_per_turn`: 10
    /// - `enable_streaming`: false
    /// - `fallback_providers`: empty (no failover)
    /// - `failover_policy`: [`FailoverPolicy::OnError`]
    ///
    /// # Example
    ///
    /// ```
    /// use claw_loop::types::AgentLoopConfig;
    ///
    /// let config = AgentLoopConfig::new();
    /// assert_eq!(config.max_turns, 20);
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum number of turns before the loop stops.
    ///
    /// Each turn consists of one LLM call and any resulting tool executions.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_loop::types::AgentLoopConfig;
    ///
    /// let config = AgentLoopConfig::new().with_max_turns(50);
    /// assert_eq!(config.max_turns, 50);
    /// ```
    pub fn with_max_turns(mut self, n: u32) -> Self {
        self.max_turns = n;
        self
    }

    /// Set the maximum token budget for the session (0 = unlimited).
    ///
    /// When the cumulative token usage exceeds this budget, the loop stops
    /// with `FinishReason::TokenBudget`.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_loop::types::AgentLoopConfig;
    ///
    /// let config = AgentLoopConfig::new().with_token_budget(100_000);
    /// assert_eq!(config.token_budget, 100_000);
    /// ```
    pub fn with_token_budget(mut self, budget: u64) -> Self {
        self.token_budget = budget;
        self
    }

    /// Set the system prompt (overrides any default persona).
    ///
    /// The system prompt sets the behavior and context for the LLM throughout
    /// the entire conversation.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_loop::types::AgentLoopConfig;
    ///
    /// let config = AgentLoopConfig::new()
    ///     .with_system("You are a helpful coding assistant.");
    /// assert_eq!(config.system_prompt, Some("You are a helpful coding assistant.".to_string()));
    /// ```
    pub fn with_system(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set the per-tool-call execution timeout in seconds.
    ///
    /// If a tool takes longer than this to execute, it will be cancelled
    /// and return a timeout error.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_loop::types::AgentLoopConfig;
    ///
    /// let config = AgentLoopConfig::new().with_tool_timeout_seconds(60);
    /// assert_eq!(config.tool_timeout_seconds, 60);
    /// ```
    pub fn with_tool_timeout_seconds(mut self, seconds: u64) -> Self {
        self.tool_timeout_seconds = seconds;
        self
    }

    /// Set the maximum number of tool calls allowed per turn.
    ///
    /// This prevents runaway tool execution loops. When exceeded,
    /// the loop stops with an error.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_loop::types::AgentLoopConfig;
    ///
    /// let config = AgentLoopConfig::new().with_max_tool_calls_per_turn(5);
    /// assert_eq!(config.max_tool_calls_per_turn, 5);
    /// ```
    pub fn with_max_tool_calls_per_turn(mut self, max: usize) -> Self {
        self.max_tool_calls_per_turn = max;
        self
    }

    /// Enable or disable streaming responses from the LLM.
    ///
    /// When enabled, use `AgentLoop::stream_run()` to receive partial
    /// responses as they are generated.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_loop::types::AgentLoopConfig;
    ///
    /// let config = AgentLoopConfig::new().with_enable_streaming(true);
    /// assert!(config.enable_streaming);
    /// ```
    pub fn with_enable_streaming(mut self, enabled: bool) -> Self {
        self.enable_streaming = enabled;
        self
    }

    /// Set the ordered list of fallback providers for automatic failover.
    ///
    /// When the primary provider encounters an error (or meets the configured
    /// [`FailoverPolicy`]), the agent loop tries each fallback in order.
    /// If all providers fail, the last error is returned.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use claw_loop::types::AgentLoopConfig;
    /// use claw_provider::OllamaProvider;
    /// use std::sync::Arc;
    ///
    /// let fallback = Arc::new(OllamaProvider::new("llama3.2:latest").with_base_url("http://backup:11434"));
    /// let config = AgentLoopConfig::new().with_fallback_providers(vec![fallback]);
    /// ```
    pub fn with_fallback_providers(
        mut self,
        providers: Vec<Arc<dyn LLMProvider>>,
    ) -> Self {
        self.fallback_providers = providers;
        self
    }

    /// Set the failover policy (controls when to switch to the next provider).
    ///
    /// Only meaningful when [`fallback_providers`](Self::fallback_providers) is non-empty.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claw_loop::types::{AgentLoopConfig, FailoverPolicy};
    ///
    /// let config = AgentLoopConfig::new()
    ///     .with_failover_policy(FailoverPolicy::OnConsecutiveErrors(3));
    /// ```
    pub fn with_failover_policy(mut self, policy: FailoverPolicy) -> Self {
        self.failover_policy = policy;
        self
    }
}

/// Why the agent loop finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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
    /// Create a zeroed initial loop state.
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
///
/// Returned by `AgentLoop::run()`. For streaming mode, the final `StreamChunk::Finish`
/// variant carries the `FinishReason`; build an `AgentResult` from the accumulated chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    /// Why the loop finished.
    pub finish_reason: FinishReason,
    /// The last assistant message (None if loop errored before any response).
    pub last_message: Option<Message>,
    /// Total token usage across all turns.
    pub usage: TokenUsage,
    /// Total turns executed.
    pub turns: u32,
    /// Convenience accessor for the final assistant text; equivalent to
    /// `last_message.as_ref().map(|m| m.content.clone()).unwrap_or_default()`.
    pub content: String,
    /// All tool calls executed across all turns, in order.
    pub tool_calls: Vec<ToolCall>,
    /// Wall-clock execution time for the entire loop in milliseconds.
    pub execution_time_ms: u64,
}

/// A chunk yielded by `AgentLoop::stream_run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamChunk {
    /// Text delta from the LLM.
    Text { content: String, is_final: bool },
    /// Tool call started (id + name available).
    ToolStart { id: String, name: String },
    /// Streaming tool arguments (accumulate across chunks).
    ToolArguments { id: String, arguments: String },
    /// Tool call completed with result.
    ToolComplete { id: String, result: serde_json::Value },
    /// Tool call failed.
    ToolError { id: String, error: String },
    /// Token usage update.
    UsageUpdate(TokenUsage),
    /// Loop finished.
    Finish(FinishReason),
    /// Error occurred during streaming.
    Error(String),
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
