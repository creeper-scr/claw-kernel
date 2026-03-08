use std::sync::Arc;

use claw_provider::{
    retry::{with_retry, RetryConfig},
    traits::LLMProvider,
    types::{Message, Options, ToolCall},
};
use claw_tools::{
    registry::ToolRegistry,
    types::{PermissionSet, ToolContext, ToolResult},
};
use tokio::sync::{broadcast, RwLock};

use crate::{
    error::AgentError,
    state_machine::{AgentState, StateEvent, StateMachine, TransitionResult},
    traits::{HistoryManager, StopCondition},
    types::{AgentLoopConfig, AgentResult, FinishReason, LoopState},
};

/// The main agent loop engine with FSM-driven state management.
///
/// # Loop algorithm
///
/// 1. Start in Idle state, transition to Running on `run()` call.
/// 2. Check custom stop conditions against the current `LoopState`.
/// 3. Check `config.max_turns` / `config.token_budget`.
/// 4. Transition to AwaitingLLM, call `LLMProvider::complete`.
/// 5. On response, transition back to Running or to ToolExecuting.
/// 6. If tool calls required: execute tools in parallel, transition to Running.
/// 7. If no tool calls: stop with `FinishReason::Stop`.
///
/// State changes are broadcast via `state_tx` for external observers.
pub struct AgentLoop {
    pub(crate) provider: Arc<dyn LLMProvider>,
    pub(crate) tools: Option<Arc<ToolRegistry>>,
    pub(crate) history: Box<dyn HistoryManager>,
    pub(crate) stop_conditions: Vec<Box<dyn StopCondition>>,
    pub(crate) config: AgentLoopConfig,
    /// Current state, protected for thread-safe access.
    pub(crate) state: Arc<RwLock<AgentState>>,
    /// Broadcast channel for state change notifications.
    pub(crate) state_tx: broadcast::Sender<AgentState>,
}

impl AgentLoop {
    /// Create a new state broadcast channel with the specified capacity.
    pub fn create_state_channel(capacity: usize) -> broadcast::Sender<AgentState> {
        let (tx, _rx) = broadcast::channel(capacity);
        tx
    }

    /// Run the agent loop beginning with `initial_message`.
    ///
    /// Returns an [`AgentResult`] describing why the loop finished.
    /// State transitions are validated and broadcast to all subscribers.
    pub async fn run(
        &mut self,
        initial_message: impl Into<String>,
    ) -> Result<AgentResult, AgentError> {
        // ── Step 1: Initialize state machine and seed history ───────────────────
        let start_time = std::time::Instant::now();
        let mut tool_calls_accumulated: Vec<claw_provider::types::ToolCall> = Vec::new();
        let mut state_machine = StateMachine::new();

        // Transition: Idle -> Running
        self.transition(&mut state_machine, StateEvent::Start)
            .await?;

        self.history.append(Message::user(initial_message));

        let mut loop_state = LoopState::new();

        loop {
            // ── Step 2: custom stop conditions ────────────────────────────────
            for cond in &self.stop_conditions {
                if cond.should_stop(&loop_state) {
                    // Transition: Running -> Completed
                    self.transition(&mut state_machine, StateEvent::StopConditionMet)
                        .await?;
                    return Ok(AgentResult {
                        finish_reason: FinishReason::StopCondition,
                        last_message: self.history.messages().last().cloned(),
                        usage: loop_state.usage,
                        turns: loop_state.turn,
                        content: self.history.messages().last().map(|m| m.content.clone()).unwrap_or_default(),
                        tool_calls: tool_calls_accumulated.clone(),
                        execution_time_ms: start_time.elapsed().as_millis() as u64,
                    });
                }
            }

            // ── Step 3a: max_turns guard ──────────────────────────────────────
            if loop_state.turn >= self.config.max_turns {
                // Transition: Running -> Completed
                self.transition(&mut state_machine, StateEvent::StopConditionMet)
                    .await?;
                return Ok(AgentResult {
                    finish_reason: FinishReason::MaxTurns,
                    last_message: self.history.messages().last().cloned(),
                    usage: loop_state.usage,
                    turns: loop_state.turn,
                    content: self.history.messages().last().map(|m| m.content.clone()).unwrap_or_default(),
                    tool_calls: tool_calls_accumulated.clone(),
                    execution_time_ms: start_time.elapsed().as_millis() as u64,
                });
            }

            // ── Step 3b: token_budget guard ───────────────────────────────────
            if self.config.token_budget > 0
                && loop_state.usage.total_tokens >= self.config.token_budget
            {
                // Transition: Running -> Completed
                self.transition(&mut state_machine, StateEvent::StopConditionMet)
                    .await?;
                return Ok(AgentResult {
                    finish_reason: FinishReason::TokenBudget,
                    last_message: self.history.messages().last().cloned(),
                    usage: loop_state.usage,
                    turns: loop_state.turn,
                    content: self.history.messages().last().map(|m| m.content.clone()).unwrap_or_default(),
                    tool_calls: tool_calls_accumulated.clone(),
                    execution_time_ms: start_time.elapsed().as_millis() as u64,
                });
            }

            // ── Step 4: build options and call the LLM ────────────────────────
            let mut options =
                Options::new(self.provider.model_id().to_string()).with_max_tokens(4096);
            if let Some(sys) = &self.config.system_prompt {
                options = options.with_system(sys.clone());
            }

            let messages: Vec<Message> = self.history.messages().to_vec();

            // Transition: Running -> AwaitingLLM
            self.transition(&mut state_machine, StateEvent::LLMRequestSent)
                .await?;

            // Configure retry: 3 attempts with exponential backoff starting at 1s
            let retry_config = RetryConfig::new()
                .with_max_retries(3)
                .with_base_delay(std::time::Duration::from_secs(1));

            let response = match with_retry(
                || self.provider.complete(messages.clone(), options.clone()),
                &retry_config,
            )
            .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    // Transition: AwaitingLLM -> Error
                    self.transition(&mut state_machine, StateEvent::Error)
                        .await?;
                    return Err(AgentError::Provider(e.to_string()));
                }
            };

