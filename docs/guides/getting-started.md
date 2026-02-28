---
title: Getting Started with claw-kernel
description: Build your first agent using claw-kernel
status: design-phase
version: "0.1.0"
last_updated: "2026-02-28"
language: bilingual
---

> **Project Status**: Design/Planning Phase — This guide shows target API design, not working code.

[English](#english) | [中文](#chinese)

<a name="english"></a>

# Getting Started with claw-kernel

This guide will help you build your first agent using claw-kernel.

claw-kernel is a lightweight framework for building LLM-powered agents. It provides core components like provider interfaces, tool systems, and agent loops at Layer 1-3.

> [Warning]  **Note**: This guide shows the **target API design**. The `claw-kernel` crate is not yet implemented. See [BUILD_PLAN.md](../../BUILD_PLAN.md) for the implementation roadmap.

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
claw-kernel = { version = "0.1", features = ["engine-lua"] }
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

<a name="chinese"></a>

# claw-kernel 入门指南

本指南将帮助你使用 claw-kernel 构建你的第一个智能体。

claw-kernel 是一个用于构建大语言模型（LLM）驱动的智能体的轻量级框架。它在 Layer 1-3 提供核心组件，如提供商接口、工具系统和智能体循环。

---

## 前置条件

- Rust 工具链（稳定版，**1.83+**）
- 至少一个 LLM 提供商的 API 密钥（Anthropic、OpenAI 或 Ollama）

---

## 安装

### 1. 创建新的 Rust 项目

```bash
cargo new my-agent
cd my-agent
```

### 2. 添加依赖

编辑 `Cargo.toml`：

```toml
[dependencies]
claw-kernel = { version = "0.1", features = ["engine-lua"] }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
```

### 3. 设置环境

创建 `.env` 文件：

```bash
# 使用 Anthropic (Claude)
ANTHROPIC_API_KEY=sk-ant-...

# 或使用 OpenAI
OPENAI_API_KEY=sk-...

# 或使用 Ollama 本地模型
OLLAMA_BASE_URL=http://localhost:11434
```

---

## 你的第一个智能体

创建 `src/main.rs`：

```rust
use claw_kernel::{provider::AnthropicProvider, loop_::AgentLoop, tools::ToolRegistry};
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 从环境变量初始化提供商
    let provider = AnthropicProvider::from_env()?;
    
    // 创建空工具注册表
    let tools = ToolRegistry::new();
    
    // 构建智能体循环
    let mut agent = AgentLoop::builder()
        .provider(provider)
        .tools(tools)
        .build();
    
    // 运行单次对话回合
    let response = agent.run("你好，你能做什么？").await?;
    println!("智能体: {}", response.content);
    
    Ok(())
}
```

运行：

```bash
cargo run
```

---

## 添加工具

工具赋予你的智能体能力。让我们添加一个简单的计算器。

创建 `tools/calculator.lua`：

```lua
-- 计算器工具
-- @name calculator
-- @description 执行基本数学运算
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
            return { success = false, error = "除零错误" }
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

更新 `main.rs`：

```rust
use claw_kernel::tools::ToolRegistry;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = AnthropicProvider::from_env()?;
    
    // 从目录加载工具
    let mut tools = ToolRegistry::new();
    tools.load_from_directory(PathBuf::from("tools")).await?;
    
    let mut agent = AgentLoop::builder()
        .provider(provider)
        .tools(tools)
        .build();
    
    // 现在智能体可以使用计算器工具了
    let response = agent.run("25 * 17 等于多少？").await?;
    println!("智能体: {}", response.content);
    
    Ok(())
}
```

---

## 多轮对话

用于交互式智能体：

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
    
    println!("智能体已就绪。输入 'exit' 退出。");
    
    while let Some(line) = lines.next_line().await? {
        if line.trim() == "exit" {
            break;
        }
        
        let response = agent.run(&line).await?;
        println!("智能体: {}", response.content);
        
        // 打印使用的工具调用
        for call in &response.tool_calls {
            println!("  [使用工具: {}]", call.name);
        }
    }
    
    Ok(())
}
```

---

## 使用不同的 LLM 提供商

### OpenAI

```rust
use claw_kernel::provider::OpenAIProvider;

let provider = OpenAIProvider::from_env()?;
```

### Ollama（本地模型）

```rust
use claw_kernel::provider::{OllamaProvider, OllamaConfig};

let provider = OllamaProvider::new(OllamaConfig {
    base_url: "http://localhost:11434".to_string(),
    model: "llama2".to_string(),
});
```

---

## 下一步

- [编写自定义工具](writing-tools.md) — 构建更强大的工具
- [安全模式配置](safe-mode.md) — 保护你的智能体
- [扩展能力指南](extension-capabilities.md) — 使用内核热加载和动态注册功能
