---
title: Agent Loop State Machine Design
description: "Detailed design for the claw-loop crate: state machine, turn lifecycle, tool execution, history truncation, and streaming"
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](agent-loop-state-machine.zh.md)


# Agent Loop State Machine Design

> **Crate:** `claw-loop` (Phase 4)  
> **Layer:** Layer 2 — Agent Kernel Protocol  
> **Purpose:** Defines the execution algorithm, state transitions, and memory management for the agent loop engine. This document fills the gap left by BUILD_PLAN.md, which specified the data structures but not the runtime behavior.

---

## 1. Overview

`AgentLoop` is the heart of the claw-kernel execution model. It drives the conversation between a user, an LLM provider, and a set of tools. The loop runs until a stop condition fires or an unrecoverable error occurs.

**Key invariants:**

- One *turn* = one LLM call + all tool calls that result from it (parallel execution allowed).
- Stop conditions are evaluated only after a complete turn, never mid-turn.
- The system prompt is always preserved in context, regardless of token pressure.
- `get_context(max_tokens)` never panics. If the budget is too small for anything beyond the system prompt, it returns only the system prompt.
- Tool calls within a single LLM response are executed concurrently via `tokio::join_all`, subject to `max_tool_calls_per_turn`.

---

## 2. State Machine Diagram

```
                         ┌─────────────────────────────────────────────────────┐
                         │                   AgentLoop States                  │
                         └─────────────────────────────────────────────────────┘

              start / run(user_message)
                         │
                         ▼
                    ┌─────────┐
                    │  Idle   │◄──────────────────────────────────────────────┐
                    └────┬────┘                                               │
                         │ append user message to history                     │
                         │ increment turn_count                               │
                         ▼                                                    │
                   ┌──────────┐                                               │
                   │ Running  │                                               │
                   └────┬─────┘                                               │
                        │ call history.get_context(token_budget)              │
                        │ submit context to provider                          │
                        ▼                                                    │
              ┌──────────────────┐                                           │
              │  AwaitingLLM     │                                           │
              │  (streaming or   │                                           │
              │   blocking)      │                                           │
              └────────┬─────────┘                                           │
                       │                                                     │
          ┌────────────┴────────────┐                                        │
          │ response has            │ response has                           │
          │ tool_calls              │ no tool_calls                          │
          ▼                         ▼                                        │
 ┌──────────────────┐    ┌──────────────────────┐                           │
 │ ProcessingTool   │    │  EvaluatingStop      │                           │
 │ Calls            │    │                      │                           │
 │ (validate,       │    │  1. MaxTurnsReached  │                           │
 │  dispatch)       │    │  2. TokenBudget      │                           │
 └────────┬─────────┘    │  3. NoToolCall       │                           │
          │              │  4. Custom           │                           │
          │              └──────────┬───────────┘                           │
          ▼                         │                                        │
 ┌──────────────────┐               │                                        │
 │ AwaitingTool     │    ┌──────────┴──────────┐                            │
 │ Results          │    │ stop?               │ no stop                    │
 │ (tokio::join_all)│    ▼                     ▼                            │
 └────────┬─────────┘  ┌────────────┐    ──────────────                    │
          │            │ Terminated │   (loop back)  ──────────────────────┘
          │            └────────────┘
          │ all results collected
          │ append assistant + tool_result messages
          ▼
 ┌──────────────────────┐
 │  EvaluatingStop      │
 │  (same order as      │
 │   above)             │
 └──────────────────────┘

  Error paths (from any state):
  ┌──────────────────────────────────────────────────────────────────────┐
  │  ProviderError ──► retry up to config.max_retries ──► Terminated    │
  │  ToolTimeout   ──► inject error result ──► continue turn            │
  │  ToolPanic     ──► inject error result ──► continue turn            │
  │  UserInterrupt ──► Terminated (FinishReason::UserInterrupted)       │
  └──────────────────────────────────────────────────────────────────────┘
```

**State descriptions:**

