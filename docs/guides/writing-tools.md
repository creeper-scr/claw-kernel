---
title: Writing Custom Tools
description: Guide for writing custom tools with scripts
status: implemented
version: "1.0.0"
last_updated: "2026-03-01"
language: en
---



# Writing Custom Tools

> ⚠️ **Pre-release notice:** v0.4.0 is a beta and may be unstable. APIs are subject to change without notice.

Tools are the primary way to extend your agent's capabilities at Layer 2 (Agent Kernel Protocol) and Layer 3 (Extension Foundation). This guide covers writing tools in Lua (default) and TypeScript/Deno.

> [Info] **Note**: This guide documents the implemented API in v1.0.0.

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
