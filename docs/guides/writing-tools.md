[English](#english) | [中文](#chinese)

<a name="english"></a>

# Writing Custom Tools

Tools are the primary way to extend your agent's capabilities at Layer 2 (Agent Kernel Protocol) and Layer 3 (Extension Foundation). This guide covers writing tools in Lua (default), with notes on TypeScript/Deno and Python alternatives.

> ⚠️ **Note**: This guide shows the **target API design**. The `claw-kernel` crate is not yet implemented.

---

## Tool Structure

A tool consists of:

1. **Metadata** — Name, description, permissions
2. **Schema** — JSON Schema defining parameters
3. **Execute function** — The actual logic

---

## Lua Tool (Default)

### Basic Example

```lua
-- greet.lua
-- @name greet
-- @description Greet a user by name
-- @permissions none
-- @schema {
--   "type": "object",
--   "properties": {
--     "name": { "type": "string", "description": "Name to greet" },
--     "enthusiasm": { 
--       "type": "integer", 
--       "minimum": 1, 
--       "maximum": 5,
--       "description": "How enthusiastic (1-5)"
--     }
--   },
--   "required": ["name"]
-- }

local M = {}

function M.execute(params)
    local name = params.name
    local enthusiasm = params.enthusiasm or 3
    
    local greeting = "Hello"
    for i = 1, enthusiasm do
        greeting = greeting .. "!"
    end
    
    return {
        success = true,
        result = greeting .. " " .. name
    }
end

return M
```

### With File System Access

```lua
-- read_file.lua
-- @name read_file
-- @description Read contents of a file
-- @permissions fs.read
-- @schema {
--   "type": "object",
--   "properties": {
--     "path": { 
--       "type": "string", 
--       "description": "Path to file (relative or absolute)" 
--     }
--   },
--   "required": ["path"]
-- }

local M = {}

function M.execute(params)
    local path = params.path
    
    -- Use RustBridge for file operations
    -- This respects Safe Mode restrictions
    local content, err = rust.fs.read(path)
    
    if err then
        return {
            success = false,
            error = "Failed to read file: " .. tostring(err)
        }
    end
    
    return {
        success = true,
        result = content
    }
end

return M
```

### With HTTP Requests

```lua
-- fetch_weather.lua
-- @name fetch_weather
-- @description Get weather for a city
-- @permissions net.http
-- @schema {
--   "type": "object",
--   "properties": {
--     "city": { "type": "string" }
--   },
--   "required": ["city"]
-- }

local M = {}

function M.execute(params)
    local city = params.city
    local url = "https://api.weather.example/v1/current?city=" .. city
    
    local response, err = rust.net.get(url)
    
    if err then
        return { success = false, error = tostring(err) }
    end
    
    local data = rust.json.parse(response.body)
    
    return {
        success = true,
        result = {
            temperature = data.temp,
            condition = data.condition
        }
    }
end
```

---

## RustBridge API Reference

Tools can access system capabilities through `rust.*`:

### File System

```lua
-- Read file to string
local content = rust.fs.read("/path/to/file")

-- Write string to file
rust.fs.write("/path/to/file", "content")

-- Check if exists
local exists = rust.fs.exists("/path/to/file")

-- List directory
local entries = rust.fs.list_dir("/path/to/dir")
-- Returns: [{ name: "file.txt", type: "file", size: 1234 }, ...]

-- Glob pattern
local matches = rust.fs.glob("/path/*.txt")
```

### HTTP

```lua
-- GET request
local response = rust.net.get("https://api.example.com/data")
-- Returns: { status: 200, headers: {}, body: "..." }

-- POST request
local response = rust.net.post(
    "https://api.example.com/data",
    { ["Content-Type"] = "application/json" },
    '{"key": "value"}'
)

-- Note: Safe Mode restricts allowed domains
```

### JSON

```lua
-- Parse JSON string to Lua table
local data = rust.json.parse('{"key": "value"}')

-- Serialize Lua table to JSON string
local json = rust.json.stringify({ key = "value" }, { pretty = true })
```

### Memory

```lua
-- Store value
rust.memory.set("key", { data = "value" })

-- Retrieve value
local value = rust.memory.get("key")

-- Note: Stored as JSON, supports any serializable data
```

### Events

```lua
-- Emit event
rust.events.emit("tool_completed", { tool = "my_tool" })

-- Subscribe to events (advanced)
rust.events.on("custom_event", function(data)
    print("Received:", data)
end)
```

---

## Permission Declaration

Always declare required permissions:

```lua
-- @permissions fs.read, net.http, memory.write
```

Available permissions:

| Permission | Description |
|------------|-------------|
| `none` | No special permissions (default) |
| `fs.read` | Read files from allowlisted directories |
| `fs.write` | Write files to allowlisted directories |
| `net.http` | Make HTTP requests (Safe Mode restricts domains) |
| `memory.read` | Read from agent memory |
| `memory.write` | Write to agent memory |
| `process.spawn` | Spawn subprocesses (Power Mode only) |

---

## TypeScript/Deno Tools

For complex tools, use Deno/V8 engine:

```typescript
// fetch_analytics.ts
// @name fetch_analytics
// @description Fetch and analyze data
// @permissions net.http

interface Params {
    url: string;
    analysis: "summary" | "full";
}

interface Result {
    success: boolean;
    result?: unknown;
    error?: string;
}

export async function execute(params: Params): Promise<Result> {
    try {
        const response = await rust.net.get(params.url);
        const data = rust.json.parse(response.body);
        
        if (params.analysis === "summary") {
            return {
                success: true,
                result: {
                    count: data.length,
                    fields: Object.keys(data[0] || {})
                }
            };
        }
        
        return { success: true, result: data };
    } catch (err) {
        return { success: false, error: String(err) };
    }
}
```

Enable in `Cargo.toml`:

```toml
[dependencies]
claw-kernel = { version = "0.1", features = ["engine-v8"] }
```

---

## Testing Tools

### Manual Testing

```bash
# Place tool in tools directory
mkdir -p ~/.local/share/claw-kernel/tools
cp my_tool.lua ~/.local/share/claw-kernel/tools/

# Run agent that loads tools
cargo run --example tool_tester
```

### Unit Testing (Rust)

```rust
#[cfg(test)]
mod tests {
    use claw_kernel::tools::{ToolRegistry, Tool};
    
    #[tokio::test]
    async fn test_calculator() {
        let mut registry = ToolRegistry::new();
        registry.load_from_script("tools/calculator.lua".into()).await.unwrap();
        
        let tool = registry.get("calculator").unwrap();
        let result = tool.execute(json!({
            "operation": "add",
            "a": 2,
            "b": 3
        })).await.unwrap();
        
        assert_eq!(result["result"], 5);
    }
}
```

---

## Best Practices

1. **Validate inputs** — Check types and ranges
2. **Handle errors gracefully** — Return `{ success: false, error: "..." }`
3. **Keep focused** — One tool = one capability
4. **Document clearly** — LLM uses description to choose tools
5. **Request minimal permissions** — Principle of least privilege

---

## Tool Hot-Loading

The kernel provides tool registration and hot-loading capabilities at Layer 2 and Layer 3:

### Loading Tools

Tools are typically loaded from a directory:

```lua
-- my_api_tool.lua
-- @name my_api_tool
-- @description Call a specific API endpoint
-- @permissions net.http
-- @schema { ... }

local M = {}

function M.execute(params)
    -- Tool implementation
    return { success = true, result = ... }
end

return M
```

### Hot-Loading Support

Applications can enable hot-loading to detect tool changes without restart:

```rust
let mut tools = ToolRegistry::new();
tools.enable_hot_loading().await?;
tools.load_from_directory("./tools").await?;
```

**Key Points:**
- Kernel provides: `Tool` trait, `ToolRegistry`, file system APIs, hot-loading mechanism (Layer 2-3)
- Applications decide: when to load tools, how to organize them

See [Extension Capabilities Guide](extension-capabilities.md) for more on kernel extension features.

---

<a name="chinese"></a>

# 编写自定义工具

工具是在 Layer 2（Agent 内核协议）和 Layer 3（扩展基础）扩展智能体能力的主要方式。本指南介绍如何使用 Lua（默认）编写工具，并提供 TypeScript/Deno 和 Python 替代方案的说明。

---

## 工具结构

一个工具包含：

1. **元数据** — 名称、描述、权限
2. **模式** — 定义参数的 JSON Schema
3. **执行函数** — 实际逻辑

---

## Lua 工具（默认）

### 基础示例

```lua
-- greet.lua
-- @name greet
-- @description 按名称向用户打招呼
-- @permissions none
-- @schema {
--   "type": "object",
--   "properties": {
--     "name": { "type": "string", "description": "要问候的名称" },
--     "enthusiasm": { 
--       "type": "integer", 
--       "minimum": 1, 
--       "maximum": 5,
--       "description": "热情程度 (1-5)"
--     }
--   },
--   "required": ["name"]
-- }

local M = {}

function M.execute(params)
    local name = params.name
    local enthusiasm = params.enthusiasm or 3
    
    local greeting = "Hello"
    for i = 1, enthusiasm do
        greeting = greeting .. "!"
    end
    
    return {
        success = true,
        result = greeting .. " " .. name
    }
end

return M
```

### 文件系统访问

```lua
-- read_file.lua
-- @name read_file
-- @description 读取文件内容
-- @permissions fs.read
-- @schema {
--   "type": "object",
--   "properties": {
--     "path": { 
--       "type": "string", 
--       "description": "文件路径（相对或绝对）" 
--     }
--   },
--   "required": ["path"]
-- }

local M = {}

function M.execute(params)
    local path = params.path
    
    -- 使用 RustBridge 进行文件操作
    -- 这会遵守安全模式限制
    local content, err = rust.fs.read(path)
    
    if err then
        return {
            success = false,
            error = "读取文件失败: " .. tostring(err)
        }
    end
    
    return {
        success = true,
        result = content
    }
end

return M
```

### HTTP 请求

```lua
-- fetch_weather.lua
-- @name fetch_weather
-- @description 获取城市天气
-- @permissions net.http
-- @schema {
--   "type": "object",
--   "properties": {
--     "city": { "type": "string" }
--   },
--   "required": ["city"]
-- }

local M = {}

function M.execute(params)
    local city = params.city
    local url = "https://api.weather.example/v1/current?city=" .. city
    
    local response, err = rust.net.get(url)
    
    if err then
        return { success = false, error = tostring(err) }
    end
    
    local data = rust.json.parse(response.body)
    
    return {
        success = true,
        result = {
            temperature = data.temp,
            condition = data.condition
        }
    }
end
```

---

## RustBridge API 参考

工具可以通过 `rust.*` 访问系统功能：

### 文件系统

```lua
-- 读取文件到字符串
local content = rust.fs.read("/path/to/file")

-- 写入字符串到文件
rust.fs.write("/path/to/file", "content")

-- 检查是否存在
local exists = rust.fs.exists("/path/to/file")

-- 列出目录
local entries = rust.fs.list_dir("/path/to/dir")
-- 返回: [{ name: "file.txt", type: "file", size: 1234 }, ...]

-- Glob 模式匹配
local matches = rust.fs.glob("/path/*.txt")
```

### HTTP

```lua
-- GET 请求
local response = rust.net.get("https://api.example.com/data")
-- 返回: { status: 200, headers: {}, body: "..." }

-- POST 请求
local response = rust.net.post(
    "https://api.example.com/data",
    { ["Content-Type"] = "application/json" },
    '{"key": "value"}'
)

-- 注意：安全模式会限制允许的域名
```

### JSON

```lua
-- 将 JSON 字符串解析为 Lua 表
local data = rust.json.parse('{"key": "value"}')

-- 将 Lua 表序列化为 JSON 字符串
local json = rust.json.stringify({ key = "value" }, { pretty = true })
```

### 内存

```lua
-- 存储值
rust.memory.set("key", { data = "value" })

-- 检索值
local value = rust.memory.get("key")

-- 注意：以 JSON 形式存储，支持任何可序列化数据
```

### 事件

```lua
-- 触发事件
rust.events.emit("tool_completed", { tool = "my_tool" })

-- 订阅事件（高级）
rust.events.on("custom_event", function(data)
    print("收到:", data)
end)
```

---

## 权限声明

始终声明所需权限：

```lua
-- @permissions fs.read, net.http, memory.write
```

可用权限：

| 权限 | 描述 |
|------|------|
| `none` | 无特殊权限（默认） |
| `fs.read` | 从允许列表目录读取文件 |
| `fs.write` | 向允许列表目录写入文件 |
| `net.http` | 发起 HTTP 请求（安全模式会限制域名） |
| `memory.read` | 从智能体内存读取 |
| `memory.write` | 写入智能体内存 |
| `process.spawn` | 生成子进程（仅强力模式） |

---

## TypeScript/Deno 工具

对于复杂工具，使用 Deno/V8 引擎：

```typescript
// fetch_analytics.ts
// @name fetch_analytics
// @description 获取并分析数据
// @permissions net.http

interface Params {
    url: string;
    analysis: "summary" | "full";
}

interface Result {
    success: boolean;
    result?: unknown;
    error?: string;
}

export async function execute(params: Params): Promise<Result> {
    try {
        const response = await rust.net.get(params.url);
        const data = rust.json.parse(response.body);
        
        if (params.analysis === "summary") {
            return {
                success: true,
                result: {
                    count: data.length,
                    fields: Object.keys(data[0] || {})
                }
            };
        }
        
        return { success: true, result: data };
    } catch (err) {
        return { success: false, error: String(err) };
    }
}
```

在 `Cargo.toml` 中启用：

```toml
[dependencies]
claw-kernel = { version = "0.1", features = ["engine-v8"] }
```

---

## 测试工具

### 手动测试

```bash
# 将工具放入工具目录
mkdir -p ~/.local/share/claw-kernel/tools
cp my_tool.lua ~/.local/share/claw-kernel/tools/

# 运行加载工具的示例
cargo run --example tool_tester
```

### 单元测试（Rust）

```rust
#[cfg(test)]
mod tests {
    use claw_kernel::tools::{ToolRegistry, Tool};
    
    #[tokio::test]
    async fn test_calculator() {
        let mut registry = ToolRegistry::new();
        registry.load_from_script("tools/calculator.lua".into()).await.unwrap();
        
        let tool = registry.get("calculator").unwrap();
        let result = tool.execute(json!({
            "operation": "add",
            "a": 2,
            "b": 3
        })).await.unwrap();
        
        assert_eq!(result["result"], 5);
    }
}
```

---

## 最佳实践

1. **验证输入** — 检查类型和范围
2. **优雅地处理错误** — 返回 `{ success: false, error: "..." }`
3. **保持专注** — 一个工具 = 一个能力
4. **清晰文档** — LLM 使用描述来选择工具
5. **请求最小权限** — 最小权限原则

---

## 工具热加载

内核在 Layer 2 和 Layer 3 提供工具注册和热加载能力：

### 加载工具

工具通常从目录加载：

```lua
-- my_api_tool.lua
-- @name my_api_tool
-- @description 调用特定 API 端点
-- @permissions net.http
-- @schema { ... }

local M = {}

function M.execute(params)
    -- 工具实现
    return { success = true, result = ... }
end

return M
```

### 热加载支持

应用可以启用热加载以检测工具变更而无需重启：

```rust
let mut tools = ToolRegistry::new();
tools.enable_hot_loading().await?;
tools.load_from_directory("./tools").await?;
```

**关键点：**
- 内核提供：`Tool` trait、`ToolRegistry`、文件系统 API、热加载机制（Layer 2-3）
- 应用决定：何时加载工具、如何组织它们

有关内核扩展功能的更多信息，请参阅[扩展能力指南](extension-capabilities.md)。
