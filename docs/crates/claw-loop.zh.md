---
title: claw-loop
description: Agent loop engine, history management, stop conditions
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](claw-loop.md)


# claw-loop

多轮对话的智能体循环引擎。

---

## 概述

`claw-loop` 管理对话生命周期：
- 消息历史
- 工具调用循环
- 停止条件
- 上下文窗口管理

---

## 用法

```toml
[dependencies]
claw-loop = "0.1"
```

```rust
use claw_loop::AgentLoop;
use claw_provider::AnthropicProvider;
use claw_tools::ToolRegistry;

let provider = AnthropicProvider::from_env()?;
let tools = ToolRegistry::new();

let mut agent = AgentLoop::builder()
    .provider(provider)
    .tools(tools)
    .max_turns(10)
    .build();

let response = agent.run("Hello!").await?;
```

---

## 配置

```rust
let agent = AgentLoop::builder()
    // Provider (必需)
    .provider(my_provider)
    // Tools (可选)
    .tools(tool_registry)
    // 系统提示词
    .system_prompt("You are a helpful assistant.")
    // 停止条件
    .max_turns(10)
    .token_budget(8000)
    // 历史管理
    .history_backend(SqliteHistory::new("./history.db"))
    .summarizer(SlidingWindowSummarizer::new(4000))
    .build();
```

---

## 停止条件

```rust
use claw_loop::conditions::*;

let agent = AgentLoop::builder()
    .stop_condition(MaxTurnsCondition::new(10))
    .stop_condition(TokenBudgetCondition::new(8000))
    .stop_condition(NoToolCallCondition::new())
    .stop_condition(UserInterruptCondition::new(signal_rx))
    .build();
```

内置条件：
- `MaxTurnsCondition` — 限制对话轮数
- `TokenBudgetCondition` — 限制总 token 数
- `NoToolCallCondition` — 如果上一轮未使用工具则停止
- `UserInterruptCondition` — 收到信号时停止

自定义条件：

```rust
use claw_loop::{StopCondition, LoopState};

pub struct MyCondition;

impl StopCondition for MyCondition {
    fn should_stop(&self, state: &LoopState) -> bool {
        state.turn_count > 5 && state.last_message.as_ref().map(|m| m.content.contains("DONE")).unwrap_or(false)
    }
}
```

---

## 历史管理

```rust
use claw_loop::history::{HistoryManager, SqliteHistory};

// 持久化历史
let history = SqliteHistory::new("./conversation.db").await?;

let agent = AgentLoop::builder()
    .history(history)
    .build();
```

---

## 流式响应

```rust
let mut stream = agent.stream_run("Hello!").await?;

while let Some(chunk) = stream.next().await {
    match chunk {
        StreamChunk::Text(text) => print!("{}", text),
        StreamChunk::ToolStart(name) => println!("\n[Using tool: {}]", name),
        StreamChunk::ToolResult(result) => println!("[Tool done]"),
    }
}
```

---

## 多轮对话

```rust
// 在调用之间保持上下文
let response1 = agent.run("My name is Alice.").await?;
let response2 = agent.run("What's my name?").await?; // "Alice"

// 访问历史
for message in agent.history().messages() {
    println!("{:?}: {}", message.role, message.content);
}

// 清除历史
agent.clear_history();
```
