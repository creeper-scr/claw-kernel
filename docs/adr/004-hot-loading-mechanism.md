[English](#english) | [中文](#chinese)

<a name="english"></a>
# ADR 004: Tool Hot-Loading as Extension Infrastructure

**Status:** Accepted  
**Date:** 2024-02-01  
**Deciders:** claw-kernel core team

---

## Context

Hot-loading is a core kernel capability for runtime extensibility. It enables:
1. Loading new tools (scripts) without restart
2. Updating existing tools dynamically
3. Immediate availability after loading

This capability serves as infrastructure for higher-level features, but **the kernel itself does not dictate**:
- What content should be hot-loaded
- When hot-loading should occur
- Who decides to trigger hot-loading

These decisions belong to the **application layer** (e.g., a self-evolving system, a plugin manager, or a development tool).

---

## Decision

Implement **file-system based hot-loading** as a kernel service with the following flow:

```
Application decides to load tool
            │
            ▼
Write tool script ──► ~/.local/share/claw-kernel/tools/
            │                      │
            │                      ▼
            │               File system watcher (notify crate)
            │                      │
            ▼                      ▼
ToolRegistry validates ◄─── File change detected
            │
            ▼
ScriptEngine compiles
            │
            ▼
Tool registered & available immediately
```

**Kernel Responsibility Boundary:**
- ✅ Provide hot-loading mechanism (watcher, validation, compilation)
- ✅ Ensure safe execution (sandbox, permission audit)
- ❌ Decide what/when to load (application layer decision)
- ❌ Implement self-evolution logic (out of scope for kernel)

> **Note on Layer Boundary:** Hot-loading is a **Layer 3 (Extension Foundation)** kernel capability. How applications use this capability (e.g., for self-evolving agents, plugin systems) is an application-layer concern (Layers 4-5), not part of the kernel.

### Key Mechanisms

**1. Watcher-Based Discovery**

```rust
pub struct ToolWatcher {
    watcher: RecommendedWatcher,
    tools_dir: PathBuf,
}

impl ToolWatcher {
    pub async fn run(mut self, registry: Arc<ToolRegistry>) {
        while let Ok(event) = self.rx.recv().await {
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    for path in event.paths {
                        if path.extension() == Some("lua") {
                            registry.load_from_script(&path).await;
                        }
                    }
                }
                EventKind::Remove(_) => {
                    // Unload removed tools
                }
                _ => {}
            }
        }
    }
}
```

**2. Validation Pipeline**

Before loading, scripts must pass:

1. **Syntax check** — Engine-specific parsing
2. **Permission audit** — Verify declared permissions match Safe Mode policy
3. **Schema validation** — Tool schema must be valid JSON Schema
4. **Sandbox compilation** — Compile in isolated context first

```rust
pub async fn load_from_script(&self, path: &Path) -> Result<()> {
    // 1. Read
    let source = fs::read_to_string(path).await?;
    
    // 2. Syntax
    let ast = self.engine.parse(&source)?;
    
    // 3. Permission audit (Safe Mode only)
    if self.mode == ExecutionMode::Safe {
        let declared = extract_permissions(&ast)?;
        self.audit_permissions(&declared)?;
    }
    
    // 4. Schema validation
    let schema = extract_schema(&ast)?;
    validate_schema(&schema)?;
    
    // 5. Compile
    let compiled = self.engine.compile(&source)?;
    
    // 6. Register
    let tool = ScriptTool::new(compiled, self.bridge.clone());
    self.registry.register(tool)?;
    
    // 7. Emit event
    self.events.emit(Event::ToolLoaded { 
        name: tool.name(),
        source: path.to_path_buf(),
    });
    
    Ok(())
}
```

**3. Version Management**

Tools can be versioned for rollback:

```
~/.local/share/claw-kernel/tools/
├── file_search/
│   ├── v1/
│   │   └── tool.lua
│   ├── v2/
│   │   └── tool.lua
│   └── current -> v2/
└── web_scraper/
    └── ...
```

---

## Consequences

### Positive (Kernel Level)

- **Runtime extensibility:** No restart required for tool updates
- **Clean separation:** Kernel provides capability, application decides usage
- **Version control friendly:** Tools are just files in git
- **Debugging:** Edit script, save, immediate test
- **Flexible deployment:** Supports manual, automated, or AI-driven tool management (at application layer)

### Negative (Kernel Level)

- **File system dependency:** Requires writable directory
- **Race conditions:** Multiple processes writing simultaneously
- **Orphaned tools:** Removed from disk but still in memory

### Mitigations

- Lock files for concurrent writes
- TTL-based cleanup for orphaned tools
- Clear documentation of kernel/application boundaries

---

## Alternatives Considered

### Alternative 1: In-Memory Only

**Rejected:** Lost on restart, no persistence

### Alternative 2: Database Storage

**Rejected:** Adds dependency, harder to version control

### Alternative 3: Compile to Shared Library

**Rejected:** Platform-specific (.so/.dll/.dylib), complex build

---

## Implementation Details

### Tool Script Format (Lua Example)

```lua
-- file_search.lua
-- @name file_search
-- @description Search files by pattern
-- @permissions fs.read
-- @schema {
--   "type": "object",
--   "properties": {
--     "pattern": { "type": "string" },
--     "directory": { "type": "string" }
--   },
--   "required": ["pattern"]
-- }

local M = {}

function M.execute(params)
    local pattern = params.pattern
    local directory = params.directory or "."
    
    -- Use RustBridge for filesystem access
    local files = rust.fs.glob(directory, pattern)
    
    return {
        success = true,
        result = files
    }
end

return M
```

### Hot-Loading vs Cold-Start

| Aspect | Hot-Loading | Cold-Start |
|--------|-----------|------------|
| Latency | ~10-100ms | ~100-500ms |
| State preserved | Yes | No |
| Memory leaks possible | Yes (mitigated by TTL) | No |
| Use case | Development, dynamic updates | Production stability |

---

## Usage Example

```rust
// Application manually triggers hot-loading
let kernel = Kernel::new();
kernel.tools().load_from_path("./my_tool.lua").await?;
```

> **Note:** The kernel provides the `load_from_path` API as infrastructure. How applications decide *when* and *what* to load (e.g., implementing self-evolving systems, plugin managers) is outside the kernel scope.

---

## References

- [Writing Tools Guide](../guides/writing-tools.md)
- [claw-tools crate docs](../crates/claw-tools.md)

---

<a name="chinese"></a>
# ADR 004: 工具热加载作为扩展性基础设施

**状态：** 已接受  
**日期：** 2024-02-01  
**决策者：** claw-kernel 核心团队

---

## 背景

热加载是内核的运行时可扩展性核心能力。它支持：
1. 无需重启即可加载新工具（脚本）
2. 动态更新现有工具
3. 加载后立即可用

此能力作为上层功能的基础设施，但 **内核本身不规定**：
- 应该热加载什么内容
- 何时进行热加载
- 由谁决定触发热加载

这些决策属于 **应用层**（例如，自进化系统、插件管理器或开发工具）。

---

## 决策

实现**基于文件系统的热加载**作为内核服务，流程如下：

```
应用决定加载工具
            │
            ▼
写入工具脚本 ──► ~/.local/share/claw-kernel/tools/
            │                      │
            │                      ▼
            │               文件系统监听器（notify crate）
            │                      │
            ▼                      ▼
ToolRegistry 验证 ◄─── 检测到文件变更
            │
            ▼
ScriptEngine 编译
            │
            ▼
工具注册并立即可用
```

**内核职责边界：**
- ✅ 提供热加载机制（监听器、验证、编译）
- ✅ 确保安全执行（沙箱、权限审计）
- ❌ 决定加载什么/何时加载（应用层决策）
- ❌ 实现自进化逻辑（超出内核范围）

> **分层边界说明：** 热加载是**第 3 层（扩展基础 / Extension Foundation）**的内核能力。应用如何使用此能力（例如，用于自进化智能体、插件系统）是应用层（第 4-5 层）的问题，不属于内核的一部分。

### 关键机制

**1. 基于监听器的发现**

```rust
pub struct ToolWatcher {
    watcher: RecommendedWatcher,
    tools_dir: PathBuf,
}

impl ToolWatcher {
    pub async fn run(mut self, registry: Arc<ToolRegistry>) {
        while let Ok(event) = self.rx.recv().await {
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    for path in event.paths {
                        if path.extension() == Some("lua") {
                            registry.load_from_script(&path).await;
                        }
                    }
                }
                EventKind::Remove(_) => {
                    // 卸载已移除的工具
                }
                _ => {}
            }
        }
    }
}
```

**2. 验证管道**

加载前，脚本必须通过：

1. **语法检查** — 引擎特定解析
2. **权限审计** — 验证声明的权限是否符合安全模式策略
3. **模式验证** — 工具模式必须是有效的 JSON Schema
4. **沙箱编译** — 首先在隔离上下文中编译

```rust
pub async fn load_from_script(&self, path: &Path) -> Result<()> {
    // 1. 读取
    let source = fs::read_to_string(path).await?;
    
    // 2. 语法
    let ast = self.engine.parse(&source)?;
    
    // 3. 权限审计（仅安全模式）
    if self.mode == ExecutionMode::Safe {
        let declared = extract_permissions(&ast)?;
        self.audit_permissions(&declared)?;
    }
    
    // 4. 模式验证
    let schema = extract_schema(&ast)?;
    validate_schema(&schema)?;
    
    // 5. 编译
    let compiled = self.engine.compile(&source)?;
    
    // 6. 注册
    let tool = ScriptTool::new(compiled, self.bridge.clone());
    self.registry.register(tool)?;
    
    // 7. 发送事件
    self.events.emit(Event::ToolLoaded { 
        name: tool.name(),
        source: path.to_path_buf(),
    });
    
    Ok(())
}
```

**3. 版本管理**

工具可以版本化以便回滚：

```
~/.local/share/claw-kernel/tools/
├── file_search/
│   ├── v1/
│   │   └── tool.lua
│   ├── v2/
│   │   └── tool.lua
│   └── current -> v2/
└── web_scraper/
    └── ...
```

---

## 后果

### 积极方面（内核层面）

- **运行时可扩展性：** 无需重启即可更新工具
- **职责清晰分离：** 内核提供能力，应用决定如何使用
- **版本控制友好：** 工具只是 git 中的文件
- **调试便捷：** 编辑脚本，保存，立即测试
- **部署灵活：** 支持手动、自动化或 AI 驱动的工具管理（在应用层）

### 消极方面（内核层面）

- **文件系统依赖：** 需要可写目录
- **竞争条件：** 多个进程同时写入
- **孤立工具：** 从磁盘移除但仍在内存中

### 缓解措施

- 并发写入的锁文件
- 孤立工具的基于 TTL 的清理
- 清晰记录内核与应用层的边界

---

## 考虑的替代方案

### 替代方案 1：仅内存

**已拒绝：** 重启丢失，无持久化

### 替代方案 2：数据库存储

**已拒绝：** 增加依赖，更难版本控制

### 替代方案 3：编译为共享库

**已拒绝：** 平台特定（.so/.dll/.dylib），构建复杂

---

## 实现细节

### 工具脚本格式（Lua 示例）

```lua
-- file_search.lua
-- @name file_search
-- @description 按模式搜索文件
-- @permissions fs.read
-- @schema {
--   "type": "object",
--   "properties": {
--     "pattern": { "type": "string" },
--     "directory": { "type": "string" }
--   },
--   "required": ["pattern"]
-- }

local M = {}

function M.execute(params)
    local pattern = params.pattern
    local directory = params.directory or "."
    
    -- 使用 RustBridge 进行文件系统访问
    local files = rust.fs.glob(directory, pattern)
    
    return {
        success = true,
        result = files
    }
end

return M
```

### 热加载 vs 冷启动

| 方面 | 热加载 | 冷启动 |
|------|--------|--------|
| 延迟 | ~10-100ms | ~100-500ms |
| 状态保持 | 是 | 否 |
| 可能发生内存泄漏 | 是（通过 TTL 缓解） | 否 |
| 用例 | 开发、动态更新 | 生产稳定性 |

---

## 使用示例

```rust
// 应用手动触发热加载
let kernel = Kernel::new();
kernel.tools().load_from_path("./my_tool.lua").await?;
```

> **注意：** 内核提供 `load_from_path` API 作为基础设施。应用如何决定*何时*和*加载什么*（例如，实现自进化系统、插件管理器）超出了内核范围。

---

## 参考

- [编写工具指南](../guides/writing-tools.md)
- [claw-tools crate 文档](../crates/claw-tools.md)
