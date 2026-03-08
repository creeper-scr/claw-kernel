---
title: claw-script
description: Embedded script engines (Lua default, Deno/V8 and Python planned)
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
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

  // Memory 🚧 Not implemented (planned for v0.2)
  memory: {
    get(key: string): Promise<any>;
    set(key: string, value: any): Promise<void>;
    search(query: string, topK: number): Promise<MemoryItem[]>;
  };

  // Events 🚧 Not implemented (planned for v0.2)
  events: {
    emit(event: string, data: any): void;
    on(event: string, handler: (data: any) => void): void;
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

  // Directories 🚧 Not implemented (planned for v0.1.1)
  dirs: {
    configDir(): string;
    dataDir(): string;
    cacheDir(): string;
    toolsDir(): string;
  };

  // Agent 🚧 Not implemented (planned for v0.2)
  agent: {
    spawn(config: AgentConfig): Promise<AgentHandle>;
    kill(handle: AgentHandle): Promise<void>;
    list(): AgentInfo[];
  };
}
```

---

## Lua Example

```lua
-- Example: Tool script using Filesystem, Network and Tools bridges
local M = {}

function M.execute(params)
    -- Check if file exists
    if not rust.fs.exists(params.path) then
        return {
            success = false,
            error = "File not found: " .. params.path
        }
    end

    -- Read file content
    local content = rust.fs.read(params.path)

    -- Fetch additional data from web
    local response = rust.net.get("https://api.example.com/data")
    if response.status == 200 then
        content = content .. "\n" .. response.body
    end

    -- Call a registered tool (via tools bridge)
    local result = rust.tools:call("summarize", {
        text = content
    })

    if result:success() then
        return {
            success = true,
            summary = result:output()
        }
    else
        return {
            success = false,
            error = result:error()
        }
    end
end

return M
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
