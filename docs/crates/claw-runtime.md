# claw-runtime

> **Layer 1: System Runtime** — Event bus, process management, and multi-agent orchestration  
> 系统运行时 (Layer 1) — 事件总线、进程管理和多智能体编排

[English](#english) | [中文](#chinese)

<a name="english"></a>

System runtime with event bus, process management, and multi-agent orchestration.

---

## Overview

`claw-runtime` provides:
- Event bus for component communication
- Process management for subagents
- IPC routing
- Multi-agent orchestration

---

## Usage

```toml
[dependencies]
claw-runtime = "0.1"
```

```rust
use claw_runtime::{Runtime, EventBus};

let runtime = Runtime::new().await?;

// Subscribe to events
let mut events = runtime.event_bus().subscribe(EventFilter::all());
while let Ok(event) = events.recv().await {
    println!("Event: {:?}", event);
}
```

---

## Event Bus

```rust
use claw_runtime::{EventBus, Event};

let bus = EventBus::new();

// Subscribe
let rx = bus.subscribe(EventFilter::ToolCalls);

// Emit
bus.emit(Event::ToolCalled {
    tool_name: "calculator".to_string(),
    params: json!({"a": 1, "b": 2}),
});
```

---

## Multi-Agent Orchestration

```rust
use claw_runtime::{AgentOrchestrator, AgentConfig};

let orchestrator = AgentOrchestrator::new(runtime);

// Spawn subagent
let config = AgentConfig {
    name: "searcher".to_string(),
    provider: ProviderConfig::default(),
    tools: vec!["web_search".to_string()],
};

let handle = orchestrator.spawn(config).await?;

// Send message
orchestrator.send_message(
    AgentId::main(),
    handle.id(),
    A2AMessage::request("Search for Rust tutorials"),
).await?;

// List agents
for agent in orchestrator.list() {
    println!("{}: {:?}", agent.name, agent.status);
}

// Terminate
orchestrator.terminate(handle, Duration::from_secs(5)).await?;
```

---

## Process Management

```rust
use claw_runtime::{ProcessManager, ProcessConfig};

let manager = ProcessManager::new();

let handle = manager.spawn(ProcessConfig {
    command: "worker".to_string(),
    args: vec!["--task".to_string(), "1".to_string()],
    sandbox: Some(sandbox_config),
}).await?;

// Monitor
let status = manager.wait(handle).await?;
```

---

<a name="chinese"></a>

# claw-runtime

包含事件总线、进程管理和多智能体编排的系统运行时。

---

## 概述

`claw-runtime` 提供：
- 组件通信的事件总线
- 子智能体的进程管理
- IPC 路由
- 多智能体编排

---

## 用法

```toml
[dependencies]
claw-runtime = "0.1"
```

```rust
use claw_runtime::{Runtime, EventBus};

let runtime = Runtime::new().await?;

// 订阅事件
let mut events = runtime.event_bus().subscribe(EventFilter::all());
while let Ok(event) = events.recv().await {
    println!("Event: {:?}", event);
}
```

---

## 事件总线

```rust
use claw_runtime::{EventBus, Event};

let bus = EventBus::new();

// 订阅
let rx = bus.subscribe(EventFilter::ToolCalls);

// 发送
bus.emit(Event::ToolCalled {
    tool_name: "calculator".to_string(),
    params: json!({"a": 1, "b": 2}),
});
```

---

## 多智能体编排

```rust
use claw_runtime::{AgentOrchestrator, AgentConfig};

let orchestrator = AgentOrchestrator::new(runtime);

// 生成子智能体
let config = AgentConfig {
    name: "searcher".to_string(),
    provider: ProviderConfig::default(),
    tools: vec!["web_search".to_string()],
};

let handle = orchestrator.spawn(config).await?;

// 发送消息
orchestrator.send_message(
    AgentId::main(),
    handle.id(),
    A2AMessage::request("Search for Rust tutorials"),
).await?;

// 列出智能体
for agent in orchestrator.list() {
    println!("{}: {:?}", agent.name, agent.status);
}

// 终止
orchestrator.terminate(handle, Duration::from_secs(5)).await?;
```

---

## 进程管理

```rust
use claw_runtime::{ProcessManager, ProcessConfig};

let manager = ProcessManager::new();

let handle = manager.spawn(ProcessConfig {
    command: "worker".to_string(),
    args: vec!["--task".to_string(), "1".to_string()],
    sandbox: Some(sandbox_config),
}).await?;

// 监控
let status = manager.wait(handle).await?;
```
