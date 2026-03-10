---
title: claw-runtime
description: Event bus, async runtime, multi-agent orchestration
status: implemented
version: "1.4.1"
last_updated: "2026-03-10"
language: en
---



System runtime with event bus, process management, and multi-agent orchestration.

---

## Overview

`claw-runtime` provides Layer 1 (System Runtime) infrastructure:
- **EventBus**: Broadcast channel for inter-component communication (capacity 1024)
- **AgentOrchestrator**: Multi-agent lifecycle management
- **IpcRouter**: A2A (Agent-to-Agent) message routing
- **Scheduler**: Time-triggered task scheduling
- **Runtime**: Unified composition root

### Architecture Position

```
Layer 2 (claw-provider, claw-tools, etc.)
    ↓
Layer 1: claw-runtime ←── You are here
    ↓
Layer 0.5: claw-pal
```

**Dependency Rule:** `claw-runtime` ONLY depends on `claw-pal`. It does NOT depend on any Layer 2 crates.

---

## Usage

```toml
[dependencies]
claw-runtime = "1.0"
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

## Runtime Structure

The `Runtime` struct is the top-level composition root:

```rust
pub struct Runtime {
    pub event_bus: Arc<EventBus>,
    pub orchestrator: Arc<AgentOrchestrator>,
    pub ipc_router: Arc<IpcRouter>,
    /// Shared process manager — use via `orchestrator.spawn()` or
    /// directly for one-off process operations.
    pub process_manager: Arc<dyn ProcessManager>,
}

impl Runtime {
    /// Construct a new `Runtime` with the given IPC endpoint.
    ///
    /// Creates a fresh `EventBus`, `TokioProcessManager` (as the default
    /// process manager implementation), wires the `AgentOrchestrator` and 
    /// `IpcRouter` to them, then wraps everything in `Arc`s.
    /// Construct a new `Runtime` and auto-start the orchestrator and IPC listener (v1.1.0+).
    pub async fn new(ipc_endpoint: impl Into<String>) -> Result<Self, RuntimeError>;

    /// Deprecated since v1.1.0 — Runtime::new() now auto-starts; this is a no-op.
    #[deprecated(since = "1.1.0")]
    pub async fn start(&self) -> Result<(), RuntimeError>;

    /// Broadcast a `Shutdown` event to all subscribers.
    pub fn shutdown(&self) -> Result<(), RuntimeError>;
}
```

---

## Event Bus

The `EventBus` is a broadcast channel with capacity 1024, providing decoupled communication between components.

```rust
use claw_runtime::{EventBus, Event, EventFilter, LagStrategy};

let bus = EventBus::new();

// Subscribe to all events
let mut rx = bus.subscribe();

// Subscribe with a declarative filter
let mut tool_rx = bus.subscribe_with_filter(EventFilter::ToolCalls);

// Subscribe with a custom closure predicate
let mut agent_rx = bus.subscribe_filtered(|e| matches!(e, Event::AgentStarted { .. }));

// Publish an event
bus.publish(Event::ToolCalled {
    agent_id: agent_id.clone(),
    tool_name: "calculator".to_string(),
    call_id: "call-001".to_string(),
})?;
```

### Lag Strategy

Control behavior when receiver falls behind:

```rust
// Return error when lagged (default)
let bus = EventBus::with_lag_strategy(LagStrategy::Error);

// Skip lagged messages and continue
let bus = EventBus::with_lag_strategy(LagStrategy::Skip);

// Log warning and continue
let bus = EventBus::with_lag_strategy(LagStrategy::Warn);
```

### Event Filters

| Filter | Matches |
|--------|---------|
| `All` | Every event |
| `AgentLifecycle` | `AgentStarted`, `AgentStopped` |
| `ToolCalls` | `ToolCalled`, `ToolResult` |
| `LlmRequests` | `LlmRequestStarted`, `LlmRequestCompleted` |
| `MemoryEvents` | `ContextWindowApproachingLimit`, `MemoryArchiveComplete` |
| `A2A` | A2A messaging events |
| `ShutdownOnly` | Only `Shutdown` event |
| `Custom(fn)` | User-defined predicate |

---

## Multi-Agent Orchestration

`AgentOrchestrator` manages agent lifecycle with health checking, resource quotas, and auto-restart capabilities.

```rust
use claw_runtime::{AgentOrchestrator, AgentConfig, AgentId, EventBus};
use claw_pal::{ProcessConfig, TokioProcessManager, ExecutionMode};
use std::sync::Arc;
use std::time::Duration;

