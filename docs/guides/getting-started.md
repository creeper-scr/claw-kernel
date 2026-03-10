---
title: Getting Started with claw-kernel
description: Build your first agent using claw-kernel
status: implemented
version: "1.4.1"
last_updated: "2026-03-10"
language: en
---


# Getting Started with claw-kernel

> ⚠️ **Pre-release notice:** v0.4.0 is a beta and may be unstable. APIs are subject to change without notice.

This guide will help you build your first agent using claw-kernel.

claw-kernel is a lightweight Rust framework for building LLM-powered agents. It provides core components like provider interfaces, tool systems, and agent loops at Layer 1–3.

> **Note**: This guide documents the implemented API in v1.4.1.

---

## Prerequisites

- Rust toolchain (stable, **1.83+**)
- API key for at least one LLM provider (Anthropic, OpenAI, or a local Ollama instance)

---

## Installation

### 1. Create a New Rust Project

```bash
cargo new my-agent
cd my-agent
```

### 2. Add Dependencies

Edit `Cargo.toml`:

```toml
[dependencies]
claw-kernel = { version = "1.4", features = ["engine-lua"] }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
async-trait = "0.1"
serde_json = "1"
```

### 3. Set Up Environment

Create a `.env` file (or export directly):

```bash
# For Anthropic (Claude)
ANTHROPIC_API_KEY=sk-ant-...

# Or for OpenAI
OPENAI_API_KEY=sk-...

# Ollama runs locally — no API key needed
```

---

## Your First Agent

Create `src/main.rs`:

```rust
use std::sync::Arc;
use claw_kernel::prelude::*;
use claw_kernel::provider::AnthropicProvider;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize provider from environment variable ANTHROPIC_API_KEY
    let provider = Arc::new(AnthropicProvider::from_env()?) as Arc<dyn LLMProvider>;

    // Create an empty tool registry
    let tools = Arc::new(ToolRegistry::new());

    // Build the agent loop
    let mut agent = AgentLoopBuilder::new()
        .with_provider(provider)
        .with_tools(tools)
        .with_max_turns(10)
        .build()?;

    // Run a single query
    let result = agent.run("Hello, what can you do?").await?;
    println!("Agent: {}", result.content);

    Ok(())
}
```

Run it:

```bash
cargo run
```

### What's in `AgentResult`

The value returned by `agent.run()` is an `AgentResult` with these fields:

| Field | Type | Description |
|-------|------|-------------|
| `content` | `String` | The assistant's final text response |
| `turns` | `usize` | Number of turns executed |
| `finish_reason` | `FinishReason` | Why the loop stopped (`MaxTurns`, `StopSequence`, etc.) |
| `last_message` | `Option<Message>` | The last message object (role + content) |
| `usage` | `TokenUsage` | Token counts: `prompt_tokens`, `completion_tokens`, `total_tokens` |

---

## Adding Tools

Tools give your agent capabilities beyond conversation. Implement the `Tool` trait:

```rust
use std::sync::Arc;
use claw_kernel::prelude::*;
use claw_kernel::provider::AnthropicProvider;
use claw_kernel::tools::{Tool, ToolContext, ToolResult, ToolSchema, PermissionSet};
use async_trait::async_trait;

// ── Tool definition ────────────────────────────────────────────────────────────

struct CalculatorTool {
    schema: ToolSchema,
    perms: PermissionSet,
}

impl CalculatorTool {
    fn new() -> Self {
        let schema = ToolSchema::new(
            "calculator",
            "Add two numbers together",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "a": { "type": "number", "description": "First number" },
                    "b": { "type": "number", "description": "Second number" }
                },
                "required": ["a", "b"]
            }),
        );
        Self {
            schema,
            perms: PermissionSet::minimal(),
        }
    }
}

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str { "calculator" }
    fn description(&self) -> &str { "Add two numbers together" }
    fn schema(&self) -> &ToolSchema { &self.schema }
    fn permissions(&self) -> &PermissionSet { &self.perms }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let a = args.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = args.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
        ToolResult::ok(
            serde_json::json!({ "result": a + b }),
            0,
        )
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = Arc::new(AnthropicProvider::from_env()?) as Arc<dyn LLMProvider>;

    let tools = Arc::new(ToolRegistry::new());
    tools.register(Box::new(CalculatorTool::new()))?;

    let mut agent = AgentLoopBuilder::new()
        .with_provider(provider)
        .with_tools(tools)
        .with_max_turns(10)
        .build()?;

    let result = agent.run("What is 25 * 17?").await?;
    println!("Agent: {}", result.content);

    Ok(())
}
```