| State | Description |
|-------|-------------|
| `Idle` | Loop is ready. No active turn. Waiting for `run()` call. |
| `Running` | A turn has started. History context is being prepared. |
| `AwaitingLLM` | HTTP request to provider is in flight. Streaming chunks may be emitted. |
| `ProcessingToolCalls` | LLM response received. Tool calls are validated and dispatched. |
| `AwaitingToolResults` | All tool futures are running concurrently. Waiting for all to complete or timeout. |
| `EvaluatingStop` | All results for this turn are collected. Stop conditions are checked in order. |
| `Terminated` | Loop has exited. `FinishReason` is set. No further turns will run. |

---

## 3. Turn Lifecycle

A single turn proceeds through these phases in strict order:

```
Turn N lifecycle:
─────────────────────────────────────────────────────────────────────────

Phase 1: Context Preparation
  - history.get_context(token_budget) → Vec<Message>
  - If token count > 80% of budget AND summarizer is configured:
      invoke summarizer on oldest turns (see Section 7)
  - Prepend system_prompt if present and not already in context

Phase 2: LLM Call
  - Submit context to provider
  - If enable_streaming = true:
      forward Delta chunks to EventBus while buffering full response
  - Await complete response (text + tool_calls)

Phase 3: Tool Dispatch (if tool_calls present)
  - Validate: tool_calls.len() <= max_tool_calls_per_turn
    - If exceeded: truncate to first max_tool_calls_per_turn, log warning
  - For each tool_call: look up tool in ToolRegistry
    - If tool not found: inject ToolNotFound error result immediately
  - Dispatch all valid tool calls concurrently: tokio::join_all(futures)
  - Each future is wrapped with timeout(config.tool_timeout)
    - On timeout: inject ToolTimeout error result for that call
    - Other calls continue unaffected

Phase 4: History Update
  - Append assistant message (text + tool_calls) to history
  - Append all tool_result messages to history
  - Update LoopState: turn_count++, token_usage, last_message, tool_calls_made

Phase 5: Stop Condition Evaluation
  - Evaluate in fixed order (see Section 6)
  - If any condition fires: set FinishReason, transition to Terminated
  - If no condition fires: transition back to Idle (ready for next turn)

─────────────────────────────────────────────────────────────────────────
```

---

## 4. Pseudocode: `AgentLoop::run()`

This is the complete algorithm for `AgentLoop::run(user_message)`. Not Rust syntax — algorithmic steps only.

```
FUNCTION AgentLoop::run(user_message: String) -> Result<AgentResult>

  // --- Initialization ---
  append Message { role: User, content: user_message } to history
  turn_count = 0
  total_tokens = 0
  final_text = ""

  LOOP:

    // --- Phase 1: Context Preparation ---
    turn_count += 1

    IF token_budget is set AND history.estimated_tokens() > 0.8 * token_budget:
      IF summarizer is configured:
        history.summarize(summarizer)
      ELSE:
        history.truncate_to_fit(token_budget)

    context = history.get_context(token_budget ?? MAX_USIZE)
    // context always includes system_prompt + as many recent turns as fit

    // --- Phase 2: LLM Call ---
    IF enable_streaming:
      response = provider.stream(context)
      WHILE chunk = response.next():
        IF chunk is Delta(text):
          emit StreamEvent::Delta(text) to EventBus
        buffer chunk
      full_response = assemble(buffer)
    ELSE:
      full_response = provider.complete(context)

    // full_response has: .text, .tool_calls, .token_usage
    total_tokens += full_response.token_usage.total

    // --- Phase 3: Tool Dispatch ---
    IF full_response.tool_calls is not empty:

      calls_to_run = full_response.tool_calls
      IF calls_to_run.len() > max_tool_calls_per_turn:
        log WARNING "truncating tool calls"
        calls_to_run = calls_to_run[0..max_tool_calls_per_turn]

      futures = []
      FOR EACH call IN calls_to_run:
        tool = tool_registry.get(call.name)
        IF tool is None:
          futures.push(ready(ToolResult::error(call.id, "tool not found")))
        ELSE:
          future = async { tool.call(call.arguments) }
          future = timeout(tool_timeout, future)
            .map_err(|_| ToolResult::error(call.id, "timeout"))
          futures.push(future)

      tool_results = join_all(futures).await
      // All results collected. Timed-out calls have error results.

    ELSE:
      tool_results = []

    // --- Phase 4: History Update ---
    append Message { role: Assistant, content: full_response.text,
                     tool_calls: full_response.tool_calls } to history

    FOR EACH result IN tool_results:
      append Message { role: ToolResult, content: result } to history

    final_text = full_response.text

    // Update LoopState
    loop_state = LoopState {
      turn_count,
      token_usage: TokenUsage { total: total_tokens, ... },
      last_message: Some(last assistant message),
      tool_calls_made: tool_results.len(),
    }

    // --- Phase 5: Stop Condition Evaluation ---
    finish_reason = evaluate_stop_conditions(loop_state, full_response)
    IF finish_reason is Some(reason):
      RETURN Ok(AgentResult {
        content: final_text,
        tool_calls: all_tool_calls_this_session,
        turns: turn_count,
        token_usage: ...,
        finish_reason: reason,
        execution_time: elapsed,
      })

    // No stop condition fired. Continue to next turn.
    // (Loop back to Phase 1)

  END LOOP

END FUNCTION
```