let bus = Arc::new(EventBus::new());
let pm = Arc::new(TokioProcessManager::new());
let orchestrator = AgentOrchestrator::new(Arc::clone(&bus), pm);

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

### Health Checking

> **Note — Test vs. Production Initialization**
>
> Background tasks (`health_check_task`, `auto_restart_task`) are started automatically
> in production builds. In `#[cfg(test)]` environments they are **not** started automatically;
> call `orchestrator.init_background_tasks()` explicitly in integration tests that require them.

```rust
// Check health of a specific agent
let health = orchestrator.health_check(&agent_id).await?;
println!("Healthy: {}, Memory: {}KB", health.is_healthy, health.memory_usage_kb.unwrap_or(0));

// Check all agents
let all_health = orchestrator.health_check_all().await;

// Record heartbeat
orchestrator.record_heartbeat(&agent_id).await?;
```

### Resource Quotas

```rust
use claw_runtime::orchestrator::ResourceQuota;

let quota = ResourceQuota::new()
    .with_memory(512)      // 512 MB
    .with_cpu(80.0);       // 80% CPU

orchestrator.set_quota(&agent_id, quota).await?;

// Check if quota exceeded
let violations = orchestrator.check_quota(&agent_id).await?;
```

### Auto-Restart

```rust
use claw_runtime::orchestrator::{RestartPolicy, ResourceQuota};

let policy = RestartPolicy::enabled()
    .with_max_restarts(5)
    .with_backoff(Duration::from_secs(2));

orchestrator.enable_auto_restart(policy).await;

// Disable auto-restart
orchestrator.disable_auto_restart().await;
```

### Steer (Control) Commands

`SteerCommand::Custom` is used by the IPC handler (`claw-server`) to relay steering commands from external clients (v1.3.0+):

```rust
use claw_runtime::orchestrator::SteerCommand;

// Pause agent
orchestrator.steer(&agent_id, SteerCommand::Pause).await?;

// Resume agent
orchestrator.steer(&agent_id, SteerCommand::Resume).await?;

// Update config
orchestrator.steer(&agent_id, SteerCommand::UpdateConfig(Box::new(new_config))).await?;

// Trigger heartbeat
orchestrator.steer(&agent_id, SteerCommand::TriggerHeartbeat).await?;

// Custom command from external IPC client (v1.3.0+)
orchestrator.steer(&agent_id, SteerCommand::Custom {
    command: "set_priority".to_string(),
    payload: Some("high".to_string()),
}).await?;
```

---

## IPC Token Authentication (v1.3.0+)

每个连接到 kernel daemon 的客户端必须通过 `kernel.auth` 握手帧，否则后续请求会被拒绝。

### 认证流程

1. Daemon 启动时生成一次性 token（`DefaultHasher + SystemTime + PID`）
2. Token 写入 `~/.local/share/claw-kernel/kernel.token`（权限 `0o600`）
3. 客户端建立连接后，**第一帧必须**是 `kernel.auth` 类型，携带该 token
4. 认证通过后 `authenticated = true`，否则连接立即关闭

```jsonc
// 客户端握手帧示例 (4-byte BE length prefix + JSON payload)
{
  "type": "kernel.auth",
  "token": "<contents of ~/.local/share/claw-kernel/kernel.token>"
}
```

> **Note:** Token 是每次 daemon 启动时重新生成的，客户端每次都需要重新读取文件。

---

## G-10 Fix: IpcAgentHandle + RestartPolicy (v1.4.1)

v1.4.1 修复了 agent health_check 与 RestartPolicy 的完整实现（原 G-10 gap）。

### SharedSender 热替换机制

`IpcAgentHandle` 携带一个 `SharedSender`（`Arc<Mutex<Option<Sender<AgentMessage>>>>`），而非直接持有 mpsc sender。当 orchestrator 重启一个失败的 agent 时，只需原地替换 `SharedSender` 的内部值，所有已分发出去的 `IpcAgentHandle` 克隆体无需重新获取 handle 即可自动路由到新的消息循环。

```rust
use claw_runtime::{IpcAgentHandle, AgentResponse};
use std::time::Duration;

// Fire-and-forget (returns Err(AgentNotFound) during restart window)
handle.send("task data").await?;

// Wait for response with timeout
let response: AgentResponse = handle
    .send_await("query", Duration::from_secs(30))
    .await?;
println!("Agent replied: {}", response.content);
```