Key points:
- `ToolRegistry::new()` creates an empty registry.
- `tools.register(Box::new(MyTool::new()))?` registers a tool (returns an error if the name is already taken).
- `PermissionSet::minimal()` grants no special OS permissions.

---

## Multi-Turn Conversation

`AgentLoopBuilder` maintains history across `.run()` calls automatically. For an interactive REPL:

```rust
use std::sync::Arc;
use claw_kernel::prelude::*;
use claw_kernel::provider::AnthropicProvider;
use tokio::io::{self, AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = Arc::new(AnthropicProvider::from_env()?) as Arc<dyn LLMProvider>;
    let tools = Arc::new(ToolRegistry::new());

    let mut agent = AgentLoopBuilder::new()
        .with_provider(provider)
        .with_tools(tools)
        .with_system_prompt("You are a helpful assistant. Be concise.")
        .with_max_turns(20)
        .build()?;

    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();

    println!("Agent ready. Type 'exit' to quit.");

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line == "exit" {
            break;
        }
        if line.is_empty() {
            continue;
        }

        let result = agent.run(&line).await?;
        println!("Agent: {}", result.content);
        println!("  [turns: {}, finish: {:?}]", result.turns, result.finish_reason);
    }

    // Inspect full conversation history
    println!("\nConversation history ({} messages):", agent.history().len());
    for (i, msg) in agent.history().iter().enumerate() {
        println!("  [{}] {:?}: {}", i, msg.role, &msg.content[..msg.content.len().min(80)]);
    }

    Ok(())
}
```

---

## Using Different LLM Providers

### Anthropic (Claude)

```rust
use std::sync::Arc;
use claw_kernel::prelude::*;
use claw_kernel::provider::AnthropicProvider;

let provider = Arc::new(AnthropicProvider::from_env()?) as Arc<dyn LLMProvider>;
// Reads ANTHROPIC_API_KEY from environment
```

### OpenAI

```rust
use std::sync::Arc;
use claw_kernel::prelude::*;
use claw_kernel::provider::OpenAIProvider;

let provider = Arc::new(OpenAIProvider::from_env()?) as Arc<dyn LLMProvider>;
// Reads OPENAI_API_KEY from environment
```

### Ollama (Local Models — No API Key Required)

Ollama is ideal for local development and offline use:

```rust
use std::sync::Arc;
use claw_kernel::prelude::*;
use claw_kernel::provider::OllamaProvider;

// Connect to local Ollama with model name
let provider = Arc::new(OllamaProvider::new("llama3.2")) as Arc<dyn LLMProvider>;
// Default endpoint: http://localhost:11434
// Make sure `ollama serve` is running locally
```

---

## What `use claw_kernel::prelude::*` Exports

The prelude re-exports the most commonly used types so you don't need to spell out full module paths:

| Symbol | Description |
|--------|-------------|
| `AgentLoopBuilder` | Fluent builder for `AgentLoop` |
| `AgentLoop` | The agent loop runtime |
| `AgentResult` | Return type of `agent.run()` |
| `ToolRegistry` | Container for registered tools |
| `LLMProvider` | Trait for provider implementations |
| `Message` | A single conversation message |
| `FinishReason` | Why the agent loop stopped |
| `TokenUsage` | Token count breakdown |

Provider types (`AnthropicProvider`, `OpenAIProvider`, `OllamaProvider`) are **not** in the prelude — import them explicitly from `claw_kernel::provider`.

Tool implementation types (`Tool`, `ToolSchema`, `ToolResult`, `ToolContext`, `PermissionSet`) come from `claw_kernel::tools`.

---

## Next Steps

- [Writing Custom Tools](writing-tools.md) — Build more powerful Rust and Lua tools
- [Safe Mode Configuration](safe-mode.md) — Secure your agent's filesystem and network access
- [Extension Capabilities](extension-capabilities.md) — Hot-loading and dynamic tool registration

---
