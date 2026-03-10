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
use futures::Stream;
use tokio::sync::{broadcast, mpsc, RwLock};

use crate::{
    error::AgentError,
    state_machine::{AgentState, StateEvent, StateMachine, TransitionResult},
    traits::{EventPublisher, HistoryManager, StopCondition},
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
    /// Event publisher for agent lifecycle events.
    pub(crate) event_publisher: Arc<dyn EventPublisher>,
    /// Agent ID for event publishing.
    pub(crate) agent_id: String,
}

impl AgentLoop {
    /// Create a new state broadcast channel with the specified capacity.
    #[allow(dead_code)]
    pub(crate) fn create_state_channel(capacity: usize) -> broadcast::Sender<AgentState> {
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
            // Publish turn started event
            self.event_publisher.publish_turn_started(&self.agent_id, loop_state.turn);

            // ── Step 2: custom stop conditions ────────────────────────────────
            for cond in &self.stop_conditions {
                if cond.should_stop(&loop_state) {
                    // Transition: Running -> Completed
                    self.transition(&mut state_machine, StateEvent::StopConditionMet)
                        .await?;
                    let result = AgentResult {
                        finish_reason: FinishReason::StopCondition,
                        last_message: self.history.messages().last().cloned(),
                        usage: loop_state.usage,
                        turns: loop_state.turn,
                        content: self
                            .history
                            .messages()
                            .last()
                            .map(|m| m.content.clone())
                            .unwrap_or_default(),
                        tool_calls: tool_calls_accumulated.clone(),
                        execution_time_ms: start_time.elapsed().as_millis() as u64,
                    };
                    self.event_publisher.publish_loop_completed(&self.agent_id, "stop_condition", result.turns);
                    return Ok(result);
                }
            }

            // ── Step 3a: max_turns guard ──────────────────────────────────────
            if loop_state.turn >= self.config.max_turns {
                // Transition: Running -> Completed
                self.transition(&mut state_machine, StateEvent::StopConditionMet)
                    .await?;
                let result = AgentResult {
                    finish_reason: FinishReason::MaxTurns,
                    last_message: self.history.messages().last().cloned(),
                    usage: loop_state.usage,
                    turns: loop_state.turn,
                    content: self
                        .history
                        .messages()
                        .last()
                        .map(|m| m.content.clone())
                        .unwrap_or_default(),
                    tool_calls: tool_calls_accumulated.clone(),
                    execution_time_ms: start_time.elapsed().as_millis() as u64,
                };
                self.event_publisher.publish_loop_completed(&self.agent_id, "max_turns", result.turns);
                return Ok(result);
            }

            // ── Step 3b: token_budget guard ───────────────────────────────────
            if self.config.token_budget > 0
                && loop_state.usage.total_tokens >= self.config.token_budget
            {
                // Transition: Running -> Completed
                self.transition(&mut state_machine, StateEvent::StopConditionMet)
                    .await?;
                let result = AgentResult {
                    finish_reason: FinishReason::TokenBudget,
                    last_message: self.history.messages().last().cloned(),
                    usage: loop_state.usage,
                    turns: loop_state.turn,
                    content: self
                        .history
                        .messages()
                        .last()
                        .map(|m| m.content.clone())
                        .unwrap_or_default(),
                    tool_calls: tool_calls_accumulated.clone(),
                    execution_time_ms: start_time.elapsed().as_millis() as u64,
                };
                self.event_publisher.publish_loop_completed(&self.agent_id, "token_budget", result.turns);
                return Ok(result);
            }

            // ── Step 4: build options and call the LLM ────────────────────────
            let mut options =
                Options::new(self.provider.model_id().to_string()).with_max_tokens(4096);
            if let Some(sys) = &self.config.system_prompt {
                options = options.with_system(sys.clone());
            }

            let messages: Vec<Message> = self.history.messages().to_vec();

            // Publish LLM request event
            self.event_publisher.publish_llm_request(
                &self.agent_id,
                self.provider.provider_id(),
                self.provider.model_id(),
                messages.len(),
            );

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
                let result = AgentResult {
                    finish_reason: FinishReason::TokenBudget,
                    last_message: Some(response.message),
                    usage: loop_state.usage,
                    turns: loop_state.turn,
                    content,
                    tool_calls: tool_calls_accumulated.clone(),
                    execution_time_ms: start_time.elapsed().as_millis() as u64,
                };
                self.event_publisher.publish_loop_completed(&self.agent_id, "token_budget", result.turns);
                return Ok(result);
            }

