---
title: ADR-009: claw-script Bridge Roadmap
status: accepted
version: "0.1.0"
date: "2026-03-08"
implemented: "2026-03-08"
---

# ADR-009: claw-script Bridge Roadmap

> **实现状态（2026-03-08）：所有 4 个 Bridge 均已在 v0.1.0 内提前完成。** 原规划时间线作为历史参考保留。

## 实现进度

| Bridge | 原目标版本 | 实际完成 | 文件 |
|--------|-----------|---------|------|
| `dirs` | v0.1.1 | ✅ v0.1.0 | `crates/claw-script/src/bridge/dirs.rs` |
| `memory` | v0.2.0 | ✅ v0.1.0 | `crates/claw-script/src/bridge/memory.rs` |
| `events` | v0.2.0 | ✅ v0.1.0 | `crates/claw-script/src/bridge/events.rs` |
| `agent` | v0.3.0 | ✅ v0.1.0 | `crates/claw-script/src/bridge/agent.rs` |

## 背景

`claw-script` 目前实现了 3 个 Bridge（fs、net、tools），还有 4 个 Bridge 需要实现：
- `dirs` - 目录路径获取
- `memory` - 记忆存储
- `events` - 事件系统
- `agent` - Agent 管理

本 ADR 制定实现优先级和时间线。

## 决策

### 1. 优先级划分

| 优先级 | Bridge | 原因 | 目标版本 |
|--------|--------|------|----------|
| P0 | `dirs` | 实现简单，依赖 claw-pal，高实用性 | v0.1.1 |
| P1 | `memory` | 核心功能，让脚本有状态 | v0.2.0 |
| P2 | `events` | 需要 Runtime 支持，中等复杂度 | v0.2.0 |
| P3 | `agent` | 复杂度高，需要更多设计 | v0.3.0 |

### 2. 实现计划

#### Phase 1: Dirs Bridge (v0.1.1)

**目标**：提供标准目录路径给脚本

**Lua API**:
```lua
local config_dir = rust.dirs.config_dir()   -- ~/.config/claw-kernel/
local data_dir = rust.dirs.data_dir()       -- ~/.local/share/claw-kernel/
local cache_dir = rust.dirs.cache_dir()     -- ~/.cache/claw-kernel/
local tools_dir = rust.dirs.tools_dir()     -- ~/.config/claw-kernel/tools/
```

**实现要点**:
- 复用 `claw-pal::dirs` 模块
- 只需同步方法，无需 async
- 简单的字符串返回

**工作量**: 小 (~2 小时)

---

#### Phase 2: Memory Bridge (v0.2.0)

**目标**：让脚本能够读写长期记忆

**Lua API**:
```lua
-- Key-value 存储
rust.memory.set("last_summary", "some content")
local value = rust.memory.get("last_summary")

-- 语义搜索（返回最相关的记忆）
local results = rust.memory:search("关于 Rust 的讨论", 5)
for _, item in ipairs(results) do
    print(item.content, item.score)
end

-- 带命名空间的存储（自动使用脚本名称作为 namespace）
rust.memory.set("preference", "concise", {namespace = "user"})
```

**实现要点**:
- 依赖 `claw-memory` crate
- 需要异步支持（async/await）
- 使用 `MemoryStore` trait
- 自动使用 agent_id 作为 namespace 隔离

**工作量**: 中 (~1 天)

---

#### Phase 3: Events Bridge (v0.2.0)

**目标**：允许脚本发送和监听事件

**Lua API**:
```lua
-- 发送事件
rust.events.emit("task_completed", {
    task_id = "123",
    result = "success"
})

-- 监听事件（回调函数）
rust.events.on("shutdown", function(data)
    print("Received shutdown signal:", data.reason)
end)

-- 一次性监听
rust.events.once("config_reloaded", function(data)
    print("Config reloaded")
end)
```

**实现要点**:
- 依赖 `claw-runtime` 的 EventBus
- 需要管理 Lua 回调函数的生命周期
- 使用 `mlua::Function` 存储回调
- 需要清理机制（脚本卸载时取消订阅）

**工作量**: 中 (~2 天)

**技术难点**:
- Lua 回调函数需要在 Rust 中安全存储
- 需要处理脚本热重载时的回调清理
- 可能需要在 `ScriptContext` 中跟踪活跃的回调

---

#### Phase 4: Agent Bridge (v0.3.0)

**目标**：允许脚本创建和管理其他 Agent

**Lua API**:
```lua
-- 创建子 Agent
local handle = rust.agent.spawn({
    name = "analyzer",
    provider = "openai",
    model = "gpt-4o-mini",
    system_prompt = "You are a data analyzer..."
})

-- 向子 Agent 发送任务
local result = rust.agent:send_message(handle, "Analyze this data: ...")

-- 获取 Agent 状态
local status = rust.agent.status(handle)

-- 终止 Agent
rust.agent.kill(handle)

-- 列出活跃的子 Agent
local agents = rust.agent.list()
```

**实现要点**:
- 依赖 `claw-runtime` 的 AgentOrchestrator
- 需要 `AgentHandle` 的序列化/反序列化
- 复杂的生命周期管理
- 需要防止循环创建导致的资源泄漏

**工作量**: 大 (~3-5 天)

**技术难点**:
- Agent 生命周期管理复杂
- 需要防止资源泄漏（忘记 kill 的 Agent）
- 父子 Agent 的权限继承问题
- 错误处理（Agent panic、超时等）

---

### 3. 通用实现模式

每个 Bridge 的实现遵循以下模式：

```rust
// 1. 定义 Bridge 结构体
pub struct XxxBridge {
    // 依赖的其他组件
    inner: Arc<dyn XxxTrait>,
    // 上下文信息
    agent_id: String,
}

// 2. 为 mlua 实现 UserData
impl UserData for XxxBridge {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method("method_name", |lua, this, args| async move {
            // 实现逻辑
        });
    }
}

// 3. 注册函数
pub fn register_xxx(lua: &Lua, bridge: XxxBridge) -> LuaResult<()> {
    lua.globals().set("xxx", bridge)
}
```

### 4. 安全考虑

- **dirs**: 只返回路径，无安全风险
- **memory**: 使用 namespace 隔离，防止脚本访问其他 agent 的记忆
- **events**: 需要防止事件风暴（速率限制）
- **agent**: 最危险，需要严格的资源限制（最大子 Agent 数、执行时间等）

## 时间线

```
v0.1.1 (2周内)
└── dirs Bridge

v0.2.0 (1-2个月)
├── memory Bridge
└── events Bridge

v0.3.0 (待定)
└── agent Bridge (需要更多设计和测试)
```

## 相关文档

- `docs/crates/claw-script.md` - 用户文档
- `crates/claw-script/src/bridge/` - 实现代码
- `claw-memory` crate - memory Bridge 依赖
- `claw-runtime` crate - events/agent Bridge 依赖
