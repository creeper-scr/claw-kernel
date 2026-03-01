---
title: claw-runtime
description: Event bus, async runtime, multi-agent orchestration
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](claw-runtime.zh.md)


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