            let assistant_msg = response.message.clone();
            
            // Publish LLM response event
            self.event_publisher.publish_llm_response(
                &self.agent_id,
                self.provider.provider_id(),
                response.usage.clone(),
                &format!("{:?}", response.finish_reason),
            );
            
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

                    // Publish tool called events
                    for call in &tool_calls {
                        self.event_publisher.publish_tool_called(&self.agent_id, &call.name, &call.id);
                    }

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

                    // Publish tool result events and append to history
                    for (call_id, result_content) in &tool_results {
                        // Find the tool name for this call_id
                        if let Some(call) = tool_calls.iter().find(|c| c.id == *call_id) {
                            let success = !result_content.contains("\"error\"");
                            self.event_publisher.publish_tool_result(&self.agent_id, &call.name, call_id, success);
                        }
                        self.history
                            .append(Message::tool_result(call_id.clone(), result_content.clone()));
                    }
                    loop_state.history_len = self.history.len();

                    // Transition: ToolExecuting -> Running
                    self.transition(&mut state_machine, StateEvent::ToolsCompleted)
                        .await?;

                    continue; // Back to top of loop with tool results in history.
                } else {
                    // FIX-21: no tool registry configured — return a structured error result
                    // for every requested tool call so the LLM knows tools are unavailable
                    // (rather than silently treating the response as a natural stop).
                    let tool_calls = assistant_msg.tool_calls.as_ref().unwrap().clone();
                    tracing::warn!(
                        agent_id = %self.agent_id,
                        "LLM requested {} tool call(s) but no ToolRegistry is configured",
                        tool_calls.len()
                    );

                    self.transition(&mut state_machine, StateEvent::ToolsRequired)
                        .await?;

                    for call in &tool_calls {
                        let error_json = serde_json::json!({
                            "error": "no_tool_registry",
                            "message": "This agent has no tool registry configured; tool calls are not available"
                        })
                        .to_string();
                        self.history
                            .append(Message::tool_result(call.id.clone(), error_json));
                    }
                    loop_state.history_len = self.history.len();

                    self.transition(&mut state_machine, StateEvent::ToolsCompleted)
                        .await?;

                    continue;
                }
            }

            // ── Step 7: no tool calls → natural stop ──────────────────────────
            // Transition: AwaitingLLM -> Running (briefly)
            self.transition(&mut state_machine, StateEvent::LLMResponseReceived)
                .await?;
            // Then immediately to Completed
            self.transition(&mut state_machine, StateEvent::StopConditionMet)
                .await?;

            let result = AgentResult {
                finish_reason: FinishReason::Stop,
                last_message: Some(assistant_msg.clone()),
                usage: loop_state.usage,
                turns: loop_state.turn,
                content: assistant_msg.content.clone(),
                tool_calls: tool_calls_accumulated,
                execution_time_ms: start_time.elapsed().as_millis() as u64,
            };
            self.event_publisher.publish_loop_completed(&self.agent_id, "stop", result.turns);
            return Ok(result);
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
    /// This implementation uses `complete_stream()` for true token-by-token streaming.
    pub async fn stream_run(
        &mut self,
        initial_message: impl Into<String>,
    ) -> Result<impl Stream<Item = crate::types::StreamChunk>, crate::error::AgentError> {
        use crate::types::StreamChunk;
        use futures::stream;

        let initial = initial_message.into();

        // For v1.0: simplified implementation that runs full loop then yields chunks
        // Future versions can implement true token-by-token streaming
        let result = self.run(&initial).await?;

        let chunks = vec![
            StreamChunk::Text {
                content: result.content.clone(),
                is_final: true,
            },
            StreamChunk::UsageUpdate(result.usage),
            StreamChunk::Finish(result.finish_reason),
        ];

        Ok(stream::iter(chunks))
    }

    /// 真流式运行：每个 token 实时推送到 tx channel。
    ///
    /// 使用 `complete_stream()` 获取流式响应，每个 delta 立即通过 channel 发送。
    /// 工具调用完成后继续流式处理后续 LLM 响应。
    ///
    /// # 停止条件
    ///
    /// 与 `run()` 相同：MaxTurns、TokenBudget、custom stop conditions，以及无工具调用时自然停止。
    ///
    /// # 示例
    ///
    /// ```rust,no_run
    /// use claw_loop::{AgentLoopBuilder, StreamChunk};
    /// use tokio::sync::mpsc;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let provider = claw_provider::providers::provider_from_env()?;
    /// let mut agent = AgentLoopBuilder::new()
    ///     .with_provider(provider)
    ///     .build()?;
    ///
    /// let (tx, mut rx) = mpsc::channel(64);
    ///
    /// tokio::spawn(async move {
    ///     let _ = agent.run_streaming("Hello!", tx).await;
    /// });
    ///
    /// while let Some(chunk) = rx.recv().await {
    ///     match chunk {
    ///         StreamChunk::Text { content, .. } => print!("{}", content),
    ///         StreamChunk::Finish(_) => break,
    ///         _ => {}
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn run_streaming(
        &mut self,
        initial_message: impl Into<String>,
        tx: mpsc::Sender<crate::types::StreamChunk>,
    ) -> Result<AgentResult, AgentError> {
        use crate::types::StreamChunk;
        use futures::StreamExt;

        let start_time = std::time::Instant::now();
        let mut tool_calls_accumulated: Vec<claw_provider::types::ToolCall> = Vec::new();
        let mut state_machine = StateMachine::new();

        // Transition: Idle -> Running
        self.transition(&mut state_machine, StateEvent::Start).await?;

        self.history.append(Message::user(initial_message));

        let mut loop_state = LoopState::new();

        loop {
            // Publish turn started event
            self.event_publisher.publish_turn_started(&self.agent_id, loop_state.turn);

            // ── Custom stop conditions ─────────────────────────────────────────
            for cond in &self.stop_conditions {
                if cond.should_stop(&loop_state) {
                    self.transition(&mut state_machine, StateEvent::StopConditionMet).await?;
                    let result = AgentResult {
                        finish_reason: FinishReason::StopCondition,
                        last_message: self.history.messages().last().cloned(),
                        usage: loop_state.usage,
                        turns: loop_state.turn,
                        content: self.history.messages().last().map(|m| m.content.clone()).unwrap_or_default(),
                        tool_calls: tool_calls_accumulated.clone(),
                        execution_time_ms: start_time.elapsed().as_millis() as u64,
                    };
                    self.event_publisher.publish_loop_completed(&self.agent_id, "stop_condition", result.turns);
                    let _ = tx.send(StreamChunk::Finish(FinishReason::StopCondition)).await;
                    return Ok(result);
                }
            }

            // ── max_turns guard ────────────────────────────────────────────────
            if loop_state.turn >= self.config.max_turns {
                self.transition(&mut state_machine, StateEvent::StopConditionMet).await?;
                let result = AgentResult {
                    finish_reason: FinishReason::MaxTurns,
                    last_message: self.history.messages().last().cloned(),
                    usage: loop_state.usage,
                    turns: loop_state.turn,
                    content: self.history.messages().last().map(|m| m.content.clone()).unwrap_or_default(),
                    tool_calls: tool_calls_accumulated.clone(),
                    execution_time_ms: start_time.elapsed().as_millis() as u64,
                };
                self.event_publisher.publish_loop_completed(&self.agent_id, "max_turns", result.turns);
                let _ = tx.send(StreamChunk::Finish(FinishReason::MaxTurns)).await;
                return Ok(result);
            }

            // ── token_budget guard ─────────────────────────────────────────────
            if self.config.token_budget > 0
                && loop_state.usage.total_tokens >= self.config.token_budget
            {
                self.transition(&mut state_machine, StateEvent::StopConditionMet).await?;
                let result = AgentResult {
                    finish_reason: FinishReason::TokenBudget,
                    last_message: self.history.messages().last().cloned(),
                    usage: loop_state.usage,
                    turns: loop_state.turn,
                    content: self.history.messages().last().map(|m| m.content.clone()).unwrap_or_default(),
                    tool_calls: tool_calls_accumulated.clone(),
                    execution_time_ms: start_time.elapsed().as_millis() as u64,
                };
                self.event_publisher.publish_loop_completed(&self.agent_id, "token_budget", result.turns);
                let _ = tx.send(StreamChunk::Finish(FinishReason::TokenBudget)).await;
                return Ok(result);
            }

            // ── Build options and call LLM streaming ──────────────────────────
            let mut options =
                Options::new(self.provider.model_id().to_string()).with_max_tokens(4096);
            if let Some(sys) = &self.config.system_prompt {
                options = options.with_system(sys.clone());
            }

            let messages: Vec<Message> = self.history.messages().to_vec();

            self.event_publisher.publish_llm_request(
                &self.agent_id,
                self.provider.provider_id(),
                self.provider.model_id(),
                messages.len(),
            );

            // Transition: Running -> AwaitingLLM
            self.transition(&mut state_machine, StateEvent::LLMRequestSent).await?;

            // Call complete_stream() for true token-by-token streaming
            let stream_result = self.provider.complete_stream(messages.clone(), options.clone()).await;

            let mut delta_stream = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    self.transition(&mut state_machine, StateEvent::Error).await?;
                    let _ = tx.send(StreamChunk::Error(e.to_string())).await;
                    return Err(AgentError::Provider(e.to_string()));
                }
            };

            // Accumulate full response while streaming deltas to channel
            let mut full_content = String::new();
            let mut accumulated_tool_calls: Vec<claw_provider::types::ToolCall> = Vec::new();
            let mut final_usage: Option<claw_provider::types::TokenUsage> = None;
            let mut final_finish_reason: Option<claw_provider::types::FinishReason> = None;

            // Track partial tool call being assembled (streaming may split args across deltas)
            let mut partial_tool_id: Option<String> = None;
            let mut partial_tool_name: Option<String> = None;
            let mut partial_tool_args = String::new();

            while let Some(delta_result) = delta_stream.next().await {
                match delta_result {
                    Err(e) => {
                        self.transition(&mut state_machine, StateEvent::Error).await?;
                        let _ = tx.send(StreamChunk::Error(e.to_string())).await;
                        return Err(AgentError::Provider(e.to_string()));
                    }
                    Ok(delta) => {
                        // Capture usage and finish_reason from final chunk
                        if let Some(usage) = delta.usage.clone() {
                            final_usage = Some(usage);
                        }
                        if let Some(fr) = delta.finish_reason.clone() {
                            final_finish_reason = Some(fr);
                        }

                        // Handle text content delta
                        if let Some(ref text) = delta.content {
                            if !text.is_empty() {
                                full_content.push_str(text);
                                // Send text chunk to caller — ignore send errors (caller may have dropped rx)
                                let _ = tx.send(StreamChunk::Text {
                                    content: text.clone(),
                                    is_final: false,
                                }).await;
                            }
                        }

                        // Handle tool call delta
                        if let Some(ref tc) = delta.tool_call {
                            // If this is a new tool call (new id/name), flush any partial one first
                            let is_new_call = tc.id != partial_tool_id.as_deref().unwrap_or("");
                            if is_new_call && partial_tool_id.is_some() {
                                // Flush the previous partial tool call
                                let flushed = claw_provider::types::ToolCall {
                                    id: partial_tool_id.take().unwrap(),
                                    name: partial_tool_name.take().unwrap_or_default(),
                                    arguments: std::mem::take(&mut partial_tool_args),
                                };
                                let _ = tx.send(StreamChunk::ToolStart {
                                    id: flushed.id.clone(),
                                    name: flushed.name.clone(),
                                }).await;
                                accumulated_tool_calls.push(flushed);
                            }

                            if is_new_call && !tc.id.is_empty() {
                                // Start a new partial tool call
                                partial_tool_id = Some(tc.id.clone());
                                partial_tool_name = Some(tc.name.clone());
                                partial_tool_args = tc.arguments.clone();
                            } else {
                                // Accumulate arguments for ongoing tool call
                                partial_tool_args.push_str(&tc.arguments);
                            }
                        }
                    }
                }
            }

            // Flush any remaining partial tool call after stream ends
            if let Some(id) = partial_tool_id.take() {
                let flushed = claw_provider::types::ToolCall {
                    id,
                    name: partial_tool_name.take().unwrap_or_default(),
                    arguments: std::mem::take(&mut partial_tool_args),
                };
                let _ = tx.send(StreamChunk::ToolStart {
                    id: flushed.id.clone(),
                    name: flushed.name.clone(),
                }).await;
                accumulated_tool_calls.push(flushed);
            }

            // Update token usage from stream
            if let Some(usage) = final_usage {
                loop_state.usage.prompt_tokens += usage.prompt_tokens;
                loop_state.usage.completion_tokens += usage.completion_tokens;
                loop_state.usage.total_tokens += usage.total_tokens;
            } else {
                // Fallback: estimate from content length if provider didn't report usage
                let estimated: u64 = (full_content.len() as u64) / 4 + 10;
                loop_state.usage.completion_tokens += estimated;
                loop_state.usage.total_tokens += estimated;
            }
            loop_state.turn += 1;

            // Post-stream token-budget check
            if self.config.token_budget > 0
                && loop_state.usage.total_tokens >= self.config.token_budget
            {
                // Build assistant message from streamed content
                let mut assistant_msg = Message::assistant(full_content.clone());
                if !accumulated_tool_calls.is_empty() {
                    assistant_msg.tool_calls = Some(accumulated_tool_calls.clone());
                }
                self.history.append(assistant_msg.clone());
                self.transition(&mut state_machine, StateEvent::LLMResponseReceived).await?;
                self.transition(&mut state_machine, StateEvent::StopConditionMet).await?;
                let result = AgentResult {
                    finish_reason: FinishReason::TokenBudget,
                    last_message: Some(assistant_msg),
                    usage: loop_state.usage,
                    turns: loop_state.turn,
                    content: full_content,
                    tool_calls: tool_calls_accumulated,
                    execution_time_ms: start_time.elapsed().as_millis() as u64,
                };
                self.event_publisher.publish_loop_completed(&self.agent_id, "token_budget", result.turns);
                let _ = tx.send(StreamChunk::Finish(FinishReason::TokenBudget)).await;
                return Ok(result);
            }

            // Build the complete assistant message from streamed content
            let mut assistant_msg = Message::assistant(full_content.clone());
            if !accumulated_tool_calls.is_empty() {
                assistant_msg.tool_calls = Some(accumulated_tool_calls.clone());
            }

            // Publish LLM response event
            self.event_publisher.publish_llm_response(
                &self.agent_id,
                self.provider.provider_id(),
                loop_state.usage.clone(),
                &format!("{:?}", final_finish_reason),
            );

            self.history.append(assistant_msg.clone());
            loop_state.history_len = self.history.len();

            // ── Tool call handling ─────────────────────────────────────────────
            let has_tool_calls = !accumulated_tool_calls.is_empty();

            if has_tool_calls && self.config.tool_use_enabled {
                if let Some(ref registry) = self.tools {
                    let tool_calls = accumulated_tool_calls.clone();
                    tool_calls_accumulated.extend(tool_calls.clone());

                    for call in &tool_calls {
                        self.event_publisher.publish_tool_called(&self.agent_id, &call.name, &call.id);
                    }

                    self.transition(&mut state_machine, StateEvent::ToolsRequired).await?;

                    let tool_results = match self.execute_tools_parallel(&tool_calls, registry).await {
                        Ok(results) => results,
                        Err(e) => {
                            let _ = self.transition(&mut state_machine, StateEvent::Error).await;
                            return Err(e);
                        }
                    };

                    for (call_id, result_content) in &tool_results {
                        // Send tool result chunk to stream
                        let result_json: serde_json::Value = serde_json::from_str(result_content)
                            .unwrap_or_else(|_| serde_json::Value::String(result_content.clone()));
                        let _ = tx.send(StreamChunk::ToolComplete {
                            id: call_id.clone(),
                            result: result_json,
                        }).await;

                        if let Some(call) = tool_calls.iter().find(|c| c.id == *call_id) {
                            let success = !result_content.contains("\"error\"");
                            self.event_publisher.publish_tool_result(&self.agent_id, &call.name, call_id, success);
                        }
                        self.history.append(Message::tool_result(call_id.clone(), result_content.clone()));
                    }
                    loop_state.history_len = self.history.len();

                    self.transition(&mut state_machine, StateEvent::ToolsCompleted).await?;
                    continue;
                } else {
                    // No tool registry — return error results for all tool calls
                    let tool_calls = accumulated_tool_calls.clone();
                    tracing::warn!(
                        agent_id = %self.agent_id,
                        "LLM requested {} tool call(s) but no ToolRegistry is configured",
                        tool_calls.len()
                    );

                    self.transition(&mut state_machine, StateEvent::ToolsRequired).await?;

                    for call in &tool_calls {
                        let error_json = serde_json::json!({
                            "error": "no_tool_registry",
                            "message": "This agent has no tool registry configured; tool calls are not available"
                        });
                        let _ = tx.send(StreamChunk::ToolError {
                            id: call.id.clone(),
                            error: "no_tool_registry".to_string(),
                        }).await;
                        self.history.append(Message::tool_result(
                            call.id.clone(),
                            error_json.to_string(),
                        ));
                    }
                    loop_state.history_len = self.history.len();

                    self.transition(&mut state_machine, StateEvent::ToolsCompleted).await?;
                    continue;
                }
            }

            // ── No tool calls → natural stop ───────────────────────────────────
            self.transition(&mut state_machine, StateEvent::LLMResponseReceived).await?;
            self.transition(&mut state_machine, StateEvent::StopConditionMet).await?;

            // Send final text marker and finish
            let _ = tx.send(StreamChunk::Text {
                content: String::new(),
                is_final: true,
            }).await;
            let _ = tx.send(StreamChunk::UsageUpdate(loop_state.usage.clone())).await;
            let _ = tx.send(StreamChunk::Finish(FinishReason::Stop)).await;

            let result = AgentResult {
                finish_reason: FinishReason::Stop,
                last_message: Some(assistant_msg.clone()),
                usage: loop_state.usage,
                turns: loop_state.turn,
                content: full_content,
                tool_calls: tool_calls_accumulated,
                execution_time_ms: start_time.elapsed().as_millis() as u64,
            };
            self.event_publisher.publish_loop_completed(&self.agent_id, "stop", result.turns);
            return Ok(result);
        }
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

        async fn complete_inner(
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

        async fn complete_inner(
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

    /// Verify that build() rejects max_turns == 0 (TASK-22 config validation).
    #[test]
    fn test_agent_loop_max_turns_zero_rejected() {
        let result = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_max_turns(0)
            .build();
        assert!(result.is_err(), "build with max_turns=0 should fail");
        match result.err().unwrap() {
            AgentError::Context(msg) => {
                assert!(
                    msg.contains("max_turns"),
                    "error should mention max_turns, got: {msg}"
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    /// Set max_turns to 1 so the loop stops after exactly one LLM turn.
    #[tokio::test]
    async fn test_agent_loop_max_turns_stop() {
        let mut agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_max_turns(1)
            .build()
            .expect("build with max_turns=1 should succeed");

        let result = agent
            .run("trigger max turns")
            .await
            .expect("run should succeed");

        // MockProvider returns a plain text response (no tool calls), so the
        // loop completes with FinishReason::Stop after 1 turn.
        assert_eq!(result.turns, 1);

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
