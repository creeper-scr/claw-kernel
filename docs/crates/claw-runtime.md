---
title: claw-runtime
description: Event bus, async runtime, multi-agent orchestration
status: implemented
version: "0.1.0"
last_updated: "2026-03-09"
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

## Runtime Structure

The `Runtime` struct is the top-level composition root:

```rust
pub struct Runtime {
    pub event_bus: Arc<EventBus>,
    pub orchestrator: Arc<AgentOrchestrator>,
    pub ipc_router: Arc<IpcRouter>,
    pub process_manager: Arc<TokioProcessManager>,
}

impl Runtime {
    /// Create new Runtime with the given IPC endpoint
    pub fn new(ipc_endpoint: impl Into<String>) -> Self;
    
    /// Start accepting IPC connections
    pub async fn start(&self) -> Result<(), RuntimeError>;
    
    /// Broadcast shutdown event
    pub fn shutdown(&self) -> Result<(), RuntimeError>;
}
```

---

## Usage

```toml
[dependencies]
claw-runtime = "0.1"
```

```rust
use claw_runtime::{Runtime, EventBus, EventFilter};

// Create runtime with IPC endpoint
let runtime = Runtime::new("/tmp/claw.sock");

// Subscribe to all events
let mut events = runtime.event_bus.subscribe();
while let Ok(event) = events.recv().await {
    println!("Event: {:?}", event);
}

// Or subscribe to a specific category
let mut tool_events = runtime.event_bus.subscribe_with_filter(EventFilter::ToolCalls);

// Available filters: All, AgentLifecycle, ToolCalls, LlmRequests, MemoryEvents, A2A, ShutdownOnly
let mut agent_events = runtime.event_bus.subscribe_with_filter(EventFilter::AgentLifecycle);
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
use claw_pal::{ProcessConfig, TokioProcessManager, ExecutionMode};
use std::sync::Arc;
use std::time::Duration;

let bus = Arc::new(EventBus::new());
let orchestrator = AgentOrchestrator::new(Arc::clone(&bus));

// Register an in-process agent (no OS process spawned)
let config = AgentConfig::new("searcher");
let handle = orchestrator.register(config)?;

// Unregister an in-process agent
orchestrator.unregister(&handle.agent_id, "task completed")?;

// Or spawn an out-of-process agent
let agent_config = AgentConfig::new("worker")
    .with_mode(ExecutionMode::Safe)
    .with_meta("region", "us-east");
let process_config = ProcessConfig::new("worker-bin".to_string())
    .with_arg("--task".to_string());
let handle = orchestrator.spawn(agent_config, process_config).await?;

// Query agents
let ids = orchestrator.agent_ids();
let count = orchestrator.agent_count();
let info = orchestrator.agent_info(&handle.agent_id);

// Terminate an agent gracefully (SIGTERM → SIGKILL after grace period)
orchestrator.terminate(&handle.agent_id, Duration::from_secs(5)).await?;

// Kill an agent immediately (SIGKILL / TerminateProcess)
orchestrator.kill(&handle.agent_id).await?;
```

---

## Process Management

```rust
use claw_runtime::{Runtime, ProcessConfig};
use claw_pal::TokioProcessManager;
use claw_pal::traits::ProcessManager as _;
use std::sync::Arc;
use std::collections::HashMap;

let manager = Arc::new(TokioProcessManager::new());

// Create process config with builder methods
let handle = manager.spawn(
    ProcessConfig::new("worker".to_string())
        .with_arg("--task".to_string())
        .with_arg("1".to_string())
        .with_env("KEY".to_string(), "value".to_string())
).await?;

// Wait for process to complete
let status = manager.wait(handle).await?;

// Or use ManagedProcess for ergonomic wait/kill
use claw_runtime::ManagedProcess;
let proc = ManagedProcess::new(handle, Arc::clone(&manager));
let status = proc.wait().await?;

// Associate a process with an agent via the orchestrator
let agent_handle = orchestrator.spawn(agent_config, process_config).await?;
```

---

## Agent Types

### `AgentConfig`

Configuration for registering an agent with the orchestrator:

```rust
pub struct AgentConfig {
    pub agent_id: AgentId,           // Auto-generated if using AgentConfig::new()
    pub name: String,                // Human-readable agent name
    pub mode: ExecutionMode,         // Safe (default) or Power mode
    pub metadata: HashMap<String, String>, // Custom key-value metadata
}

impl AgentConfig {
    /// Create new config with auto-generated ID and Safe mode
    pub fn new(name: impl Into<String>) -> Self;
    
    /// Set execution mode (builder pattern)
    pub fn with_mode(mut self, mode: ExecutionMode) -> Self;
    
    /// Add metadata entry (builder pattern)
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self;
}
```

### `AgentId`

Unique agent identifier:

```rust
pub struct AgentId(pub String);

impl AgentId {
    /// Create from any string-like value
    pub fn new(id: impl Into<String>) -> Self;
    
    /// Generate unique ID using nanosecond-based hex token
    pub fn generate() -> Self;
    
    /// Get underlying string slice
    pub fn as_str(&self) -> &str;
}
```

### `AgentInfo`

Runtime information about a registered agent:

```rust
pub struct AgentInfo {
    pub config: AgentConfig,
    pub started_at: u64,              // Unix timestamp in milliseconds
    pub process_handle: Option<ProcessHandle>, // None for in-process agents
    pub status: AgentStatus,          // Starting, Running, Paused, Stopped, Error
}
```

---
