---
title: claw-script
description: Embedded script engines (Lua default, Deno/V8, PyO3)
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](claw-script.md)


# claw-script

嵌入式脚本引擎（Lua、Deno/V8、Python），提供统一的 RustBridge API。

## 架构

**进程模型：Per-Agent 脚本沙箱**

```
┌─────────────────────────────────────────────────────────────┐
│  Agent 进程                                                 │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ claw-loop (AgentLoop)                                 │  │
│  │  ┌─────────────────┐  ┌─────────────────────────────┐ │  │
│  │  │ 原生工具        │  │ 脚本工具（通过 IPC）        │ │  │
│  │  │（进程内）       │  │                             │ │  │
│  │  └─────────────────┘  │ ┌─────────────────────────┐ │ │  │
│  │                       │ │ 脚本沙箱进程            │ │ │  │
│  │                       │ │ (claw-script)           │ │ │  │
│  │                       │ │                         │ │ │  │
│  │                       │ │ • Lua/V8/Python 引擎    │ │ │  │
│  │                       │ │ • 脚本执行              │ │ │  │
│  │                       │ │ • RustBridge API        │ │ │  │
│  │                       │ └─────────────────────────┘ │ │  │
│  │                       └─────────────────────────────┘ │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

- 每个 Agent 有自己的**脚本沙箱进程**
- 脚本在隔离进程中运行（崩溃不影响 Agent）
- 通过 IPC（Unix socket / Named Pipe）通信
- 低耦合：内核不依赖脚本实现

---

## 概述

`claw-script` 嵌入脚本语言：
- **Lua**（默认）：快速，零依赖
- **Deno/V8**：完整的 TypeScript/JavaScript
- **Python**：访问 ML 生态系统

---

## 用法

```toml
[dependencies]
# 仅 Lua（默认）
claw-script = "0.1"

# 带 Deno/V8
claw-script = { version = "0.1", features = ["engine-v8"] }

# 带 Python
claw-script = { version = "0.1", features = ["engine-py"] }
```

```rust
use claw_script::{ScriptEngine, LuaEngine};

let engine = LuaEngine::new()?;

// 编译并执行
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

暴露给所有脚本引擎：

```typescript
// 所有引擎中可用
interface RustBridge {
  // LLM
  llm: {
    complete(messages: Message[]): Promise<Response>;
    stream(messages: Message[]): AsyncIterable<Delta>;
  };
  
  // 工具
  tools: {
    register(def: ToolDef): void;
    call(name: string, params: any): Promise<any>;
    list(): ToolMeta[];
  };
  
  // 记忆
  memory: {
    get(key: string): Promise<any>;
    set(key: string, value: any): Promise<void>;
    search(query: string, topK: number): Promise<MemoryItem[]>;
  };
  
  // 事件
  events: {
    emit(event: string, data: any): void;
    on(event: string, handler: (data: any) => void): void;
  };
  
  // 文件系统
  fs: {
    read(path: string): Promise<Uint8Array>;
    write(path: string, data: Uint8Array): Promise<void>;
    exists(path: string): boolean;
    listDir(path: string): DirEntry[];
    glob(pattern: string): string[];
  };
  
  // 网络
  net: {
    get(url: string, headers?: Headers): Promise<Response>;
    post(url: string, headers: Headers, body: string): Promise<Response>;
  };
  
  // JSON
  json: {
    parse(text: string): any;
    stringify(value: any, opts?: { pretty?: boolean }): string;
  };
  
  // 目录
  dirs: {
    configDir(): string;
    dataDir(): string;
    cacheDir(): string;
    toolsDir(): string;
  };
  
  // 智能体
  agent: {
    spawn(config: AgentConfig): Promise<AgentHandle>;
    kill(handle: AgentHandle): Promise<void>;
    list(): AgentInfo[];
  };
}
```

---

## Lua 示例

```lua
local M = {}

function M.execute(params)
    -- 读取文件
    local content = rust.fs.read(params.path)
    
    -- 调用 LLM
    local response = rust.llm.complete({
        { role = "user", content = "Summarize: " .. content }
    })
    
    -- 存储结果
    rust.memory.set("last_summary", response.content)
    
    return {
        success = true,
        result = response.content
    }
end

return M
```

---

## TypeScript 示例 (Deno)

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

## Python 示例

```python
# @name data_analyzer
# @permissions fs.read, memory.write

import json

async def execute(params):
    content = await rust.fs.read(params["path"])
    data = json.loads(content)
    
    # 使用 Python 生态系统
    import numpy as np
    values = np.array([d["value"] for d in data])
    mean = np.mean(values)
    
    await rust.memory.set("analysis", {"mean": mean})
    
    return {"success": True, "result": {"mean": float(mean)}}
```

---

## 特性

```toml
[features]
default = ["engine-lua"]
engine-lua = ["mlua"]          # 零依赖，编译快
engine-v8 = ["deno_core"]      # 完整 JS/TS，~100MB
engine-py = ["pyo3"]           # Python 生态系统
```

---

## 引擎选择

```rust
use claw_script::{ScriptEngine, EngineType};

// 根据文件扩展名自动检测
let engine = ScriptEngine::for_file("tool.lua")?;  // LuaEngine
let engine = ScriptEngine::for_file("tool.ts")?;   // V8Engine（如果启用）
let engine = ScriptEngine::for_file("tool.py")?;   // PythonEngine（如果启用）

// 通过 EngineType 创建
let engine = ScriptEngine::new(EngineType::Lua)?;
#[cfg(feature = "engine-v8")]
let engine = ScriptEngine::new(EngineType::V8)?;
#[cfg(feature = "engine-py")]
let engine = ScriptEngine::new(EngineType::Python)?;
```
