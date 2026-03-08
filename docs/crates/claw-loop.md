---
title: claw-loop
description: Agent loop engine, history management, stop conditions
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---



Agent loop engine for multi-turn conversations.

---

## Overview

`claw-loop` manages the conversation lifecycle:
- Message history
- Tool call loops
- Stop conditions
- Context window management

---

## Usage

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

## Configuration

```rust
let agent = AgentLoop::builder()
    // Provider (required)
    .provider(my_provider)
    // Tools (optional)
    .tools(tool_registry)
    // System prompt
    .system_prompt("You are a helpful assistant.")
    // Stop conditions
    .max_turns(10)
    .token_budget(8000)
    .build();
```

---

## Stop Conditions

```rust
use claw_loop::{MaxTurns, TokenBudget, NoToolCall};

let agent = AgentLoop::builder()
    .stop_condition(Box::new(MaxTurns(10)))
    .stop_condition(Box::new(TokenBudget(8000)))
    .stop_condition(Box::new(NoToolCall))
    .build();
```

Built-in conditions:
- `MaxTurns(n)` — Stop after n turns
- `TokenBudget(n)` — Stop when total tokens ≥ n (0 = unlimited)
- `NoToolCall` — Registered for logging; loop handles this exit path directly

Custom condition:

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

## History Management

```rust
use claw_loop::history::{HistoryManager, SqliteHistory};

// Persistent history
let history = SqliteHistory::new("./conversation.db").await?;

let agent = AgentLoop::builder()
    .history(history)
    .build();
```

---

## Streaming Responses

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

## Multi-Turn Conversation

```rust
// Preserves context between calls
let response1 = agent.run("My name is Alice.").await?;
let response2 = agent.run("What's my name?").await?; // "Alice"

// Access history
for message in agent.history().messages() {
    println!("{:?}: {}", message.role, message.content);
}

// Clear history
agent.clear_history();
```

---
