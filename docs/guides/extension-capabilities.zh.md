---
title: 扩展能力指南
description: Extension points and runtime evolution guide
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](extension-capabilities.md)


# 扩展能力指南

claw-kernel 提供构建可扩展智能体的**基础设施**。本指南说明内核在 Layer 2（Agent 内核协议）和 Layer 3（扩展基础）提供的能力。

---

## 内核能力

### claw-kernel 提供什么

| 能力 | 描述 | 层级 |
|------|------|------|
| **脚本热加载** | 运行时加载和执行 Lua 脚本，无需重启 | Layer 3 |
| **动态工具注册** | 随时向 ToolRegistry 注册新工具 | Layer 2 |
| **运行时扩展点** | 文件变更监听、工具生命周期事件钩子 | Layer 2 |
| **沙箱 (Sandbox)执行** | 安全运行不受信任的工具代码 | Layer 0.5 |

---

## 内核能力详解

### 1. 脚本热加载

动态加载 Lua 脚本，无需重启智能体：

```rust
use claw_kernel::tools::ToolRegistry;

let mut tools = ToolRegistry::new();

// 启用热加载监听
tools.enable_hot_loading().await?;

// 从目录加载工具
tools.load_from_directory("./tools").await?;

// 之后：新添加的脚本会自动可用
```

### 2. 动态工具注册

运行时以编程方式注册工具：

```rust
// 从 Lua 源码注册新工具
let tool_source = std::fs::read_to_string("./new_tool.lua")?;
tools.register_lua_tool("new_tool", &tool_source).await?;

// 工具立即可用于智能体
```

### 3. 运行时扩展点

```rust
use claw_kernel::tools::{ToolRegistry, ToolEvent};

let mut tools = ToolRegistry::new();

// 监听工具生命周期事件
tools.on_event(|event| match event {
    ToolEvent::ToolLoaded { name } => {
        println!("工具已加载: {}", name);
    }
    ToolEvent::ToolUnloaded { name } => {
        println!("工具已卸载: {}", name);
    }
    ToolEvent::ToolModified { name } => {
        println!("工具已修改: {}", name);
        // 应用层可以触发重载或验证
    }
});
```

---

## 使用内核扩展能力

基于 claw-kernel 构建的应用可以利用这些能力实现自己的扩展机制：

```rust
use claw_kernel::{
    provider::AnthropicProvider,
    loop_::AgentLoop,
    tools::ToolRegistry,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = AnthropicProvider::from_env()?;
    
    // 设置热加载（内核能力）
    let mut tools = ToolRegistry::new();
    tools.enable_hot_loading().await?;
    tools.load_from_directory("./tools").await?;
    
    // 构建智能体循环
    let agent = AgentLoop::builder()
        .provider(provider)
        .tools(tools)
        .build();
    
    // ... 运行智能体
    
    Ok(())
}
```

---

## 最佳实践

### 1. 清晰的职责分离

```rust
// 内核处理：热加载、沙箱 (Sandbox)、工具执行
// 应用层处理：创建什么工具、何时加载
```

### 2. 工具生命周期管理

应用可以使用内核能力实现工具管理：

```lua
-- 示例：列出可用工具（应用层工具）
-- @name list_tools
-- @description 列出所有可用工具
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

### 3. 安全考虑

- 工具不能超过声明的权限（内核强制执行）
- 工具代码在沙箱 (Sandbox)中运行（内核提供）
- 应用层应维护审计日志

---

## 调试工具

### 启用调试日志

```rust
std::env::set_var("RUST_LOG", "claw_script=debug,claw_tools=debug");
```

### 隔离测试

```bash
# 测试特定工具
cargo run --example tool_tester -- --tool calculator --input '{"a": 2, "b": 3}'
```

### 审查工具事件

```rust
// 应用层可以记录所有工具事件
tools.on_event(|event| {
    log::info!("工具事件: {:?}", event);
});
```

---

## 总结

| 方面 | claw-kernel | 应用层 |
|------|-------------|--------|
| **热加载** | Yes 提供基础设施 | Yes 决定加载什么 |
| **工具执行** | Yes 沙箱 (Sandbox)运行时 | Yes 定义工具逻辑 |
| **代码生成** | No 未实现 | 应用层决定 |
| **扩展策略** | No 未实现 | 应用层决定 |

**claw-kernel 提供可扩展性的基础设施。应用层决定如何使用它。**

---

## 另请参阅

- [编写工具](writing-tools.md) — 工具开发基础
- [架构概述](../architecture/overview.md) — 热加载工作原理
