use std::sync::Arc;

use claw_provider::{
    traits::LLMProvider,
    types::{Message, Options},
};
use claw_tools::{
    registry::ToolRegistry,
    types::{PermissionSet, ToolContext},
};

use crate::{
    error::AgentError,
    traits::{HistoryManager, StopCondition},
    types::{AgentLoopConfig, AgentResult, FinishReason, LoopState},
};

/// The main agent loop engine.
///
/// # Loop algorithm
///
/// 1. Append the initial user message to history.
/// 2. Check custom stop conditions against the current `LoopState`.
/// 3. Check `config.max_turns` / `config.token_budget`.
/// 4. Call `LLMProvider::complete` with the full history.
/// 5. Append the assistant response to history; update usage counters.
/// 6. If the response contains tool calls **and** `tool_use_enabled` is set:
///    - Execute each tool via `ToolRegistry`.
///    - Append each tool result to history.
///    - `continue` the loop (go back to step 2).
/// 7. If there are no tool calls (pure text reply): stop with `FinishReason::Stop`.
pub struct AgentLoop {
    pub(crate) provider: Arc<dyn LLMProvider>,
    pub(crate) tools: Option<Arc<ToolRegistry>>,
    pub(crate) history: Box<dyn HistoryManager>,
    pub(crate) stop_conditions: Vec<Box<dyn StopCondition>>,
    pub(crate) config: AgentLoopConfig,
}

