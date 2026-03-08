---
title: claw-runtime
description: Event bus, async runtime, multi-agent orchestration
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---



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
use claw_runtime::{Runtime, EventBus, EventFilter};

let runtime = Runtime::new("/tmp/claw.sock");

// Subscribe to all events
let mut events = runtime.event_bus.subscribe();
while let Ok(event) = events.recv().await {
    println!("Event: {:?}", event);
}

// Or subscribe to a specific category
let mut tool_events = runtime.event_bus.subscribe_with_filter(EventFilter::ToolCalls);
```

---

## Event Bus

```rust
use claw_runtime::{EventBus, Event, EventFilter};

let bus = EventBus::new();

// Subscribe to all events
let mut rx = bus.subscribe();

// Subscribe with a declarative filter
let mut tool_rx = bus.subscribe_with_filter(EventFilter::ToolCalls);

// Subscribe with a custom closure predicate
let mut agent_rx = bus.subscribe_filtered(|e| matches!(e, Event::AgentStarted { .. }));

// Publish
bus.publish(Event::ToolCalled {
    agent_id: agent_id.clone(),
    tool_name: "calculator".to_string(),
    call_id: "call-001".to_string(),
}).unwrap();
```

---

## Multi-Agent Orchestration

```rust
use claw_runtime::{AgentOrchestrator, AgentConfig, AgentId, EventBus};
use claw_pal::{ProcessConfig, TokioProcessManager};
use std::sync::Arc;

let bus = Arc::new(EventBus::new());
let orchestrator = AgentOrchestrator::new(Arc::clone(&bus));

// Register an in-process agent
let config = AgentConfig::new("searcher");
let handle = orchestrator.register(config)?;

// Or spawn an out-of-process agent
let agent_config = AgentConfig::new("worker");
let process_config = ProcessConfig::new("worker-bin".to_string())
    .with_arg("--task".to_string());
let handle = orchestrator.spawn(agent_config, process_config).await?;

// Query agents
let ids = orchestrator.agent_ids();
let count = orchestrator.agent_count();
let info = orchestrator.agent_info(&handle.agent_id);

// Terminate an agent (SIGTERM → SIGKILL after grace period)
orchestrator.terminate(&handle.agent_id, Duration::from_secs(5)).await?;
```

---

## Process Management

```rust
use claw_runtime::{Runtime, ProcessConfig};
use claw_pal::TokioProcessManager;
use claw_pal::traits::ProcessManager as _;
use std::sync::Arc;

let manager = Arc::new(TokioProcessManager::new());

let handle = manager.spawn(ProcessConfig {
    program: "worker".to_string(),
    args: vec!["--task".to_string(), "1".to_string()],
    env: std::collections::HashMap::new(),
    working_dir: None,
}).await?;

// Wrap in ManagedProcess for ergonomic wait/kill
use claw_runtime::ManagedProcess;
let proc = ManagedProcess::new(handle, Arc::clone(&manager));
let status = proc.wait().await?;

// Or associate a process with an agent via the orchestrator
let agent_handle = orchestrator.spawn(agent_config, process_config).await?;
```

---