---

## 5. Tool Call Execution

### Concurrency Model

All tool calls from a single LLM response run concurrently. The loop does not start the next LLM call until every tool result is collected (or timed out).

```
LLM response: [tool_call_A, tool_call_B, tool_call_C]
                     │              │              │
                     ▼              ▼              ▼
              ┌──────────┐  ┌──────────┐  ┌──────────┐
              │ Future A │  │ Future B │  │ Future C │   ← tokio::join_all
              │ timeout  │  │ timeout  │  │ timeout  │
              └────┬─────┘  └────┬─────┘  └────┬─────┘
                   │              │              │
                   ▼              ▼              ▼
              [result_A]    [result_B]    [timeout_err_C]
                   │              │              │
                   └──────────────┴──────────────┘
                                  │
                         all results collected
                                  │
                         append to history
```

### Tool Call Limits

If `full_response.tool_calls.len() > max_tool_calls_per_turn`:
- Truncate to the first `max_tool_calls_per_turn` calls.
- Log a warning with the count of dropped calls.
- The dropped calls are never executed and never appear in history.

### Tool Not Found

If a tool name in the LLM response doesn't exist in `ToolRegistry`:
- Inject an immediate error result: `"tool '{name}' not found in registry"`.
- Do not abort the turn. Other tool calls proceed normally.

### Tool Timeout

If a tool call exceeds `config.tool_timeout`:
- Inject an error result: `"tool '{name}' timed out after {duration}"`.
- The tool's async task is cancelled (Tokio cancellation).
- Other concurrent tool calls are unaffected.

---

## 6. Stop Condition Evaluation Order

Stop conditions are evaluated in this fixed order after every complete turn. Order matters: earlier conditions take priority.

```
FUNCTION evaluate_stop_conditions(state: LoopState, response: LLMResponse) -> Option<FinishReason>

  // 1. MaxTurnsReached — checked first, always
  IF config.max_turns is Some(max) AND state.turn_count >= max:
    RETURN Some(FinishReason::MaxTurnsReached)

  // 2. TokenBudgetExceeded — checked second
  IF config.token_budget is Some(budget) AND state.token_usage.total >= budget:
    RETURN Some(FinishReason::TokenBudgetExceeded)

  // 3. NoToolCallInTurn — checked third, only if configured
  //    Fires when the LLM made no tool calls in this turn AND the condition is enabled
  IF NoToolCallCondition is in stop_conditions:
    IF response.tool_calls is empty:
      RETURN Some(FinishReason::StopConditionMet("NoToolCallInTurn"))

  // 4. Custom StopConditions — evaluated last, in registration order
  FOR EACH condition IN user_defined_stop_conditions:
    IF condition.should_stop(state):
      RETURN Some(FinishReason::StopConditionMet(condition.name()))

  // No condition fired
  RETURN None

END FUNCTION
```

**Why this order?**

