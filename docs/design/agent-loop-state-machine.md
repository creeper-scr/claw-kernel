---
title: "Agent Loop State Machine Design"
description: "Detailed design for the claw-loop crate: state machine, turn lifecycle, tool execution, history truncation, and streaming"
status: design-phase
version: "0.1.0"
last_updated: "2026-02-28"
crate: claw-loop
layer: "Layer 2: Agent Kernel Protocol"
---

[English](#english) | [中文](#chinese)

<a name="english"></a>

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

<a name="chinese"></a>

# Agent 循环状态机设计

> **Crate：** `claw-loop`（第 4 阶段）  
> **层级：** 第 2 层 — Agent 内核协议  
> **目的：** 定义 Agent 循环引擎的执行算法、状态转换和内存管理。本文档填补了 BUILD_PLAN.md 留下的空白——BUILD_PLAN.md 指定了数据结构，但未定义运行时行为。

---

## 1. 概述

`AgentLoop` 是 claw-kernel 执行模型的核心。它驱动用户、LLM 提供商和工具集之间的对话。循环持续运行，直到停止条件触发或发生不可恢复的错误。

**关键不变量：**

- 一个*轮次（turn）* = 一次 LLM 调用 + 由此产生的所有工具调用（允许并行执行）。
- 停止条件仅在完整轮次结束后评估，不在轮次中途评估。
- 无论 token 压力多大，系统提示词（system prompt）始终保留在上下文中。
- `get_context(max_tokens)` 永不 panic。如果预算太小，无法容纳系统提示词以外的内容，则只返回系统提示词。
- 单次 LLM 响应中的工具调用通过 `tokio::join_all` 并发执行，受 `max_tool_calls_per_turn` 限制。

---

## 2. 状态机图

```
                         ┌─────────────────────────────────────────────────────┐
                         │               AgentLoop 状态                        │
                         └─────────────────────────────────────────────────────┘

              start / run(user_message)
                         │
                         ▼
                    ┌─────────┐
                    │  空闲   │◄──────────────────────────────────────────────┐
                    │  Idle   │                                               │
                    └────┬────┘                                               │
                         │ 将用户消息追加到历史记录                              │
                         │ 增加 turn_count                                    │
                         ▼                                                    │
                   ┌──────────┐                                               │
                   │  运行中  │                                               │
                   │ Running  │                                               │
                   └────┬─────┘                                               │
                        │ 调用 history.get_context(token_budget)              │
                        │ 向提供商提交上下文                                    │
                        ▼                                                    │
              ┌──────────────────┐                                           │
              │  等待 LLM 响应   │                                           │
              │  AwaitingLLM     │                                           │
              │  （流式或阻塞）   │                                           │
              └────────┬─────────┘                                           │
                       │                                                     │
          ┌────────────┴────────────┐                                        │
          │ 响应包含                │ 响应不包含                               │
          │ tool_calls              │ tool_calls                             │
          ▼                         ▼                                        │
 ┌──────────────────┐    ┌──────────────────────┐                           │
 │  处理工具调用    │    │   评估停止条件        │                           │
 │ ProcessingTool   │    │  EvaluatingStop      │                           │
 │ Calls            │    │  1. MaxTurnsReached  │                           │
 │ （验证、分发）   │    │  2. TokenBudget      │                           │
 └────────┬─────────┘    │  3. NoToolCall       │                           │
          │              │  4. 自定义条件        │                           │
          │              └──────────┬───────────┘                           │
          ▼                         │                                        │
 ┌──────────────────┐               │                                        │
 │  等待工具结果    │    ┌──────────┴──────────┐                            │
 │ AwaitingTool     │    │ 触发停止？          │ 未触发                     │
 │ Results          │    ▼                     ▼                            │
 │ (tokio::join_all)│  ┌────────────┐    ──────────────                    │
 └────────┬─────────┘  │  已终止   │   （循环继续）  ──────────────────────┘
          │            │ Terminated │
          │            └────────────┘
          │ 所有结果已收集
          │ 追加 assistant + tool_result 消息
          ▼
 ┌──────────────────────┐
 │   评估停止条件       │
 │  EvaluatingStop      │
 │  （顺序同上）        │
 └──────────────────────┘

  错误路径（从任意状态）：
  ┌──────────────────────────────────────────────────────────────────────┐
  │  ProviderError ──► 重试最多 config.max_retries 次 ──► Terminated    │
  │  ToolTimeout   ──► 注入错误结果 ──► 继续当前轮次                    │
  │  ToolPanic     ──► 注入错误结果 ──► 继续当前轮次                    │
  │  UserInterrupt ──► Terminated (FinishReason::UserInterrupted)       │
  └──────────────────────────────────────────────────────────────────────┘
```

**状态说明：**

| 状态 | 描述 |
|------|------|
| `Idle`（空闲） | 循环就绪，无活跃轮次，等待 `run()` 调用。 |
| `Running`（运行中） | 轮次已启动，正在准备历史上下文。 |
| `AwaitingLLM`（等待 LLM） | 向提供商发出的 HTTP 请求正在进行中，可能正在发出流式 chunk。 |
| `ProcessingToolCalls`（处理工具调用） | 已收到 LLM 响应，正在验证和分发工具调用。 |
| `AwaitingToolResults`（等待工具结果） | 所有工具 future 并发运行，等待全部完成或超时。 |
| `EvaluatingStop`（评估停止条件） | 本轮次所有结果已收集，按顺序检查停止条件。 |
| `Terminated`（已终止） | 循环已退出，`FinishReason` 已设置，不再运行新轮次。 |

---

## 3. 轮次生命周期

单个轮次按严格顺序经历以下阶段：

```
第 N 轮次生命周期：
─────────────────────────────────────────────────────────────────────────

阶段 1：上下文准备
  - history.get_context(token_budget) → Vec<Message>
  - 如果 token 数量 > 预算的 80% 且已配置 summarizer：
      对最旧的轮次调用 summarizer（见第 7 节）
  - 如果存在 system_prompt 且尚未在上下文中：前置添加

阶段 2：LLM 调用
  - 向提供商提交上下文
  - 如果 enable_streaming = true：
      在缓冲完整响应的同时，将 Delta chunk 转发到 EventBus
  - 等待完整响应（文本 + tool_calls）

阶段 3：工具分发（如果存在 tool_calls）
  - 验证：tool_calls.len() <= max_tool_calls_per_turn
    - 如果超出：截断为前 max_tool_calls_per_turn 个，记录警告
  - 对每个 tool_call：在 ToolRegistry 中查找工具
    - 如果工具未找到：立即注入 ToolNotFound 错误结果
  - 并发分发所有有效工具调用：tokio::join_all(futures)
  - 每个 future 包装 timeout(config.tool_timeout)
    - 超时时：为该调用注入 ToolTimeout 错误结果
    - 其他调用不受影响，继续执行

阶段 4：历史更新
  - 将 assistant 消息（文本 + tool_calls）追加到历史记录
  - 将所有 tool_result 消息追加到历史记录
  - 更新 LoopState：turn_count++、token_usage、last_message、tool_calls_made

阶段 5：停止条件评估
  - 按固定顺序评估（见第 6 节）
  - 如果任何条件触发：设置 FinishReason，转换到 Terminated
  - 如果没有条件触发：转换回 Idle（准备下一轮次）

─────────────────────────────────────────────────────────────────────────
```

---

## 4. 伪代码：`AgentLoop::run()`

这是 `AgentLoop::run(user_message)` 的完整算法。非 Rust 语法，仅为算法步骤。

```
函数 AgentLoop::run(user_message: String) -> Result<AgentResult>

  // --- 初始化 ---
  将 Message { role: User, content: user_message } 追加到 history
  turn_count = 0
  total_tokens = 0
  final_text = ""

  循环：

    // --- 阶段 1：上下文准备 ---
    turn_count += 1

    如果 token_budget 已设置 且 history.estimated_tokens() > 0.8 * token_budget：
      如果 summarizer 已配置：
        history.summarize(summarizer)
      否则：
        history.truncate_to_fit(token_budget)

    context = history.get_context(token_budget ?? MAX_USIZE)
    // context 始终包含 system_prompt + 尽可能多的最近轮次

    // --- 阶段 2：LLM 调用 ---
    如果 enable_streaming：
      response = provider.stream(context)
      当 chunk = response.next()：
        如果 chunk 是 Delta(text)：
          向 EventBus 发出 StreamEvent::Delta(text)
        缓冲 chunk
      full_response = assemble(buffer)
    否则：
      full_response = provider.complete(context)

    // full_response 包含：.text、.tool_calls、.token_usage
    total_tokens += full_response.token_usage.total

    // --- 阶段 3：工具分发 ---
    如果 full_response.tool_calls 不为空：

      calls_to_run = full_response.tool_calls
      如果 calls_to_run.len() > max_tool_calls_per_turn：
        记录警告 "截断工具调用"
        calls_to_run = calls_to_run[0..max_tool_calls_per_turn]

      futures = []
      对每个 call 在 calls_to_run 中：
        tool = tool_registry.get(call.name)
        如果 tool 为 None：
          futures.push(ready(ToolResult::error(call.id, "工具未找到")))
        否则：
          future = async { tool.call(call.arguments) }
          future = timeout(tool_timeout, future)
            .map_err(|_| ToolResult::error(call.id, "超时"))
          futures.push(future)

      tool_results = join_all(futures).await
      // 所有结果已收集，超时的调用有错误结果

    否则：
      tool_results = []

    // --- 阶段 4：历史更新 ---
    将 Message { role: Assistant, content: full_response.text,
                 tool_calls: full_response.tool_calls } 追加到 history

    对每个 result 在 tool_results 中：
      将 Message { role: ToolResult, content: result } 追加到 history

    final_text = full_response.text

    // 更新 LoopState
    loop_state = LoopState {
      turn_count,
      token_usage: TokenUsage { total: total_tokens, ... },
      last_message: Some(最后一条 assistant 消息),
      tool_calls_made: tool_results.len(),
    }

    // --- 阶段 5：停止条件评估 ---
    finish_reason = evaluate_stop_conditions(loop_state, full_response)
    如果 finish_reason 是 Some(reason)：
      返回 Ok(AgentResult {
        content: final_text,
        tool_calls: 本次会话所有工具调用,
        turns: turn_count,
        token_usage: ...,
        finish_reason: reason,
        execution_time: elapsed,
      })

    // 没有停止条件触发，继续下一轮次
    // （循环回到阶段 1）

  结束循环

结束函数
```

---

## 5. 工具调用执行

### 并发模型

单次 LLM 响应中的所有工具调用并发运行。循环在所有工具结果收集完毕（或超时）之前不会启动下一次 LLM 调用。

```
LLM 响应：[tool_call_A, tool_call_B, tool_call_C]
                │              │              │
                ▼              ▼              ▼
         ┌──────────┐  ┌──────────┐  ┌──────────┐
         │ Future A │  │ Future B │  │ Future C │   ← tokio::join_all
         │  超时    │  │  超时    │  │  超时    │
         └────┬─────┘  └────┬─────┘  └────┬─────┘
              │              │              │
              ▼              ▼              ▼
         [result_A]    [result_B]    [timeout_err_C]
              │              │              │
              └──────────────┴──────────────┘
                             │
                    所有结果已收集
                             │
                    追加到历史记录
```

### 工具调用数量限制

如果 `full_response.tool_calls.len() > max_tool_calls_per_turn`：
- 截断为前 `max_tool_calls_per_turn` 个调用。
- 记录警告，包含被丢弃的调用数量。
- 被丢弃的调用永远不会被执行，也不会出现在历史记录中。

### 工具未找到

如果 LLM 响应中的工具名称在 `ToolRegistry` 中不存在：
- 注入立即错误结果：`"工具 '{name}' 在注册表中未找到"`。
- 不中止当前轮次，其他工具调用正常继续。

### 工具超时

如果工具调用超过 `config.tool_timeout`：
- 注入错误结果：`"工具 '{name}' 在 {duration} 后超时"`。
- 取消该工具的异步任务（Tokio 取消）。
- 其他并发工具调用不受影响。

---

## 6. 停止条件评估顺序

停止条件在每个完整轮次结束后按此固定顺序评估。顺序很重要：靠前的条件优先级更高。

```
函数 evaluate_stop_conditions(state: LoopState, response: LLMResponse) -> Option<FinishReason>

  // 1. MaxTurnsReached — 首先检查，始终检查
  如果 config.max_turns 是 Some(max) 且 state.turn_count >= max：
    返回 Some(FinishReason::MaxTurnsReached)

  // 2. TokenBudgetExceeded — 第二检查
  如果 config.token_budget 是 Some(budget) 且 state.token_usage.total >= budget：
    返回 Some(FinishReason::TokenBudgetExceeded)

  // 3. NoToolCallInTurn — 第三检查，仅在已配置时
  //    当 LLM 在本轮次未进行工具调用且该条件已启用时触发
  如果 NoToolCallCondition 在 stop_conditions 中：
    如果 response.tool_calls 为空：
      返回 Some(FinishReason::StopConditionMet("NoToolCallInTurn"))

  // 4. 自定义停止条件 — 最后评估，按注册顺序
  对每个 condition 在 user_defined_stop_conditions 中：
    如果 condition.should_stop(state)：
      返回 Some(FinishReason::StopConditionMet(condition.name()))

  // 没有条件触发
  返回 None

结束函数
```

**为什么是这个顺序？**

- `MaxTurnsReached` 是硬性安全上限，即使 token 预算尚未耗尽也必须触发。
- `TokenBudgetExceeded` 在 `NoToolCall` 之前检查，因为预算超支是资源约束，而非行为信号。
- `NoToolCallInTurn` 是行为信号：LLM 决定它已完成。在自定义条件之前检查，使内置行为可预测。
- 自定义条件最后，因为它们是用户定义的，可能包含任意逻辑，不能覆盖内置安全上限。

---

## 7. HistoryManager 截断算法

`HistoryManager` 负责将对话上下文保持在 token 预算内。算法是**滑动窗口**，始终保留系统提示词。

### Token 估算

Token 数量是估算值，不是精确值。实现使用快速近似方法（例如 `chars / 4` 或兼容 tiktoken 的编码器）。估算必须保守：向上取整，不向下取整。

### `get_context(max_tokens)` 算法

```
函数 get_context(max_tokens: usize) -> Vec<Message>

  messages = []

  // 系统提示词始终包含，无条件
  如果 system_prompt 是 Some(prompt)：
    system_tokens = estimate_tokens(prompt)
    如果 system_tokens >= max_tokens：
      // 预算太小，甚至无法容纳系统提示词。
      // 只返回系统提示词——永不 panic，永不返回空。
      返回 [Message { role: System, content: prompt }]
    messages.push(Message { role: System, content: prompt })
    remaining = max_tokens - system_tokens
  否则：
    remaining = max_tokens

  // 从最新到最旧遍历历史记录（逆序）
  // 包含在剩余预算内能容纳的轮次
  included = []
  对每个 turn 在 history.turns().reversed() 中：
    turn_tokens = estimate_tokens(turn)
    如果 turn_tokens <= remaining：
      included.prepend(turn)
      remaining -= turn_tokens
    否则：
      中断  // 更旧的轮次也不会更小

  messages.extend(included)
  返回 messages

结束函数
```

### `truncate_to_fit(max_tokens)` 算法

在 token 使用量超过预算 80% 时主动调用。

```
函数 truncate_to_fit(max_tokens: usize)

  当 history.estimated_tokens() > max_tokens：
    oldest_pair = history.pop_oldest_user_assistant_pair()
    如果 oldest_pair 为 None：
      中断  // 没有可丢弃的内容（只剩系统提示词）

  // 截断后，历史记录适合 max_tokens
  // 系统提示词永远不会被弹出

结束函数
```

### `summarize(summarizer)` 算法

当 token 数量超过预算 80% 且已配置 `Summarizer` 时调用。摘要化将旧轮次压缩为单个摘要消息，为新轮次释放空间。

```
函数 summarize(summarizer: &dyn Summarizer)

  // 识别要摘要化的最旧 N 个轮次
  // 保留最近 K 个轮次完整（K = summarizer.keep_recent_turns()）
  // 默认 K = 4（最后 2 个 user+assistant 对）

  turns_to_summarize = history.turns()[0 .. len - K]

  如果 turns_to_summarize 为空：
    返回  // 没有可摘要化的内容

  summary_text = summarizer.summarize(turns_to_summarize).await
  // summarizer 内部进行 LLM 调用，使用独立的提供商实例
  // 以避免递归进入主循环

  // 用单个摘要消息替换被摘要化的轮次
  history.replace_turns(
    turns_to_summarize,
    Message {
      role: System,
      content: format!("[对话摘要：{}]", summary_text),
    }
  )

  // 摘要化后，历史记录更小。
  // 如果仍超出预算，调用 truncate_to_fit() 作为回退。
  如果 history.estimated_tokens() > token_budget：
    truncate_to_fit(token_budget)

结束函数
```

### Summarizer 的调用时机

Summarizer 在轮次开始时（阶段 1）、LLM 调用之前调用，条件为：

```
history.estimated_tokens() > 0.80 * config.token_budget
且 config.summarizer 是 Some(_)
```

80% 阈值在预算完全耗尽之前给 summarizer 留出工作空间。如果 summarizer 本身失败（提供商错误、超时），循环静默回退到 `truncate_to_fit()`。

### 无 Summarizer 时的回退

当 `config.summarizer` 为 `None` 且 token 数量超过预算 80% 时：

```
history.truncate_to_fit(token_budget)
```

丢弃最旧的 user+assistant 对，直到历史记录适合预算。系统提示词永远不会被丢弃。

---

## 8. 流式输出

当 `config.enable_streaming = true` 时，循环在等待完整响应的同时向调用者发出文本 delta chunk。

### 机制

循环使用 `mpsc` 通道（Tokio）转发 chunk：

```
提供商流 ──► 缓冲区 ──► 组装完整响应（用于工具调用提取）
              │
              └──► 向 EventBus 发出 Delta chunk（供调用者消费）
```

两条路径并发运行。循环不等待调用者消费 chunk 后再继续。通道有界（默认容量：64 个 chunk）。如果调用者处理慢导致通道满，循环通过等待发送来施加背压。

### 流式事件

```
StreamEvent::TurnStart { turn: usize }
StreamEvent::Delta { text: String }
StreamEvent::ToolCallStart { id: String, name: String }
StreamEvent::ToolCallEnd { id: String, result: ToolResult }
StreamEvent::TurnEnd { finish_reason: Option<FinishReason> }
```

### 流式与工具调用

循环在分发工具调用之前缓冲完整的 LLM 响应。这是必要的，因为：
- 工具调用参数可能跨越多个 delta chunk。
- 循环需要完整的 JSON 来验证和分发工具调用。

文本部分的 Delta chunk 在到达时立即转发。工具调用 chunk 静默缓冲（不作为 delta 转发）。

### 非流式模式

当 `enable_streaming = false` 时：
- 不发出 `StreamEvent::Delta` 事件。
- 仍然发出 `StreamEvent::TurnStart` 和 `StreamEvent::TurnEnd`。
- 仍然发出工具调用事件。

---

## 9. 错误处理

### ProviderError（提供商错误）

当 LLM 提供商的 HTTP 调用失败时（网络错误、速率限制、服务器错误）发生 `ProviderError`。

```
重试策略：
  - 第 1 次：立即重试
  - 第 2 次：等待 1 秒
  - 第 3 次：等待 4 秒
  - 第 4 次及以后：等待 16 秒（上限）
  - 最大重试次数：config.max_provider_retries（默认：3）

达到最大重试次数后：
  - 设置 FinishReason::Error(AgentError::ProviderFailed { ... })
  - 转换到 Terminated
  - 从 run() 返回 Err(AgentError)
```

重试策略使用带抖动的指数退避。循环不对 `4xx` 错误重试（`429 Too Many Requests` 除外）。

### ToolTimeout（工具超时）

超过 `config.tool_timeout` 的工具调用在轮次内处理：

```
- 取消超时工具的异步任务
- 注入错误结果：ToolResult::error(id, "在 {duration} 后超时")
- 继续收集其他并发工具调用的结果
- 轮次正常完成，历史记录中包含错误结果
- LLM 在下一轮次的上下文中看到超时错误，可以决定如何处理
```

### ToolPanic（工具 panic）

如果工具实现发生 panic（通过工具执行器中的 `catch_unwind` 捕获）：

```
- 注入错误结果：ToolResult::error(id, "工具 panic：{message}")
- 与 ToolTimeout 相同方式继续
```

### UserInterrupt（用户中断）

循环在每个轮次开始时（LLM 调用之前）和每批工具完成后检查取消信号：

```
如果 cancellation_token.is_cancelled()：
  设置 FinishReason::UserInterrupted
  转换到 Terminated
  返回 Ok(AgentResult { finish_reason: UserInterrupted, ... })
```

注意：`UserInterrupted` 返回 `Ok`，而非 `Err`。调用者收到包含部分对话的有效 `AgentResult`。

### 错误状态汇总

| 错误 | 恢复方式 | FinishReason |
|------|----------|--------------|
| `ProviderError`（可重试） | 指数退避重试 | 达到最大重试次数后 `Error(ProviderFailed)` |
| `ProviderError`（4xx 非 429） | 不重试 | 立即 `Error(ProviderFailed)` |
| `ToolTimeout` | 注入错误结果，继续轮次 | 无（轮次继续） |
| `ToolPanic` | 注入错误结果，继续轮次 | 无（轮次继续） |
| `ToolNotFound` | 注入错误结果，继续轮次 | 无（轮次继续） |
| `UserInterrupt` | 优雅退出 | `UserInterrupted` |
| `HistoryManager` 失败 | Panic（编程错误） | 不适用 |
