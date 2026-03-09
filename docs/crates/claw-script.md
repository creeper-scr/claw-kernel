---
title: claw-script
description: Embedded script engines (Lua default, Deno/V8 and Python planned)
status: implemented
version: "0.1.0"
last_updated: "2026-03-08"
language: en
---



Embedded script engines (Lua, Deno/V8, Python) with unified RustBridge API.

## Architecture

**Layer 3: Script Runtime**

`claw-script` provides the kernel's script execution layer:

- **Multi-engine support**: Lua (default), Deno/V8, Python
- **Hot-loading**: Load/unload scripts without restart
- **RustBridge API**: Expose kernel capabilities to scripts
- **Sandboxed execution**: Enforce security boundaries

> **Note**: This crate provides script execution **capabilities**. The specific process isolation strategy (in-process vs separate process) is an implementation detail that applications can configure.

---

## Overview

`claw-script` embeds scripting languages:
- **Lua** (default, implemented): Fast, zero dependencies
- **Deno/V8** (planned): Full TypeScript/JavaScript support
- **Python** (planned): ML ecosystem access

---

## Usage

```toml
[dependencies]
# Lua only (default)
claw-script = "0.1"

# With Deno/V8 (planned)
# claw-script = { version = "0.1", features = ["engine-v8"] }

# With Python (planned)
# claw-script = { version = "0.1", features = ["engine-py"] }
```

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

  // Memory ✅ Implemented (namespace-isolated, key-value + semantic search)
  memory: {
    get(key: string): Promise<any>;
    set(key: string, value: any): Promise<void>;
    delete(key: string): Promise<void>;
    search(query: string, topK: number): Promise<MemoryItem[]>;
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

  // JSON (native Lua support)
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
}
```

---

## Lua Example

```lua
-- Example: Using all available bridges

-- Dirs bridge: platform-aware paths (always available)
local cfg = dirs:config_dir()
local data = dirs:data_dir()

-- Memory bridge: namespace-isolated key-value store (requires ScriptContext with memory_store)
memory:set("user_pref", "dark_mode")
local pref = memory:get("user_pref")   -- "dark_mode"
local items = memory:search("dark", 5) -- semantic search, returns array of strings

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
```

---

## TypeScript Example (Deno) 🚧 Planned

> **Note:** Deno/V8 engine support is planned but not yet implemented.

```typescript
// @name summarizer
// @permissions fs.read, tools.call

interface Params {
    path: string;
}

export async function execute(params: Params) {
    const content = await rust.fs.read(params.path);
    const text = new TextDecoder().decode(content);

    const result = await rust.tools.call("summarize", { text });

    return { success: result.success, result: result.output };
}
```

---

## Python Example 🚧 Planned

> **Note:** Python engine support is planned but not yet implemented.

```python
# @name data_analyzer
# @permissions fs.read, tools.call

import json

async def execute(params):
    content = await rust.fs.read(params["path"])
    data = json.loads(content)

    # Use Python ecosystem
    import numpy as np
    values = np.array([d["value"] for d in data])
    mean = np.mean(values)

    return {"success": True, "result": {"mean": float(mean)}}
```

---

## Features

```toml
[features]
default = ["engine-lua"]
engine-lua = ["mlua"]          # Zero deps, fast compile ✅
# engine-v8 = ["deno_core"]    # Full JS/TS, ~100MB 🚧 Planned
# engine-py = ["pyo3"]         # Python ecosystem 🚧 Planned
```

---

## Engine Selection

Currently only Lua engine is available:

```rust
use claw_script::{ScriptEngine, LuaEngine};

// Create Lua engine
let engine = LuaEngine::new();

// Planned: Auto-detect from file extension
// let engine = ScriptEngine::for_file("tool.lua")?;  // LuaEngine
// let engine = ScriptEngine::for_file("tool.ts")?;   // V8Engine (planned)
// let engine = ScriptEngine::for_file("tool.py")?;   // PythonEngine (planned)
```

---
