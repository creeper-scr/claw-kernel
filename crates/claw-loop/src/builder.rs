use std::sync::Arc;

use claw_provider::traits::LLMProvider;
use claw_tools::registry::ToolRegistry;

use crate::{
    agent_loop::AgentLoop,
    error::AgentError,
    history::InMemoryHistory,
    traits::{HistoryManager, StopCondition},
    types::AgentLoopConfig,
};

/// Fluent builder for [`AgentLoop`].
///
/// # Required fields
/// - `provider` — must be set before calling [`build`](AgentLoopBuilder::build).
///
/// # Optional fields
/// All other fields fall back to sensible defaults.
pub struct AgentLoopBuilder {
    provider: Option<Arc<dyn LLMProvider>>,
    tools: Option<Arc<ToolRegistry>>,
    history: Box<dyn HistoryManager>,
    stop_conditions: Vec<Box<dyn StopCondition>>,
    config: AgentLoopConfig,
}

impl AgentLoopBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self {
            provider: None,
            tools: None,
            history: Box::new(InMemoryHistory::default()),
            stop_conditions: Vec::new(),
            config: AgentLoopConfig::default(),
        }
    }

    /// Set the LLM provider (required).
    pub fn with_provider(mut self, provider: Arc<dyn LLMProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Attach a tool registry (optional; tool calls are skipped when absent).
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Replace the default [`InMemoryHistory`] with a custom implementation.
    pub fn with_history(mut self, history: Box<dyn HistoryManager>) -> Self {
        self.history = history;
        self
    }

    /// Add a custom stop condition.
    pub fn with_stop_condition(mut self, cond: Box<dyn StopCondition>) -> Self {
        self.stop_conditions.push(cond);
        self
    }

    /// Replace the entire [`AgentLoopConfig`].
    pub fn with_config(mut self, config: AgentLoopConfig) -> Self {
        self.config = config;
        self
    }

    /// Convenience: set `config.max_turns`.
    pub fn with_max_turns(mut self, n: u32) -> Self {
        self.config.max_turns = n;
        self
    }

    /// Convenience: set `config.system_prompt`.
    pub fn with_system_prompt(mut self, sys: impl Into<String>) -> Self {
        self.config.system_prompt = Some(sys.into());
        self
    }

    /// Convenience: set `config.token_budget`.
    pub fn with_token_budget(mut self, budget: u64) -> Self {
        self.config.token_budget = budget;
        self
    }

    /// Build the [`AgentLoop`].
    ///
    /// Returns `Err(AgentError::Context)` if no provider was set.
    pub fn build(self) -> Result<AgentLoop, AgentError> {
        let provider = self
            .provider
            .ok_or_else(|| AgentError::Context("no provider set".to_string()))?;

        Ok(AgentLoop {
            provider,
            tools: self.tools,
            history: self.history,
            stop_conditions: self.stop_conditions,
            config: self.config,
        })
    }
}

impl Default for AgentLoopBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use claw_provider::{
        error::ProviderError,
        traits::LLMProvider,
        types::{CompletionResponse, Delta, FinishReason, Message, Options, TokenUsage},
    };
    use futures::stream;
    use std::pin::Pin;

    struct MockProvider;

    #[async_trait]
    impl LLMProvider for MockProvider {
        fn provider_id(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            "mock-v1"
        }

        async fn complete(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                id: "id".to_string(),
                model: "mock-v1".to_string(),
                message: Message::assistant("ok"),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage {
                    prompt_tokens: 5,
                    completion_tokens: 3,
                    total_tokens: 8,
                },
            })
        }

        async fn complete_stream(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<
            Pin<Box<dyn futures::Stream<Item = Result<Delta, ProviderError>> + Send>>,
            ProviderError,
        > {
            Ok(Box::pin(stream::empty()))
        }
    }

    fn mock_provider() -> Arc<dyn LLMProvider> {
        Arc::new(MockProvider)
    }

    // ── test_builder_requires_provider ───────────────────────────────────────

    #[test]
    fn test_builder_requires_provider() {
        let result = AgentLoopBuilder::new().build();
        assert!(result.is_err(), "build without provider should return Err");
        // Extract the error without requiring T: Debug on AgentLoop.
        let err = result.err().expect("expected an Err");
        match err {
            AgentError::Context(msg) => {
                assert!(
                    msg.contains("no provider"),
                    "error message should mention provider, got: {msg}"
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    // ── test_builder_with_max_turns ───────────────────────────────────────────

    #[test]
    fn test_builder_with_max_turns() {
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_max_turns(42)
            .build()
            .expect("build should succeed");

        assert_eq!(agent.config.max_turns, 42);
    }

    // ── test_builder_auto_adds_stop_conditions ────────────────────────────────

    #[test]
    fn test_builder_auto_adds_stop_conditions() {
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_max_turns(10)
            .with_token_budget(5000)
            .build()
            .expect("build should succeed");

        // Builder no longer auto-adds stop conditions — default is empty.
        assert_eq!(
            agent.stop_conditions.len(),
            0,
            "builder should not auto-add any stop conditions by default, got {}",
            agent.stop_conditions.len()
        );
    }
}