`IpcAgentHandle.shared_tx` 槽为 `None` 时表示 agent 正处于两次重启之间（restart backoff 期间），此时 `send` / `send_await` 返回 `Err(AgentNotFound)`。

### 后台任务函数（内部实现，v1.4.1）

| 函数 | 说明 |
|------|------|
| `spawn_ipc_message_loop()` | 为 IPC agent 启动 mpsc 消息循环；task 退出时自动将 agent 状态置为 Error 并触发 restart |
| `trigger_restart()` | 检查 `RestartState`，等待 backoff，原地热替换 SharedSender，重新 spawn 消息循环 |
| `start_health_check_task()` | 周期性扫描所有 agent；heartbeat 超时检测**仅在** `process_handle.is_some()` 时生效（避免对纯 in-process agent 的误判） |
| `start_auto_restart_task()` | 全局 auto-restart 扫描（仅针对**没有**per-agent `RestartState` 的 agent）；设置 `AgentStatus::Starting` 作为重启锁，防止重复重启 |

### AgentState 新字段

```rust
pub(crate) struct AgentState {
    // ...existing fields...
    /// Shared sender slot for IPC agents.
    /// None for registered or out-of-process agents.
    /// Hot-swapped by trigger_restart() on each restart.
    pub(crate) ipc_tx: Option<SharedSender>,
}
```

### OrchestratorConfig

```rust
pub struct OrchestratorConfig {
    /// Heartbeat timeout (ms); agent marked unhealthy if exceeded. Default: 30_000.
    pub heartbeat_timeout_ms: u64,
    /// Health-check scan interval (seconds). Default: 10.
    pub health_check_interval_secs: u64,
}
```

---

## Process Management

`claw-runtime` provides two ways to manage processes:

### Direct Process Manager Usage

```rust
use claw_runtime::Runtime;
use claw_pal::ProcessConfig;
use claw_pal::traits::ProcessManager as _;
use std::sync::Arc;
use std::collections::HashMap;

let runtime = Runtime::new("/tmp/claw.sock");

// Create process config with builder methods
let handle = runtime.process_manager.spawn(
    ProcessConfig::new("worker".to_string())
        .with_arg("--task".to_string())
        .with_arg("1".to_string())
        .with_env("KEY".to_string(), "value".to_string())
).await?;

// Wait for process to complete
let status = runtime.process_manager.wait(handle).await?;
```

### ManagedProcess Wrapper

```rust
use claw_runtime::ManagedProcess;

// Ergonomic wrapper that pairs handle with manager
let proc = ManagedProcess::new(handle, Arc::clone(&runtime.process_manager));
let status = proc.wait().await?;
proc.kill().await?;
```

---

## IPC Router (A2A Protocol)

The `IpcRouter` enables Agent-to-Agent communication:

```rust
use claw_runtime::IpcRouter;
use claw_runtime::a2a::{A2AMessage, A2AMessageType, A2AMessagePayload};

let router = IpcRouter::with_default_transport(
    Arc::clone(&event_bus),
    "/tmp/claw-a2a.sock"
);

// Register a local agent
let agent_handle = router.register_agent(agent_id.clone(), 100).await;

// Register a remote agent endpoint
router.register_remote_endpoint(
    remote_agent_id,
    "tcp://192.168.1.1:8080"
).await;

// Route a message
let msg = A2AMessage::new(
    "msg-001",
    source_id,
    A2AMessageType::Request,
    A2AMessagePayload::Request {
        action: "compute".to_string(),
        extra: serde_json::json!({"task": "analysis"}),
    },
).with_target(target_id);

router.route_message(msg).await?;

// Start accepting incoming connections
router.start_accepting().await?;
```

### Transport Factory Pattern

The router uses a factory pattern for IPC transport, enabling testability:

```rust
use claw_runtime::ipc_router::{IpcTransportFactory, IpcConnection};

struct MyCustomTransport;

#[async_trait]
impl IpcTransportFactory for MyCustomTransport {
    async fn create_client(&self, endpoint: &str) -> Result<Box<dyn IpcConnection>, RuntimeError> {
        // Custom client implementation
    }
    
    async fn create_server(&self, endpoint: &str) -> Result<Box<dyn IpcConnection>, RuntimeError> {
        // Custom server implementation
    }
}

let router = IpcRouter::new(
    event_bus,
    endpoint,
    Arc::new(MyCustomTransport)
);
```

