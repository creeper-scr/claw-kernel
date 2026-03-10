---
title: Getting Started with claw-kernel
description: Build your first agent using claw-kernel
status: implemented
version: "1.0.0"
last_updated: "2026-03-01"
language: en
---



# Getting Started with claw-kernel

This guide will help you build your first agent using claw-kernel.

claw-kernel is a lightweight framework for building LLM-powered agents. It provides core components like provider interfaces, tool systems, and agent loops at Layer 1-3.

> [Info] **Note**: This guide documents the implemented API in v1.0.0.

---

## Prerequisites

- Rust toolchain (stable, **1.83+**)
- API key for at least one LLM provider (Anthropic, OpenAI, or Ollama)

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
claw-kernel = { version = "1.0", features = ["engine-lua"] }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
```

### 3. Set Up Environment

Create a `.env` file:

```bash
# For Anthropic (Claude)
ANTHROPIC_API_KEY=sk-ant-...

# Or for OpenAI
OPENAI_API_KEY=sk-...

# Or for local models via Ollama
OLLAMA_BASE_URL=http://localhost:11434
```

---

## Your First Agent

Create `src/main.rs`:

```rust
use claw_kernel::{provider::AnthropicProvider, loop_::AgentLoop, tools::ToolRegistry};
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize provider from environment
    let provider = AnthropicProvider::from_env()?;
    
    // Create empty tool registry
    let tools = ToolRegistry::new();
    
    // Build agent loop
    let mut agent = AgentLoop::builder()
        .provider(provider)
        .tools(tools)
        .build();
    
    // Run a single conversation turn
    let response = agent.run("Hello, what can you do?").await?;
    println!("Agent: {}", response.content);
    
    Ok(())
}
```

Run it:

```bash
cargo run
```

---

## Adding Tools

Tools give your agent capabilities. Let's add a simple calculator.

Create `tools/calculator.lua`:

```lua
-- Calculator tool
-- @name calculator
-- @description Perform basic math operations
-- @permissions none
-- @schema {
--   "type": "object",
--   "properties": {
--     "operation": { 
--       "type": "string", 
--       "enum": ["add", "subtract", "multiply", "divide"]
--     },
--     "a": { "type": "number" },
--     "b": { "type": "number" }
--   },
--   "required": ["operation", "a", "b"]
-- }

local M = {}

function M.execute(params)
    local op = params.operation
    local a = params.a
    local b = params.b
    
    local result
    if op == "add" then
        result = a + b
    elseif op == "subtract" then
        result = a - b
    elseif op == "multiply" then
        result = a * b
    elseif op == "divide" then
        if b == 0 then
            return { success = false, error = "Division by zero" }
        end
        result = a / b
    end
    
    return {
        success = true,
        result = result
    }
end

return M
```

Update `main.rs`:

```rust
use claw_kernel::tools::ToolRegistry;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = AnthropicProvider::from_env()?;
    
    // Load tools from directory
    let mut tools = ToolRegistry::new();
    tools.load_from_directory(PathBuf::from("tools")).await?;
    
    let mut agent = AgentLoop::builder()
        .provider(provider)
        .tools(tools)
        .build();
    
    // Now the agent can use the calculator tool
    let response = agent.run("What is 25 * 17?").await?;
    println!("Agent: {}", response.content);
    
    Ok(())
}
```

---

## Multi-Turn Conversation

For interactive agents:

```rust
use claw_kernel::loop_::AgentLoop;
use tokio::io::{self, AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = AnthropicProvider::from_env()?;
    let tools = load_tools().await?;
    
    let mut agent = AgentLoop::builder()
        .provider(provider)
        .tools(tools)
        .build();
    
    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();
    
    println!("Agent ready. Type 'exit' to quit.");
    
    while let Some(line) = lines.next_line().await? {
        if line.trim() == "exit" {
            break;
        }
        
        let response = agent.run(&line).await?;
        println!("Agent: {}", response.content);
        
        // Print any tool calls that were made
        for call in &response.tool_calls {
            println!("  [Used tool: {}]", call.name);
        }
    }
    
    Ok(())
}
```

---

## Using Different LLM Providers

### OpenAI

```rust
use claw_kernel::provider::OpenAIProvider;

let provider = OpenAIProvider::from_env()?;
```

### Ollama (Local Models)

```rust
use claw_kernel::provider::{OllamaProvider, OllamaConfig};

let provider = OllamaProvider::new(OllamaConfig {
    base_url: "http://localhost:11434".to_string(),
    model: "llama2".to_string(),
});
```

---

## Next Steps

- [Writing Custom Tools](writing-tools.md) — Build more powerful tools
- [Safe Mode Configuration](safe-mode.md) — Secure your agent
- [Extension Capabilities](extension-capabilities.md) — Use kernel hot-loading and dynamic registration

---