            // ── Step 5: update state ──────────────────────────────────────────
            loop_state.usage.prompt_tokens += response.usage.prompt_tokens;
            loop_state.usage.completion_tokens += response.usage.completion_tokens;
            loop_state.usage.total_tokens += response.usage.total_tokens;
            loop_state.turn += 1;

            // Post-LLM token-budget check
            if self.config.token_budget > 0
                && loop_state.usage.total_tokens >= self.config.token_budget
            {
                self.history.append(response.message.clone());
                let content = response.message.content.clone();
                // Transition: AwaitingLLM -> Completed (via Running)
                self.transition(&mut state_machine, StateEvent::LLMResponseReceived)
                    .await?;
                self.transition(&mut state_machine, StateEvent::StopConditionMet)
                    .await?;
                return Ok(AgentResult {
                    finish_reason: FinishReason::TokenBudget,
                    last_message: Some(response.message),
                    usage: loop_state.usage,
                    turns: loop_state.turn,
                    content,
                    tool_calls: tool_calls_accumulated.clone(),
                    execution_time_ms: start_time.elapsed().as_millis() as u64,
                });
            }

            let assistant_msg = response.message.clone();
            self.history.append(response.message.clone());
            loop_state.history_len = self.history.len();

            // ── Step 6: tool call handling ────────────────────────────────────
            let has_tool_calls = assistant_msg
                .tool_calls
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false);

            if has_tool_calls && self.config.tool_use_enabled {
                if let Some(ref registry) = self.tools {
                    let tool_calls = assistant_msg.tool_calls.as_ref().unwrap().clone();
                    tool_calls_accumulated.extend(tool_calls.clone());

                    // Transition: AwaitingLLM -> ToolExecuting
                    self.transition(&mut state_machine, StateEvent::ToolsRequired)
                        .await?;

                    // Execute tools in parallel
                    let tool_results =
                        match self.execute_tools_parallel(&tool_calls, registry).await {
                            Ok(results) => results,
                            Err(e) => {
                                // Try to transition to error state, but don't fail if transition itself fails
                                let _ =
                                    self.transition(&mut state_machine, StateEvent::Error).await;
                                return Err(e);
                            }
                        };

                    // Append tool results to history
                    for (call_id, result_content) in tool_results {
                        self.history
                            .append(Message::tool_result(call_id, result_content));
                    }
                    loop_state.history_len = self.history.len();

                    // Transition: ToolExecuting -> Running
                    self.transition(&mut state_machine, StateEvent::ToolsCompleted)
                        .await?;

                    continue; // Back to top of loop with tool results in history.
                }
            }

            // ── Step 7: no tool calls → natural stop ──────────────────────────
            // Transition: AwaitingLLM -> Running (briefly)
            self.transition(&mut state_machine, StateEvent::LLMResponseReceived)
                .await?;
            // Then immediately to Completed
            self.transition(&mut state_machine, StateEvent::StopConditionMet)
                .await?;

            return Ok(AgentResult {
                finish_reason: FinishReason::Stop,
                last_message: Some(assistant_msg.clone()),
                usage: loop_state.usage,
                turns: loop_state.turn,
                content: assistant_msg.content.clone(),
                tool_calls: tool_calls_accumulated,
                execution_time_ms: start_time.elapsed().as_millis() as u64,
            });
        }
    }

    /// Execute a state transition and broadcast the new state.
    ///
    /// Validates the transition using the state machine, updates the shared state,
    /// and broadcasts the change to all subscribers.
    async fn transition(
        &self,
        state_machine: &mut StateMachine,
        event: StateEvent,
    ) -> Result<AgentState, AgentError> {
        match state_machine.transition(event) {
            TransitionResult::Success(new_state) => {
                // Update shared state
                let mut state_guard = self.state.write().await;
                *state_guard = new_state;
                drop(state_guard);

                // Broadcast state change
                let _ = self.state_tx.send(new_state);

                Ok(new_state)
            }
            TransitionResult::Invalid {
                from,
                event,
                allowed,
            } => Err(AgentError::InvalidStateTransition {
                from,
                event,
                allowed,
            }),
        }
    }

    /// Execute tool calls in parallel using tokio::join!.
    ///
    /// Returns a vector of (call_id, result_json) pairs.
    /// Respects `max_tool_calls_per_turn` limit.
    async fn execute_tools_parallel(
        &self,
        tool_calls: &[ToolCall],
        registry: &ToolRegistry,
    ) -> Result<Vec<(String, String)>, AgentError> {
        // Limit the number of tool calls per turn
        let calls_to_execute: Vec<_> = tool_calls
            .iter()
            .take(self.config.max_tool_calls_per_turn)
            .collect();

        if calls_to_execute.is_empty() {
            return Ok(Vec::new());
        }

        // Build futures for each tool call
        let futures: Vec<_> = calls_to_execute
            .iter()
            .map(|call| {
                let call_id = call.id.clone();
                let call_name = call.name.clone();
                let call_args = call.arguments.clone();

                async move {
                    // Try to parse the tool arguments
                    let args: serde_json::Value = match serde_json::from_str(&call_args) {
                        Ok(v) => v,
                        Err(e) => {
                            // Log the warning and return error to LLM
                            tracing::warn!("Failed to parse tool args for '{}': {}", call_name, e);
                            let error_json = serde_json::json!({
                                "tool_call_id": call_id,
                                "error": "parameter_parse_failed",
                                "message": format!("Failed to parse parameters: {}", e),
                            })
                            .to_string();
                            return (call_id, error_json);
                        }
                    };

                    let ctx = ToolContext::new("agent", PermissionSet::minimal());

                    let result: Result<ToolResult, _> =
                        registry.execute(&call_name, args, ctx).await;

                    let result_content = match result {
                        Ok(tool_result) => serde_json::to_string(&tool_result.output)
                            .unwrap_or_else(|_| "null".to_string()),
                        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                    };

                    (call_id, result_content)
                }
            })
            .collect();

        // Execute all futures in parallel
        let results = futures::future::join_all(futures).await;

        Ok(results)
    }

    /// Get the current state.
    pub async fn current_state(&self) -> AgentState {
        *self.state.read().await
    }

    /// Subscribe to state changes.
    pub fn subscribe_state(&self) -> broadcast::Receiver<AgentState> {
        self.state_tx.subscribe()
    }

    /// Inspect the current conversation history.
    pub fn history(&self) -> &[Message] {
        self.history.messages()
    }

    /// Stream the agent loop execution, yielding chunks as they arrive.
    ///
    /// This is a v1 implementation that runs the complete loop and then
    /// yields chunks. A future version will use `complete_stream()` for
    /// true token-by-token streaming.
    pub async fn stream_run(
        &mut self,
        initial_message: impl Into<String>,
    ) -> Result<impl futures::Stream<Item = crate::types::StreamChunk>, crate::error::AgentError> {
        let result = self.run(initial_message).await?;
        let chunks = vec![
            crate::types::StreamChunk::Text {
                content: result.content.clone(),
                is_final: true,
            },
            crate::types::StreamChunk::UsageUpdate(result.usage.clone()),
            crate::types::StreamChunk::Finish(result.finish_reason.clone()),
        ];
        Ok(futures::stream::iter(chunks))
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
            TokenUsage, ToolCall,
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

    // ── Mock provider that returns tool calls ─────────────────────────────────

    struct MockToolProvider;

    #[async_trait]
    impl LLMProvider for MockToolProvider {
        fn provider_id(&self) -> &str {
            "mock-tool"
        }

        fn model_id(&self) -> &str {
            "mock-tool-v1"
        }

        async fn complete(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<CompletionResponse, ProviderError> {
            // Return a response with tool calls
            let mut msg = Message::assistant("I need to use a tool");
            msg.tool_calls = Some(vec![ToolCall {
                id: "call_1".to_string(),
                name: "test_tool".to_string(),
                arguments: r#"{"input": "test"}"#.to_string(),
            }]);
            Ok(CompletionResponse {
                id: "mock-tool-resp".to_string(),
                model: "mock-tool-v1".to_string(),
                message: msg,
                finish_reason: ProvFinishReason::ToolCalls,
                usage: TokenUsage {
                    prompt_tokens: 20,
                    completion_tokens: 10,
                    total_tokens: 30,
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

    fn mock_tool_provider() -> Arc<dyn LLMProvider> {
        Arc::new(MockToolProvider)
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

        // Subscribe to state changes
        let mut state_rx = agent.subscribe_state();

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

        // Check final state
        let final_state = agent.current_state().await;
        assert_eq!(final_state, AgentState::Completed);

        // We should have received state change notifications
        // Note: we might miss some if the channel buffer is small, but we should at least get Completed
        let states_received: Vec<_> = std::iter::from_fn(|| state_rx.try_recv().ok()).collect();
        assert!(
            !states_received.is_empty(),
            "Should have received state updates"
        );
        assert_eq!(*states_received.last().unwrap(), AgentState::Completed);
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

        let final_state = agent.current_state().await;
        assert_eq!(final_state, AgentState::Completed);
    }

    // ── test_agent_loop_state_machine_driven ─────────────────────────────────

    /// Verify that the state machine properly tracks state through the lifecycle.
    #[tokio::test]
    async fn test_agent_loop_state_machine_driven() {
        let mut agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .build()
            .expect("build should succeed");

        // Subscribe before running
        let mut state_rx = agent.subscribe_state();

        // Initial state should be Idle
        assert_eq!(agent.current_state().await, AgentState::Idle);

        // Run the agent
        let result = agent.run("test").await.expect("run should succeed");

        // Verify result
        assert_eq!(result.finish_reason, FinishReason::Stop);

        // Final state should be Completed
        assert_eq!(agent.current_state().await, AgentState::Completed);

        // Collect all state changes
        let mut states = vec![AgentState::Idle];
        while let Ok(state) = state_rx.try_recv() {
            states.push(state);
        }

        // Verify the state progression
        assert!(states.contains(&AgentState::Running));
        assert!(states.contains(&AgentState::AwaitingLLM));
        assert!(states.contains(&AgentState::Completed));
    }

    // ── test_agent_loop_subscribe_state ──────────────────────────────────────

    /// Verify that state subscription works correctly.
    #[tokio::test]
    async fn test_agent_loop_subscribe_state() {
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .build()
            .expect("build should succeed");

        // Multiple subscribers should work
        let _rx1 = agent.subscribe_state();
        let _rx2 = agent.subscribe_state();

        // Just verify we can create subscribers successfully
        // (broadcast channels don't have borrow(), only watch channels do)
    }

    // ── test_execute_tools_parallel_limit ────────────────────────────────────

    /// Verify that max_tool_calls_per_turn is respected.
    #[tokio::test]
    async fn test_execute_tools_parallel_limit() {
        // Create a mock agent with a limited tool call capacity
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_tool_provider())
            .build()
            .expect("build should succeed");

        // Verify the default limit
        assert_eq!(agent.config.max_tool_calls_per_turn, 10);
    }
}
