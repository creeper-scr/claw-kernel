---
title: claw-script
description: Embedded script engines (Lua default, Deno/V8, PyO3)
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](claw-script.zh.md)


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
- **Lua** (default): Fast, zero dependencies
- **Deno/V8**: Full TypeScript/JavaScript
- **Python**: ML ecosystem access

---

## Usage

```toml
[dependencies]
# Lua only (default)
claw-script = "0.1"

# With Deno/V8
claw-script = { version = "0.1", features = ["engine-v8"] }

# With Python
claw-script = { version = "0.1", features = ["engine-py"] }
```

```rust
use claw_script::{ScriptEngine, LuaEngine};

let engine = LuaEngine::new()?;

// Compile and execute
let script = engine.compile(r#"
    local M = {}
    function M.greet(name)
        return "Hello, " .. name
    end
    return M
"#)?;

let result: String = engine.call(&script, "greet", ("World",))?;
```

---

## RustBridge

Exposed to all script engines:

```typescript
// Available in all engines
interface RustBridge {
  // LLM
  llm: {
    complete(messages: Message[]): Promise<Response>;
    stream(messages: Message[]): AsyncIterable<Delta>;
  };
  
  // Tools
  tools: {
    register(def: ToolDef): void;
    call(name: string, params: any): Promise<any>;
    list(): ToolMeta[];
  };
  
  // Memory (Key-value storage with search)
  memory: {
    get(key: string): Promise<any>;
    set(key: string, value: any): Promise<void>;
    search(query: string, topK: number): Promise<MemoryItem[]>;
  };
  
  // Events
  events: {
    emit(event: string, data: any): void;
    on(event: string, handler: (data: any) => void): void;
  };
  
  // Filesystem
  fs: {
    read(path: string): Promise<Uint8Array>;
    write(path: string, data: Uint8Array): Promise<void>;
    exists(path: string): boolean;
    listDir(path: string): DirEntry[];
    glob(pattern: string): string[];
  };
  
  // Network
  net: {
    get(url: string, headers?: Headers): Promise<Response>;
    post(url: string, headers: Headers, body: string): Promise<Response>;
  };
  
  // JSON
  json: {
    parse(text: string): any;
    stringify(value: any, opts?: { pretty?: boolean }): string;
  };
  
  // Directories
  dirs: {
    configDir(): string;
    dataDir(): string;
    cacheDir(): string;
    toolsDir(): string;
  };
  
  // Agent
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
local M = {}

function M.execute(params)
    -- Read file
    local content = rust.fs.read(params.path)
    
    -- Call LLM
    local response = rust.llm.complete({
        { role = "user", content = "Summarize: " .. content }
    })
    
    -- Store result
    rust.memory.set("last_summary", response.content)
    
    return {
        success = true,
        result = response.content
    }
end

return M
```

---

## TypeScript Example (Deno)

```typescript
// @name summarizer
// @permissions fs.read, memory.write

interface Params {
    path: string;
}

export async function execute(params: Params) {
    const content = await rust.fs.read(params.path);
    const text = new TextDecoder().decode(content);
    
    const response = await rust.llm.complete([
        { role: "user", content: `Summarize: ${text}` }
    ]);
    
    await rust.memory.set("last_summary", response.content);
    
    return { success: true, result: response.content };
}
```

---

## Python Example

```python
# @name data_analyzer
# @permissions fs.read, memory.write

import json

async def execute(params):
    content = await rust.fs.read(params["path"])
    data = json.loads(content)
    
    # Use Python ecosystem
    import numpy as np
    values = np.array([d["value"] for d in data])
    mean = np.mean(values)
    
    await rust.memory.set("analysis", {"mean": mean})
    
    return {"success": True, "result": {"mean": float(mean)}}
```

---

## Features

```toml
[features]
default = ["engine-lua"]
engine-lua = ["mlua"]          # Zero deps, fast compile
engine-v8 = ["deno_core"]      # Full JS/TS, ~100MB
engine-py = ["pyo3"]           # Python ecosystem
```

---

## Engine Selection

```rust
use claw_script::{ScriptEngine, EngineType};

// Auto-detect from file extension
let engine = ScriptEngine::for_file("tool.lua")?;  // LuaEngine
let engine = ScriptEngine::for_file("tool.ts")?;   // V8Engine (if enabled)
let engine = ScriptEngine::for_file("tool.py")?;   // PythonEngine (if enabled)

// Create from EngineType
let engine = ScriptEngine::new(EngineType::Lua)?;
#[cfg(feature = "engine-v8")]
let engine = ScriptEngine::new(EngineType::V8)?;
#[cfg(feature = "engine-py")]
let engine = ScriptEngine::new(EngineType::Python)?;
```

---
