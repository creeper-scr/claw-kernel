---
title: claw-tools
description: Tool registry, hot-loading, schema generation
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](claw-tools.md)


# claw-tools

用于智能体能力的工具注册表和热加载。

---

## 概述

`claw-tools` 实现工具使用协议：
- 工具注册和发现
- 模式生成和验证
- 从脚本热加载
- 权限管理

---

## 用法

```toml
[dependencies]
claw-tools = { version = "0.1", features = ["hot-loading"] }
```

```rust
use claw_tools::{ToolRegistry, Tool};

let mut registry = ToolRegistry::new();

// 从目录加载
registry.load_from_directory("./tools").await?;

// 启用热加载
registry.enable_hot_loading().await?;

// 执行工具
let result = registry.execute("calculator", json!({
    "operation": "add",
    "a": 1,
    "b": 2
})).await?;
```

---

## 编写工具

### Lua（默认）

```lua
-- calculator.lua
-- @name calculator
-- @description Perform calculations
-- @permissions none
-- @schema { ... }

local M = {}

function M.execute(params)
    return { success = true, result = params.a + params.b }
end

return M
```

### TypeScript (Deno)

```typescript
// @name calculator
// @permissions none

export function execute(params: { a: number; b: number }) {
    return { success: true, result: params.a + params.b };
}
```

---

## 工具模式

工具通过 JSON Schema 声明其接口：

```lua
-- @schema {
--   "type": "object",
--   "properties": {
--     "query": { 
--       "type": "string",
--       "description": "Search query"
--     },
--     "limit": {
--       "type": "integer",
--       "minimum": 1,
--       "maximum": 100,
--       "default": 10
--     }
--   },
--   "required": ["query"]
-- }
```

---

## 权限

工具声明所需权限：

```lua
-- @permissions fs.read, net.http, memory.read
```

可用权限：
- `none` — 无特殊访问
- `fs.read` — 读取文件
- `fs.write` — 写入文件
- `net.http` — HTTP 请求
- `memory.read` / `memory.write` — 智能体记忆
- `process.spawn` — 子进程（仅强力模式）

---

## 热加载

```rust
// 监视文件变化并自动加载
registry.enable_hot_loading().await?;

// 或手动触发
registry.load_from_script("./new_tool.lua").await?;
```

---

## 自定义工具 (Rust)

```rust
use claw_tools::{Tool, ToolResult};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(JsonSchema, Deserialize)]
struct CalculatorParams {
    a: f64,
    b: f64,
    operation: String,
}

pub struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str {
        "calculator"
    }
    
    fn schema(&self) -> Value {
        serde_json::to_value(CalculatorParams::schema()).unwrap()
    }
    
    fn permissions(&self) -> PermissionSet {
        PermissionSet::empty()
    }
    
    async fn execute(&self, params: Value) -> Result<ToolResult, ToolError> {
        let params: CalculatorParams = serde_json::from_value(params)?;
        
        let start = Instant::now();
        let result = match params.operation.as_str() {
            "add" => params.a + params.b,
            "subtract" => params.a - params.b,
            _ => return Err(ToolError::invalid_operation(&params.operation)),
        };
        
        Ok(ToolResult {
            output: Some(json!(result)),
            error: None,
            logs: vec![],
            execution_time_ms: start.elapsed().as_millis() as u64,
        })
    }
}
```

---

## 特性

```toml
[features]
default = ["hot-loading"]
hot-loading = ["notify"]  # 文件监视
schema-gen = ["schemars"]
```
