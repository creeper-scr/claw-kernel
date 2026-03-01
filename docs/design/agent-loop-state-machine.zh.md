---
title: Agent 循环状态机设计
description: Detailed design for the claw-loop crate: state machine, turn lifecycle, tool execution, history truncation, and streaming
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](agent-loop-state-machine.md)


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
