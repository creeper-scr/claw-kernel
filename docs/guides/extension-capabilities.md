---
title: Extension Capabilities Guide
description: Extension points and runtime evolution guide
status: implemented
version: "1.0.0"
last_updated: "2026-03-01"
language: en
---



# Extension Capabilities Guide

claw-kernel provides **infrastructure** for building extensible agents. This guide explains the capabilities the kernel offers at Layer 2 (Agent Kernel Protocol) and Layer 3 (Extension Foundation).

> [Info] **Note**: This guide documents the implemented API in v1.0.0.

---

## Kernel Capabilities

### What claw-kernel Provides

| Capability | Description | Layer |
|------------|-------------|-------|
| **Script Hot-Loading** | Load and execute Lua scripts at runtime without restart | Layer 3 |
| **Dynamic Tool Registration** | Register new tools with the ToolRegistry at any time | Layer 2 |
| **Runtime Extension Points** | Hooks for watching file changes, tool lifecycle events | Layer 2 |
| **Sandboxed Execution** | Secure environment for running untrusted tool code | Layer 0.5 |
| **Permission Bridge** | Enforce security boundaries between scripts and system | Layer 3 |

---

## Kernel Capabilities in Detail

### 1. Script Hot-Loading

Load Lua scripts dynamically without restarting the agent:

```rust
use claw_kernel::tools::ToolRegistry;

let mut tools = ToolRegistry::new();

// Enable hot-loading watching
tools.enable_hot_loading().await?;

// Load tools from directory
tools.load_from_directory("./tools").await?;

// Later: newly added scripts are automatically available
```

### 2. Dynamic Tool Registration

Register tools programmatically at runtime:

```rust
// Register a new tool from Lua source
let tool_source = std::fs::read_to_string("./new_tool.lua")?;
tools.register_lua_tool("new_tool", &tool_source).await?;

// Tool is immediately available to the agent
```

### 3. Runtime Extension Points

```rust
use claw_kernel::tools::{ToolRegistry, ToolEvent};

let mut tools = ToolRegistry::new();

// Watch for tool lifecycle events
tools.on_event(|event| match event {
    ToolEvent::ToolLoaded { name } => {
        println!("Tool loaded: {}", name);
    }
    ToolEvent::ToolUnloaded { name } => {
        println!("Tool unloaded: {}", name);
    }
    ToolEvent::ToolModified { name } => {
        println!("Tool modified: {}", name);
        // Application can trigger reload or validation
    }
});
```

---

## Using Kernel Extension Capabilities

Applications built on claw-kernel can leverage these capabilities to implement their own extension mechanisms:

```rust
use claw_kernel::{
    provider::AnthropicProvider,
    loop_::AgentLoop,
    tools::ToolRegistry,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = AnthropicProvider::from_env()?;
    
    // Setup hot-loading (kernel capability)
    let mut tools = ToolRegistry::new();
    tools.enable_hot_loading().await?;
    tools.load_from_directory("./tools").await?;
    
    // Build agent loop
    let agent = AgentLoop::builder()
        .provider(provider)
        .tools(tools)
        .build();
    
    // ... run agent
    
    Ok(())
}
```

---

## Best Practices

### 1. Clear Separation of Concerns

```rust
// Kernel handles: hot-loading, sandboxing, tool execution
// Application handles: what tools to create, when to load them
```

### 2. Tool Lifecycle Management

Applications can implement tool management using kernel capabilities:

```lua
-- Example: List available tools (application-level tool)
-- @name list_tools
-- @description List all available tools
-- @permissions none

function M.execute(params)
    local tools_dir = rust.dirs.tools_dir()
    local entries = rust.fs.list_dir(tools_dir)
    
    local tools = {}
    for _, entry in ipairs(entries) do
        if entry.type == "file" and entry.name:match("%.lua$") then
            table.insert(tools, entry.name:gsub("%.lua$", ""))
        end
    end
    
    return { success = true, result = tools }
end
```

### 3. Safety Considerations

- Tools cannot exceed declared permissions (kernel enforces)
- Tool code is sandboxed (kernel provides)
- Applications should maintain audit logs

---

## Debugging Tools

### Enable Debug Logging

```rust
std::env::set_var("RUST_LOG", "claw_script=debug,claw_tools=debug");
```

### Test in Isolation

```bash
# Test a specific tool
cargo run --example tool_tester -- --tool calculator --input '{"a": 2, "b": 3}'
```

### Review Tool Events

```rust
// Application can log all tool events
tools.on_event(|event| {
    log::info!("Tool event: {:?}", event);
});
```

---

## Summary

| Aspect | claw-kernel | Application |
|--------|-------------|-------------|
| **Hot-loading** | Yes Provides infrastructure | Yes Decides what to load |
| **Tool execution** | Yes Sandboxed runtime | Yes Defines tool logic |
| **Code generation** | No Not implemented | Application decision |
| **Extension strategy** | No Not implemented | Application decision |

**claw-kernel provides infrastructure for extensibility. Applications decide how to use it.**

---

## See Also

- [Writing Tools](writing-tools.md) — Tool development basics
- [Architecture Overview](../architecture/overview.md) — How hot-loading works

---