- `MaxTurnsReached` is a hard safety cap. It must fire even if the token budget hasn't been hit yet.
- `TokenBudgetExceeded` is checked before `NoToolCall` because a budget overrun is a resource constraint, not a behavioral signal.
- `NoToolCallInTurn` is a behavioral signal: the LLM decided it was done. It's checked before custom conditions so built-in behavior is predictable.
- Custom conditions are last because they're user-defined and may have arbitrary logic. They can't override the built-in safety caps.

---

## 7. HistoryManager Truncation Algorithm

`HistoryManager` is responsible for keeping the conversation context within the token budget. The algorithm is a **sliding window** that always preserves the system prompt.

### Token Estimation

Token counts are estimated, not exact. The implementation uses a fast approximation (e.g., `chars / 4` or a tiktoken-compatible encoder). The estimate must be conservative: round up, not down.

### `get_context(max_tokens)` Algorithm

```
FUNCTION get_context(max_tokens: usize) -> Vec<Message>

  messages = []

  // System prompt is always included, unconditionally
  IF system_prompt is Some(prompt):
    system_tokens = estimate_tokens(prompt)
    IF system_tokens >= max_tokens:
      // Budget too small even for system prompt alone.
      // Return system prompt only — never panic, never return empty.
      RETURN [Message { role: System, content: prompt }]
    messages.push(Message { role: System, content: prompt })
    remaining = max_tokens - system_tokens
  ELSE:
    remaining = max_tokens

  // Walk history from newest to oldest (reverse order)
  // Include turns that fit within remaining budget
  included = []
  FOR EACH turn IN history.turns().reversed():
    turn_tokens = estimate_tokens(turn)
    IF turn_tokens <= remaining:
      included.prepend(turn)
      remaining -= turn_tokens
    ELSE:
      BREAK  // Older turns won't fit either (they're not smaller)

  messages.extend(included)
  RETURN messages

END FUNCTION
```

### `truncate_to_fit(max_tokens)` Algorithm

Called proactively when token usage exceeds 80% of budget.

```
FUNCTION truncate_to_fit(max_tokens: usize)

  WHILE history.estimated_tokens() > max_tokens:
    oldest_pair = history.pop_oldest_user_assistant_pair()
    IF oldest_pair is None:
      BREAK  // Nothing left to drop (only system prompt remains)

  // After truncation, history fits within max_tokens
  // System prompt is never popped

END FUNCTION
```

### `summarize(summarizer)` Algorithm

Called when token count exceeds 80% of budget AND a `Summarizer` is configured. Summarization compresses old turns into a single summary message, freeing space for new turns.

```
FUNCTION summarize(summarizer: &dyn Summarizer)

  // Identify the oldest N turns to summarize
  // Keep the most recent K turns intact (K = summarizer.keep_recent_turns())
  // Default K = 4 (last 2 user+assistant pairs)

  turns_to_summarize = history.turns()[0 .. len - K]

  IF turns_to_summarize is empty:
    RETURN  // Nothing to summarize

  summary_text = summarizer.summarize(turns_to_summarize).await
  // summarizer makes an LLM call internally, using a separate provider instance
  // to avoid recursion into the main loop

  // Replace summarized turns with a single summary message
  history.replace_turns(
    turns_to_summarize,
    Message {
      role: System,
      content: format!("[Conversation summary: {}]", summary_text),
    }
  )

  // After summarization, history is smaller.
  // If still over budget, truncate_to_fit() is called as fallback.
  IF history.estimated_tokens() > token_budget:
    truncate_to_fit(token_budget)

END FUNCTION
```

### When Summarizer is Invoked

The summarizer is invoked at the start of a turn (Phase 1), before the LLM call, when:

```
history.estimated_tokens() > 0.80 * config.token_budget
AND config.summarizer is Some(_)
```

The 80% threshold gives the summarizer room to work before the budget is fully exhausted. If the summarizer itself fails (provider error, timeout), the loop falls back to `truncate_to_fit()` silently.

### Fallback When No Summarizer

When `config.summarizer` is `None` and the token count exceeds 80% of budget:

```
history.truncate_to_fit(token_budget)
```

Oldest user+assistant pairs are dropped until the history fits. The system prompt is never dropped.

---

## 8. Streaming Output

When `config.enable_streaming = true`, the loop emits text delta chunks to the caller while still waiting for the full response.

### Mechanism

The loop uses an `mpsc` channel (Tokio) to forward chunks:

```
Provider stream ──► buffer ──► assemble full response (for tool call extraction)
                    │
                    └──► emit Delta chunks to EventBus (for caller consumption)
```

The two paths run concurrently. The loop does not wait for the caller to consume chunks before continuing. The channel is bounded (default capacity: 64 chunks). If the caller is slow and the channel fills, the loop applies backpressure by awaiting the send.

### Stream Events

```
StreamEvent::TurnStart { turn: usize }
StreamEvent::Delta { text: String }
StreamEvent::ToolCallStart { id: String, name: String }
StreamEvent::ToolCallEnd { id: String, result: ToolResult }
StreamEvent::TurnEnd { finish_reason: Option<FinishReason> }
```

### Streaming and Tool Calls

The loop buffers the full LLM response before dispatching tool calls. This is necessary because:
- Tool call arguments may span multiple delta chunks.
- The loop needs the complete JSON to validate and dispatch tool calls.

Delta chunks for the text portion are forwarded immediately as they arrive. Tool call chunks are buffered silently (not forwarded as deltas).

### Non-Streaming Mode

When `enable_streaming = false`:
- No `StreamEvent::Delta` events are emitted.
- `StreamEvent::TurnStart` and `StreamEvent::TurnEnd` are still emitted.
- Tool call events are still emitted.

---

## 9. Error Handling

### ProviderError

A `ProviderError` occurs when the HTTP call to the LLM provider fails (network error, rate limit, server error).

```
Retry policy:
  - Attempt 1: immediate
  - Attempt 2: wait 1s
  - Attempt 3: wait 4s
  - Attempt 4+: wait 16s (capped)
  - Max retries: config.max_provider_retries (default: 3)

After max retries:
  - Set FinishReason::Error(AgentError::ProviderFailed { ... })
  - Transition to Terminated
  - Return Err(AgentError) from run()
```

The retry policy uses exponential backoff with jitter. The loop does not retry on `4xx` errors (except `429 Too Many Requests`).

### ToolTimeout

A tool call that exceeds `config.tool_timeout` is handled within the turn:

```
- Cancel the timed-out tool's async task
- Inject error result: ToolResult::error(id, "timed out after {duration}")
- Continue collecting results from other concurrent tool calls
- The turn completes normally with the error result in history
- The LLM sees the timeout error in the next turn's context and can decide how to proceed
```

### ToolPanic

If a tool's implementation panics (caught via `catch_unwind` in the tool executor):

```
- Inject error result: ToolResult::error(id, "tool panicked: {message}")
- Continue as with ToolTimeout
```

### UserInterrupt

The loop checks for a cancellation signal at the start of each turn (before the LLM call) and after each tool batch completes:

```
IF cancellation_token.is_cancelled():
  SET FinishReason::UserInterrupted
  TRANSITION to Terminated
  RETURN Ok(AgentResult { finish_reason: UserInterrupted, ... })
```

Note: `UserInterrupted` returns `Ok`, not `Err`. The caller receives a valid `AgentResult` with the partial conversation.

### Error State Summary

| Error | Recovery | FinishReason |
|-------|----------|--------------|
| `ProviderError` (retryable) | Retry with backoff | `Error(ProviderFailed)` after max retries |
| `ProviderError` (4xx non-429) | No retry | `Error(ProviderFailed)` immediately |
| `ToolTimeout` | Inject error result, continue turn | None (turn continues) |
| `ToolPanic` | Inject error result, continue turn | None (turn continues) |
| `ToolNotFound` | Inject error result, continue turn | None (turn continues) |
| `UserInterrupt` | Graceful exit | `UserInterrupted` |
| `HistoryManager` failure | Panic (programming error) | N/A |

---
