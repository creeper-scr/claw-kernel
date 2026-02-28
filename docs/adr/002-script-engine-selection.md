---
title: "ADR-002: Multi-Engine Script Support"
type: adr
status: accepted
date: "2026-02-28"
---

[English](#english) | [中文](#chinese)

<a name="english"></a>
# ADR 002: Multi-Engine Script Support (Lua Default)

**Status:** Accepted  
**Date:** 2024-01-20  
**Deciders:** claw-kernel core team

---

## Context

We need a scripting layer that:
1. Supports **extensibility** and **hot-loading** for user customization
2. Is cross-platform
3. Has minimal dependencies for quick builds
4. Can leverage existing ecosystems (ML, web)

No single engine satisfies all requirements.

---

## Decision

Support **multiple script engines** with **Lua as default**:

| Engine | Status | Use Case |
|--------|--------|----------|
| **Lua (mlua)** | Default, always available | Simple tools, fast builds |
| **Deno/V8** | Optional feature | Complex agents, full JS/TS |
| **Python (PyO3)** | Optional feature | ML ecosystem integration |

### Lua as Default Rationale

```toml
[features]
default = ["engine-lua"]
engine-lua = ["mlua"]
engine-v8 = ["deno_core"]
engine-py = ["pyo3"]
```

**Why Lua:**
- Pure Rust binding (mlua), **zero system dependencies**
- **Lightweight**: runtime <500KB, compiles in <1 minute
- Sufficient for most tool logic
- Excellent C FFI if needed
- Provides a solid foundation for **application extensibility** — users can customize and extend functionality without recompiling

**Trade-off:** Less familiar than JS/Python, but simple enough to learn quickly.

### Unified Bridge API

All engines expose the same `RustBridge` interface:

```typescript
// Same API regardless of engine (simplified view - see claw-script.md for full definition)
interface RustBridge {
  llm: { complete(messages: Message[]): Promise<Response> };
  tools: { register(def: ToolDef): void; call(name: string, params: any): Promise<any>; list(): ToolMeta[] };
  memory: { get(key: string): Promise<any>; set(key: string, value: any): Promise<void>; search(query: string, topK: number): Promise<MemoryItem[]> };
  events: { emit(event: string, data: any): void; on(event: string, handler: Function): void };
  fs: { read(path: string): Promise<Buffer>; write(path: string, data: Buffer): Promise<void> };
}
```

---

## Consequences

### Positive

- **Fast default builds:** Lua only, no heavy deps
- **Flexibility:** Users choose engine by feature flag
- **Ecosystem access:** Python for ML, JS for web
- **Migration path:** Start with Lua, upgrade to V8 if needed
- **Extensibility:** Users can customize behavior via scripts without modifying core code

### Negative

- **Maintenance burden:** Three engine implementations to maintain
- **Behavior differences:** Edge cases may differ between engines
- **Documentation complexity:** Must document all three

### Mitigations

- Comprehensive test suite runs against all engines
- Bridge API is strictly typed and tested
- Users can lock to one engine in production

---

## Alternatives Considered

### Alternative 1: Deno/V8 Only

**Rejected:** 100MB+ binary, complex Windows build, slow compilation

### Alternative 2: Python Only

**Rejected:** GIL limits concurrency, sandboxing difficult

### Alternative 3: WASM (Wasmer/Wasmtime)

**Considered:** Best sandboxing, but:
- Language tooling immature (debugging, stack traces)
- Memory overhead per instance
- Complex host function binding

**Decision:** Revisit WASM for plugin isolation in future, not for main engine.

---

## Implementation Notes

### Engine Selection at Runtime

```rust
/// Engine type selector for runtime engine selection
pub enum EngineType {
    Lua,
    #[cfg(feature = "engine-v8")]
    V8,
    #[cfg(feature = "engine-py")]
    Python,
}

/// Script engine wrapper (actual engine instance)
pub enum ScriptEngine {
    Lua(LuaEngine),
    #[cfg(feature = "engine-v8")]
    V8(V8Engine),
    #[cfg(feature = "engine-py")]
    Python(PythonEngine),
}

impl ScriptEngine {
    pub fn new(engine_type: EngineType) -> Result<Self> {
        match engine_type {
            EngineType::Lua => Ok(Self::Lua(LuaEngine::new()?)),
            #[cfg(feature = "engine-v8")]
            EngineType::V8 => Ok(Self::V8(V8Engine::new()?)),
            #[cfg(feature = "engine-py")]
            EngineType::Python => Ok(Self::Python(PythonEngine::new()?)),
            _ => Err(Error::EngineNotAvailable),
        }
    }
}
```

### Per-Engine Permissions

Different engines have different sandboxing capabilities:

| Engine | Sandboxing | Permission Model |
|--------|-----------|------------------|
| Lua | Limited (code can crash host) | Runtime checks |
| Deno | Strong (V8 isolate) | Deno permissions |
| Python | Weak (GIL doesn't isolate) | OS-level only |

Recommendation: Use Safe Mode OS sandbox for all engines; Deno's built-in sandbox is additional defense.

---

## References

- [claw-script crate docs](../crates/claw-script.md)
- [mlua documentation](https://github.com/khvzak/mlua)
- [deno_core documentation](https://docs.rs/deno_core)
- [PyO3 documentation](https://pyo3.rs)

---

<a name="chinese"></a>
# ADR 002: 多引擎脚本支持（Lua 默认）

**状态：** 已接受  
**日期：** 2024-01-20  
**决策者：** claw-kernel 核心团队

---

## 背景

我们需要一个脚本层，满足：
1. 支持**扩展性**和**热加载**，允许用户自定义功能
2. 跨平台
3. 依赖最小，快速构建
4. 可以利用现有生态系统（ML、web）

没有单一引擎能满足所有要求。

---

## 决策

支持**多脚本引擎**，以 **Lua 作为默认**：

| 引擎 | 状态 | 用例 |
|------|------|------|
| **Lua (mlua)** | 默认，始终可用 | 简单工具，快速构建 |
| **Deno/V8** | 可选特性 | 复杂智能体，完整 JS/TS |
| **Python (PyO3)** | 可选特性 | ML 生态系统集成 |

### 以 Lua 为默认的理由

```toml
[features]
default = ["engine-lua"]
engine-lua = ["mlua"]
engine-v8 = ["deno_core"]
engine-py = ["pyo3"]
```

**为什么选择 Lua：**
- 纯 Rust 绑定（mlua），**零系统依赖**
- **轻量**：运行时 <500KB，编译时间 <1 分钟
- 足以满足大多数工具逻辑
- 优秀的 C FFI 支持
- 为**应用扩展性**提供坚实基础——用户无需重新编译即可自定义和扩展功能

**权衡：** 不如 JS/Python 熟悉，但足够简单可快速学习。

### 统一桥接 API

所有引擎暴露相同的 `RustBridge` 接口：

```typescript
// 无论使用哪个引擎，API 都相同（简化视图 - 完整定义参见 claw-script.md）
interface RustBridge {
  llm: { complete(messages: Message[]): Promise<Response> };
  tools: { register(def: ToolDef): void; call(name: string, params: any): Promise<any>; list(): ToolMeta[] };
  memory: { get(key: string): Promise<any>; set(key: string, value: any): Promise<void>; search(query: string, topK: number): Promise<MemoryItem[]> };
  events: { emit(event: string, data: any): void; on(event: string, handler: Function): void };
  fs: { read(path: string): Promise<Buffer>; write(path: string, data: Buffer): Promise<void> };
}
```

---

## 后果

### 积极方面

- **默认构建快速：** 仅 Lua，无繁重依赖
- **灵活性：** 用户通过特性标志选择引擎
- **生态系统访问：** Python 用于 ML，JS 用于 web
- **迁移路径：** 从 Lua 开始，需要时升级到 V8
- **可扩展性：** 用户可通过脚本自定义行为，无需修改核心代码

### 消极方面

- **维护负担：** 需要维护三个引擎实现
- **行为差异：** 边缘情况可能在引擎间不同
- **文档复杂性：** 必须记录所有三个引擎

### 缓解措施

- 全面的测试套件针对所有引擎运行
- 桥接 API 是严格类型化并经过测试的
- 用户可以在生产中锁定到一个引擎

---

## 考虑的替代方案

### 替代方案 1：仅 Deno/V8

**已拒绝：** 二进制文件 >100MB，Windows 构建复杂，编译慢

### 替代方案 2：仅 Python

**已拒绝：** GIL 限制并发，沙箱化困难

### 替代方案 3：WASM（Wasmer/Wasmtime）

**已考虑：** 最佳沙箱化，但是：
- 语言工具不成熟（调试、堆栈跟踪）
- 每个实例的内存开销
- 复杂的主机函数绑定

**决策：** 将来重新审视 WASM 用于插件隔离，不作为主引擎。

---

## 实现说明

### 运行时引擎选择

```rust
/// Engine type selector for runtime engine selection
pub enum EngineType {
    Lua,
    #[cfg(feature = "engine-v8")]
    V8,
    #[cfg(feature = "engine-py")]
    Python,
}

/// Script engine wrapper (actual engine instance)
pub enum ScriptEngine {
    Lua(LuaEngine),
    #[cfg(feature = "engine-v8")]
    V8(V8Engine),
    #[cfg(feature = "engine-py")]
    Python(PythonEngine),
}

impl ScriptEngine {
    pub fn new(engine_type: EngineType) -> Result<Self> {
        match engine_type {
            EngineType::Lua => Ok(Self::Lua(LuaEngine::new()?)),
            #[cfg(feature = "engine-v8")]
            EngineType::V8 => Ok(Self::V8(V8Engine::new()?)),
            #[cfg(feature = "engine-py")]
            EngineType::Python => Ok(Self::Python(PythonEngine::new()?)),
        }
    }
}
```

### 每引擎权限

不同引擎有不同的沙箱化能力：

| 引擎 | 沙箱化 | 权限模型 |
|------|--------|----------|
| Lua | 有限（代码可能崩溃主机） | 运行时检查 |
| Deno | 强（V8 隔离） | Deno 权限 |
| Python | 弱（GIL 不隔离） | 仅 OS 级别 |

建议：对所有引擎使用安全模式 OS 沙箱；Deno 内置沙箱是额外的防御。

---

## 参考

- [claw-script crate 文档](../crates/claw-script.md)
- [mlua 文档](https://github.com/khvzak/mlua)
- [deno_core 文档](https://docs.rs/deno_core)
- [PyO3 文档](https://pyo3.rs)
