---
title: claw-script
description: Embedded script engines (Lua default, V8 optional)
status: implemented
version: "1.4.1"
last_updated: "2026-03-10"
language: en
---

> **⚠️ Security Warning**
> 
> Script engine sandboxing has **known limitations**:
> - Lua engine relies on runtime permission checks; scripts may potentially crash the host process
> - V8 engine provides stronger isolation via isolates, but is not penetration tested
> - The bridge APIs have not undergone comprehensive security auditing
>
> **For production use with untrusted code**, we recommend:
> - Using Safe Mode OS-level sandbox (Linux/macOS)
> - Running agents in containers or VMs
> - Applying defense-in-depth strategies
>
> **Security contributions welcome!** See [SECURITY.md](../../SECURITY.md) for reporting
> vulnerabilities and [CONTRIBUTING.md](../../CONTRIBUTING.md) for contributing improvements.

Embedded script engines (Lua, V8/TypeScript) with unified RustBridge API.

## Architecture

**Layer 3: Script Runtime**

`claw-script` provides the kernel's script execution layer:

- **Multi-engine support**: Lua (default), V8/TypeScript (optional)
- **Hot-loading**: Load/unload scripts without restart (see [Hot Reload](#hot-reload))
- **RustBridge API**: Expose kernel capabilities to scripts
- **Sandboxed execution**: Enforce security boundaries

> **Note**: This crate provides script execution **capabilities**. The specific process isolation strategy (in-process vs separate process) is an implementation detail that applications can configure.

---

## Overview

`claw-script` embeds scripting languages:
- **Lua** (default, implemented): Fast, zero dependencies, ~500KB
- **V8/TypeScript** (optional, implemented): Full ES2022+ support, ~100MB

---

## Usage

```toml
[dependencies]
# Lua only (default)
claw-script = "0.1"

# With V8/TypeScript support
claw-script = { version = "0.1", features = ["engine-v8"] }
```

### Lua Example

```rust
use claw_script::{ScriptEngine, LuaEngine};
use claw_script::types::{Script, ScriptContext};

let engine = LuaEngine::new();

// Execute a script
let script = Script::lua("example", r#"
    local M = {}
    function M.greet(name)
        return "Hello, " .. name
    end
    return M.greet("World")
"#);

let ctx = ScriptContext::new("agent-1");
let result = engine.execute(&script, &ctx).await?;
```

### V8/TypeScript Example

```rust
use claw_script::{ScriptEngine, V8Engine, V8EngineOptions};
use claw_script::types::{Script, ScriptContext};

// Create V8 engine with default options
let engine = V8Engine::new();

// Or with custom options
let engine = V8Engine::with_options(V8EngineOptions {
    timeout: Duration::from_secs(60),
    heap_limit_mb: 256,
    typescript: true,
    max_recursion_depth: 64,
});

// Execute JavaScript
let script = Script::javascript("example", r#"
    const data = { message: "Hello from V8!", timestamp: Date.now() };
    claw.events.emit("data_ready", data);
    return data.message;
"#);

let ctx = ScriptContext::new("agent-1")
    .with_event_bus(event_bus);

let result = engine.execute(&script, &ctx).await?;

// Or execute TypeScript (transpiled automatically)
let ts_script = Script::typescript("example", r#"
    interface Config {
        name: string;
        value: number;
    }
    const cfg: Config = { name: "test", value: 42 };
    return cfg;
"#);
```

---

## RustBridge

Exposed to all script engines:

```typescript
// Available in all engines
interface RustBridge {
  // Tools ✅ Implemented
  tools: {
    call(name: string, params: any): Promise<ToolResult>;
    list(): ToolInfo[];
    exists(name: string): boolean;
  };

  // Events ✅ Implemented (EventBus pub/sub, lifecycle-bound)
  events: {
    emit(event: string, data: any): void;
    on(event: string, handler: (data: any) => void): void;
    once(event: string, handler: (data: any) => void): void;
    poll(): void;
  };

  // Filesystem ✅ Implemented
  fs: {
    read(path: string): Promise<Uint8Array>;
    write(path: string, data: Uint8Array): Promise<void>;
    exists(path: string): boolean;
    listDir(path: string): DirEntry[];
    glob(pattern: string): string[];
  };

  // Network ✅ Implemented
  net: {
    get(url: string, headers?: Headers): Promise<Response>;
    post(url: string, headers: Headers, body: string): Promise<Response>;
  };

  // JSON (native Lua support, V8 uses native JSON)
  json: {
    parse(text: string): any;
    stringify(value: any, opts?: { pretty?: boolean }): string;
  };

  // Directories ✅ Implemented (platform-aware paths)
  dirs: {
    configDir(): string | null;
    dataDir(): string | null;
    cacheDir(): string | null;
    toolsDir(): string | null;
    scriptsDir(): string | null;
    logsDir(): string | null;
  };

  // Agent ✅ Implemented (in-process lifecycle, auto-cleanup on script end)
  agent: {
    spawn(name: string): AgentId;
    status(id: AgentId): string;  // "running" | "stopped" | "unknown"
    kill(id: AgentId): void;
    list(): AgentId[];
    info(id: AgentId): AgentInfo | null;
  };

  // Memory: NOT exposed to scripts (D1 decision, v1.3.0)
  // Use the `claw-memory` crate's Rust API directly for memory operations.

  // LLM ✅ Implemented (GAP-01, v1.4.0) — only available when ScriptContext
  //         is constructed with an LLMProvider via RustBridge::with_llm()
  llm: {
    complete(messages: Message[], opts?: LlmOpts): string;  // blocking, returns full response
    stream(messages: Message[], opts?: LlmOpts): string[];  // returns array of text chunks
  };
}
```

> **Note:** `llm` 仅在 Lua 引擎中可用（`bridge/llm.rs`）；V8 引擎目前暂未露出 LLM bridge。

---

## Bridge 模块结构（v1.4.0+）

### bridge/mod.rs — RustBridge 聚合结构

```rust
pub struct RustBridge {
    pub tools: Option<ToolsBridge>,
    pub events: Option<EventsBridge>,
    pub fs: Option<FsBridge>,
    pub agent: Option<AgentBridge>,
    pub dirs: Option<DirsBridge>,
    pub llm: Option<LlmBridge>,   // v1.4.0 新增，GAP-01
    // Note: MemoryBridge 已移除 (D1, v1.3.0)
}
```

### bridge/llm.rs — LLM Bridge (GAP-01, v1.4.0)

`LlmBridge` 将 `LLMProvider` 暴露给 Lua 脚本。在 `spawn_blocking` 环境中遵循 `add_method + block_on` 模式（与 NetBridge Fix-F 一致）。

```lua
-- Non-streaming completion
local reply = llm:complete(
    {{ role = "user", content = "What is Rust?" }},
    { model = "claude-opus-4-6", max_tokens = 1024 }
)
print(reply)

-- Streaming: returns array of text chunks
local chunks = llm:stream(
    {{ role = "user", content = "Tell me a joke" }},
    { model = "claude-opus-4-6" }
)
for _, chunk in ipairs(chunks) do
    io.write(chunk)
end
```

支持的 `opts` 字段：`model` (string)、`max_tokens` (integer)、`temperature` (number)。
支持的 `role` 字符串：`"user"` / `"assistant"` / `"system"` / `"tool"`。

### bridge/conversion.rs — 类型转换层 (Fix-G, v1.1.0)

`conversion.rs` 从 `tools.rs` 中提取，封装 Lua value ↔ Rust/JSON 类型的公用转换函数。内部模块（`pub(crate)`），供各 bridge 共享使用。

---

## Lua Example

```lua
-- Example: Using all available bridges

-- Dirs bridge: platform-aware paths (always available)
local cfg = dirs:config_dir()
local data = dirs:data_dir()

-- Events bridge: EventBus pub/sub (requires ScriptContext with event_bus)
events:on("task_done", function(data)
    -- handle event
end)
events:emit("task_started", { name = "my_task" })
events:poll()  -- process pending events and invoke callbacks

-- Agent bridge: spawn child agents (requires ScriptContext with orchestrator)
local child_id = agent:spawn("worker")
local status = agent:status(child_id)  -- "running"
agent:kill(child_id)

-- Tools bridge (always available if ToolRegistry provided)
local result = tools:call("summarize", { text = "..." })

-- Filesystem bridge
local content = fs:read("/path/to/file.txt")
fs:write("/path/to/out.txt", content)

-- Network bridge
local resp = net:get("https://api.example.com/data")

-- LLM bridge (v1.4.0+, requires RustBridge::with_llm())
local reply = llm:complete(
    {{ role = "user", content = "Summarize this" }},
    { model = "claude-opus-4-6", max_tokens = 512 }
)
print(reply)

-- Note: Memory operations (memory:set/get/search) are NOT available in scripts.
-- Use the `claw-memory` crate's Rust API directly (D1 decision, v1.3.0).
```

---

## TypeScript/JavaScript Example

```typescript
// Example: Using all available bridges in JavaScript/TypeScript

// Dirs bridge: platform-aware paths
const cfg = claw.dirs.configDir();
const data = claw.dirs.dataDir();

// Events bridge
claw.events.on("task_done", (data) => {
    console.log("Task done:", data);
});
claw.events.emit("task_started", { name: "my_task" });

// Agent bridge
const childId = claw.agent.spawn("worker");
const status = claw.agent.status(childId);
claw.agent.kill(childId);

// Tools bridge
const result = await claw.tools.call("summarize", { text: "..." });

// Filesystem bridge
const content = await claw.fs.read("/path/to/file.txt");
await claw.fs.write("/path/to/out.txt", content);

// Network bridge
const resp = await claw.net.get("https://api.example.com/data");
const body = await resp.text();

// JSON utilities
const obj = claw.json.parse('{"key": "value"}');
const str = claw.json.stringify(obj, { pretty: true });

// Note: claw.memory.* is NOT available (D1 decision, v1.3.0).
// Use the `claw-memory` crate's Rust API directly for memory operations.
```

---

## Engine Comparison

| Feature | Lua | V8/TypeScript |
|---------|-----|---------------|
| Binary size | ~500KB | ~100MB |
| Startup time | <1ms | ~10-50ms |
| Memory per execution | ~1MB | ~10-50MB |
| Language features | Minimal | Full ES2022+ |
| TypeScript | No | Yes |
| Sandboxing | Limited | Strong (isolate) |
| Async/await | No | Yes |
| Ecosystem | Small | Massive (npm) |
| Use case | Simple tools | Complex agents |

---

## Features

```toml
[features]
default = ["engine-lua"]
engine-lua = ["mlua"]           # Zero deps, fast compile ✅
engine-v8 = ["deno_core"]       # Full JS/TS, ~100MB ✅
```

---

## Engine Selection

```rust
use claw_script::{ScriptEngine, LuaEngine, V8Engine};

// Lua engine (default, lightweight)
let lua_engine = LuaEngine::new();

// V8 engine (feature = "engine-v8")
let v8_engine = V8Engine::new();

// V8 with custom options
let v8_engine = V8Engine::with_options(V8EngineOptions {
    timeout: Duration::from_secs(60),
    heap_limit_mb: 256,
    typescript: true,
    max_recursion_depth: 64,
});

// Create script for specific engine
let lua_script = Script::lua("tool", "return 42");
let js_script = Script::javascript("tool", "return 42");
let ts_script = Script::typescript("tool", "return 42 as number");
```

---

## V8EngineOptions

Configuration options for the V8 engine:

```rust
pub struct V8EngineOptions {
    /// Script execution timeout (default: 30s)
    pub timeout: Duration,
    /// V8 heap limit in MB (default: 128MB)
    pub heap_limit_mb: usize,
    /// Enable TypeScript support (default: true)
    pub typescript: bool,
    /// Maximum recursion depth for JSON conversion (default: 32)
    pub max_recursion_depth: u32,
}
```

---

## Security Notes

### Lua Engine
- Sandboxing relies on runtime permission checks
- Scripts can potentially crash the host process
- Use Safe Mode OS sandbox for untrusted code

### V8 Engine
- Each execution runs in a fresh V8 isolate
- Memory limits enforced by V8 heap constraints
- Stronger sandboxing than Lua
- Still recommend Safe Mode for untrusted code

---

## Hot Reload

Layer 3 provides a **hot-reload mechanism** for scripts, enabling runtime updates without restart.

### Hot Reload Architecture

```
File System ──► ScriptWatcher ──► ScriptEvent ──► HotReloadManager ──► ScriptRegistry
                     │                                │
                     └─ debounce (50ms)               ├─ compile & validate
                                                      ├─ hot-swap (atomic)
                                                      └─ event notify
```

### Key Components

| Component | Purpose |
|-----------|---------|
| `ScriptWatcher` | File system watcher with debouncing |
| `HotReloadManager` | Coordinates loading, validation, and event emission |
| `ScriptRegistry` | Thread-safe registry with version history |
| `ScriptModule` | Versioned script with atomic swap support |
| `ScriptEventBus` | Event publishing for hot-reload events |

### Usage Example

```rust
use std::sync::Arc;
use claw_script::{LuaEngine, HotReloadManager, HotReloadConfig};

// Create engine and hot-reload manager
let engine = Arc::new(LuaEngine::new());
let config = HotReloadConfig::new()
    .watch_dir("./scripts")
    .extension("lua")
    .extension("js")
    .auto_reload(true);

let mut manager = HotReloadManager::new(config, engine)?;

// Subscribe to events
let mut events = manager.subscribe();
tokio::spawn(async move {
    while let Ok(event) = events.recv().await {
        match event {
            ScriptEvent::Loaded { entry, path } => {
                println!("Loaded: {} from {:?}", entry.name, path);
            }
            ScriptEvent::Reloaded { entry, previous_version, new_version, .. } => {
                println!("Reloaded: {} v{} -> v{}", 
                    entry.name, previous_version, new_version);
            }
            ScriptEvent::Failed { path, error, .. } => {
                eprintln!("Failed to load {:?}: {}", path, error);
            }
            _ => {}
        }
    }
});

// Start watching (blocks until stopped)
manager.start().await?;
```

### Manual Script Loading

```rust
// Load a script manually (not via file watcher)
let module = manager.load_file("./my_script.lua").await?;
println!("Loaded: {} (v{})", module.current().name, module.version());

// Execute the loaded script
let ctx = ScriptContext::new("agent-1");
let result = manager.execute("my_script", &ctx).await?;
```

### Configuration Options

```rust
pub struct HotReloadConfig {
    /// Directories to watch
    pub watch_dirs: Vec<PathBuf>,
    /// File extensions to watch
    pub extensions: HashSet<String>,
    /// Debounce delay for file events
    pub debounce_delay: Duration,
    /// Maximum versions to keep in history
    pub max_history_size: usize,
    /// Enable auto-reload on file change
    pub auto_reload: bool,
    /// Validate scripts before reloading
    pub validate_before_reload: bool,
    /// Engine type filter (None = all)
    pub engine_filter: Option<EngineType>,
    /// Watch subdirectories recursively
    pub recursive: bool,
}
```

### Version Management & Rollback

Scripts maintain version history for rollback:

```rust
// Get current script info
let entry = manager.get_script("my_tool").unwrap();
println!("Current version: {}", entry.version);

// Rollback to previous version
if manager.rollback("my_tool") {
    println!("Rolled back successfully");
}

// Access version history via ScriptModule
let module = manager.registry()
    .get_or_create("my_tool")
    .unwrap();
    
for (version, loaded_at) in module.history_versions() {
    println!("v{} at {:?}", version, loaded_at);
}
```

### Events

The hot-reload system emits the following events:

| Event | Trigger |
|-------|---------|
| `ScriptEvent::Loaded` | New script detected |
| `ScriptEvent::Reloaded` | Script file modified |
| `ScriptEvent::Unloaded` | Script file deleted |
| `ScriptEvent::Failed` | Validation/compilation error |
| `ScriptEvent::Debounced` | Batch of rapid changes |
| `ScriptEvent::Started` | Watcher started |
| `ScriptEvent::Stopped` | Watcher stopped |
| `ScriptEvent::CacheUpdated` | Script cache invalidated |

### Event Filtering

```rust
use claw_script::hot_reload::EventFilter;

// Subscribe only to successful loads/reloads
let mut success_events = EventFilter::success_only(&manager.event_bus());

// Subscribe only to errors
let mut error_events = EventFilter::errors_only(&manager.event_bus());

// Subscribe to specific script
let mut specific = EventFilter::for_script(&manager.event_bus(), "my_tool");
```

### Cancellation Support

```rust
use tokio::sync::watch;

let (tx, rx) = watch::channel(false);

// Run with cancellation token
tokio::spawn(async move {
    manager.run_with_cancel(rx).await.unwrap();
});

// Stop the manager
tx.send(true).unwrap();
```

---

## References

- [ADR-002: Multi-Engine Script Support](../adr/002-script-engine-selection.md)
- [ADR-004: Tool Hot-Loading](../adr/004-hot-loading-mechanism.md)
- [ADR-012: V8 Engine Implementation](../adr/012-v8-engine-implementation.md)
- [deno_core documentation](https://docs.rs/deno_core)
- [V8 Engine documentation](https://v8.dev/docs)

---