---

## Task Scheduling

Time-triggered task scheduling with cron-like expressions:

```rust
use claw_runtime::schedule::{Scheduler, TaskConfig, TaskTrigger, TokioScheduler};
use std::time::Duration;

let scheduler = TokioScheduler::new();

// Schedule interval-based task
scheduler.schedule(TaskConfig::new(
    "heartbeat",
    TaskTrigger::interval(Duration::from_secs(30)),
    || async {
        println!("Heartbeat!");
    }
)).await?;

// Schedule cron task
scheduler.schedule(TaskConfig::new(
    "daily-report",
    TaskTrigger::cron("0 9 * * *"),  // 9:00 AM daily
    || async {
        println!("Generating report...");
    }
)).await?;

// Cancel a task
scheduler.cancel(&TaskId::new("heartbeat")).await?;

// List all tasks
let tasks = scheduler.list_tasks().await;

// Shutdown gracefully
scheduler.shutdown().await?;
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

Unique agent identifier using newtype pattern for type safety:

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

## Design Patterns

### Dependency Injection

All external dependencies are abstracted via traits:

```rust
// Runtime constructor accepts ProcessManager trait
impl Runtime {
    pub fn new_with_process_manager(
        ipc_endpoint: impl Into<String>,
        process_manager: Arc<dyn ProcessManager>,
    ) -> Self {
        // ...
    }
}

// This enables testing with mock implementations
#[cfg(test)]
struct MockProcessManager;

#[async_trait]
impl ProcessManager for MockProcessManager {
    // Mock implementations for testing
}
```

### Factory Pattern

`IpcTransportFactory` enables pluggable transport implementations:

```rust
#[async_trait]
pub trait IpcTransportFactory: Send + Sync {
    async fn create_client(&self, endpoint: &str) -> Result<Box<dyn IpcConnection>, RuntimeError>;
    async fn create_server(&self, endpoint: &str) -> Result<Box<dyn IpcConnection>, RuntimeError>;
}
```

### Strategy Pattern

`LagStrategy` provides configurable event handling behavior:

```rust
pub enum LagStrategy {
    Error,  // Return error when lagged
    Skip,   // Skip lagged messages
    Warn,   // Log warning and continue
}
```

### Builder Pattern

Configuration types use builder pattern for ergonomic construction:

```rust
let config = AgentConfig::new("worker")
    .with_mode(ExecutionMode::Safe)
    .with_meta("region", "us-east")
    .with_meta("priority", "high");
```

---

## Error Types

```rust
pub enum RuntimeError {
    /// Agent not found in registry
    AgentNotFound(String),
    
    /// Agent with same ID already exists
    AgentAlreadyExists(String),
    
    /// Process operation failed
    ProcessError(String),
    
    /// IPC communication error
    IpcError(String),
    
    /// Event bus operation failed
    EventBusError(String),
    
    /// Task scheduling error
    ScheduleError(String),
}
```

---

## Testing

### Unit Testing with Mocks

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use claw_pal::TokioProcessManager;

    fn make_orchestrator() -> AgentOrchestrator {
        let bus = Arc::new(EventBus::new());
        let pm = Arc::new(TokioProcessManager::new());
        AgentOrchestrator::new(bus, pm)
    }

    #[test]
    fn test_register_agent() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("test-agent");
        let handle = orc.register(config).unwrap();
        assert_eq!(orc.agent_count(), 1);
    }
}
```

### Event Testing

```rust
#[tokio::test]
async fn test_events_published() {
    let bus = Arc::new(EventBus::new());
    let pm = Arc::new(TokioProcessManager::new());
    let orc = AgentOrchestrator::new(Arc::clone(&bus), pm);
    let mut rx = bus.subscribe();

    let config = AgentConfig::new("event-agent");
    let id = config.agent_id.clone();
    orc.register(config).unwrap();
    orc.unregister(&id, "done").unwrap();

    let e1 = rx.recv().await.unwrap();
    assert!(matches!(e1, Event::AgentStarted { .. }));

    let e2 = rx.recv().await.unwrap();
    assert!(matches!(e2, Event::AgentStopped { .. }));
}
```

---

## See Also

- [Architecture Overview](../architecture/overview.md)
- [Crate Map](../architecture/crate-map.md)
- [PAL Documentation](../architecture/pal.md)
- [claw-pal crate](claw-pal.md) - Layer 0.5 dependencies

---