impl AgentLoop {
    /// Run the agent loop beginning with `initial_message`.
    ///
    /// Returns an [`AgentResult`] describing why the loop finished.
    pub async fn run(
        &mut self,
        initial_message: impl Into<String>,
    ) -> Result<AgentResult, AgentError> {
        // ── Step 1: seed the history with the user's opening message ──────────
        self.history.append(Message::user(initial_message));

        let mut state = LoopState::new();

        loop {
            // ── Step 2: custom stop conditions ────────────────────────────────
            for cond in &self.stop_conditions {
                if cond.should_stop(&state) {
                    return Ok(AgentResult {
                        finish_reason: FinishReason::StopCondition,
                        last_message: self.history.messages().last().cloned(),
                        usage: state.usage,
                        turns: state.turn,
                    });
                }
            }

            // ── Step 3a: max_turns guard ──────────────────────────────────────
            if state.turn >= self.config.max_turns {
                return Ok(AgentResult {
                    finish_reason: FinishReason::MaxTurns,
                    last_message: self.history.messages().last().cloned(),
                    usage: state.usage,
                    turns: state.turn,
                });
            }

            // ── Step 3b: token_budget guard ───────────────────────────────────
            if self.config.token_budget > 0 && state.usage.total_tokens >= self.config.token_budget
            {
                return Ok(AgentResult {
                    finish_reason: FinishReason::TokenBudget,
                    last_message: self.history.messages().last().cloned(),
                    usage: state.usage,
                    turns: state.turn,
                });
            }

            // ── Step 4: build options and call the LLM ────────────────────────
            let mut options =
                Options::new(self.provider.model_id().to_string()).with_max_tokens(4096);
            if let Some(sys) = &self.config.system_prompt {
                options = options.with_system(sys.clone());
            }

            let messages: Vec<Message> = self.history.messages().to_vec();
            let response = self
                .provider
                .complete(messages, options)
                .await
                .map_err(|e| AgentError::Provider(e.to_string()))?;

            // ── Step 5: update state ──────────────────────────────────────────
            state.usage.prompt_tokens += response.usage.prompt_tokens;
            state.usage.completion_tokens += response.usage.completion_tokens;
            state.usage.total_tokens += response.usage.total_tokens;
            state.turn += 1;

            // Post-LLM token-budget check (catches the newly added tokens).
            if self.config.token_budget > 0 && state.usage.total_tokens >= self.config.token_budget
            {
                return Ok(AgentResult {
                    finish_reason: FinishReason::TokenBudget,
                    last_message: Some(response.message),
                    usage: state.usage,
                    turns: state.turn,
                });
            }

            let assistant_msg = response.message.clone();
            self.history.append(response.message.clone());
            state.history_len = self.history.len();

            // ── Step 6: tool call handling ────────────────────────────────────
            let has_tool_calls = assistant_msg
                .tool_calls
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false);

            if has_tool_calls && self.config.tool_use_enabled {
                if let Some(ref registry) = self.tools {
                    let tool_calls = assistant_msg.tool_calls.as_ref().unwrap();
                    for call in tool_calls {
                        let args: serde_json::Value = serde_json::from_str(&call.arguments)
                            .unwrap_or(serde_json::Value::Null);

                        let ctx = ToolContext::new("agent", PermissionSet::minimal());

                        let result = registry
                            .execute(&call.name, args, ctx)
                            .await
                            .map_err(|e| AgentError::Tool(e.to_string()))?;

                        let result_content = serde_json::to_string(&result.output)
                            .unwrap_or_else(|_| "null".to_string());

                        self.history
                            .append(Message::tool_result(call.id.clone(), result_content));
                    }
                    state.history_len = self.history.len();
                    continue; // Back to top of loop with tool results in history.
                }
            }

            // ── Step 7: no tool calls → natural stop ──────────────────────────
            return Ok(AgentResult {
                finish_reason: FinishReason::Stop,
                last_message: Some(assistant_msg),
                usage: state.usage,
                turns: state.turn,
            });
        }
    }

    /// Inspect the current conversation history.
    pub fn history(&self) -> &[Message] {
        self.history.messages()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{builder::AgentLoopBuilder, types::FinishReason};
    use async_trait::async_trait;
    use claw_provider::{
        error::ProviderError,
        traits::LLMProvider,
        types::{
            CompletionResponse, Delta, FinishReason as ProvFinishReason, Message, Options,
            TokenUsage,
        },
    };
    use futures::stream;
    use std::pin::Pin;

    // ── Mock provider that always responds with a plain text message ──────────

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
            messages: Vec<Message>,
            _opts: Options,
        ) -> Result<CompletionResponse, ProviderError> {
            let last_content = messages.last().map(|m| m.content.as_str()).unwrap_or("");
            Ok(CompletionResponse {
                id: "mock-resp".to_string(),
                model: "mock-v1".to_string(),
                message: Message::assistant(format!("Echo: {last_content}")),
                finish_reason: ProvFinishReason::Stop,
                usage: TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
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

    // ── test_agent_loop_run_simple ────────────────────────────────────────────

    /// A single-turn run: mock always returns no tool calls, so the loop stops
    /// immediately after the first LLM response with `FinishReason::Stop`.
    #[tokio::test]
    async fn test_agent_loop_run_simple() {
        let mut agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .build()
            .expect("build should succeed");

        let result = agent
            .run("Hello, agent!")
            .await
            .expect("run should succeed");

        assert_eq!(result.finish_reason, FinishReason::Stop);
        assert_eq!(result.turns, 1);
        assert!(result.last_message.is_some());
        assert!(result.usage.total_tokens > 0);

        // History should contain: [user, assistant]
        assert_eq!(agent.history().len(), 2);
    }

    // ── test_agent_loop_max_turns_stop ────────────────────────────────────────

    /// Set max_turns to 0 so the loop immediately hits the turn cap on the
    /// first iteration, before any LLM call is made.
    ///
    /// The inline `max_turns` guard in `AgentLoop::run` fires and returns
    /// `FinishReason::MaxTurns`.
    #[tokio::test]
    async fn test_agent_loop_max_turns_stop() {
        let mut agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_max_turns(0)
            .build()
            .expect("build should succeed");

        let result = agent
            .run("trigger max turns")
            .await
            .expect("run should succeed");

        assert_eq!(result.finish_reason, FinishReason::MaxTurns);
        assert_eq!(result.turns, 0);
    }
}
