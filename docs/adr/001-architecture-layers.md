---
title: "ADR-001: Five-Layer Architecture with PAL"
type: adr
status: accepted
date: "2026-02-28"
---

[English](#english) | [中文](#chinese)

<a name="english"></a>
# ADR 001: Five-Layer Architecture with PAL

> 中文：五层架构与 PAL

**Status:** Accepted  
**Date:** 2024-01-15  
**Deciders:** claw-kernel core team

---

## Context

The Claw ecosystem has 8+ implementations (OpenClaw, ZeroClaw, PicoClaw, Nanobot, etc.) each independently implementing:

- LLM provider HTTP calls
- Tool-use protocol parsing
- Agent loop management
- Memory systems
- Channel integrations

This leads to:
- Wasted engineering effort
- Inconsistent behavior across implementations
- Difficulty sharing improvements

We need a shared foundation that:
1. Eliminates duplicate code
2. Supports cross-platform deployment
3. Provides extensibility for application innovations
4. Maintains high performance

---

## Decision

We will adopt a **five-layer architecture** with a dedicated **Platform Abstraction Layer (PAL)** at Layer 0.5.

```
Layer 3: Extension Foundation (Script Runtime)  ← Extension interface (Kernel boundary)
-------------------------------------------------
Layer 2: Agent Kernel Protocol                   ← Core kernel
Layer 1: System Runtime                          ← System primitives
Layer 0.5: Platform Abstraction (PAL)            ← Platform bridge
Layer 0: Rust Hard Core                          ← Foundation
```

### Architecture Boundary

**Kernel Core (Layers 0-3):**
- Minimal, stable, high-performance foundation
- Written in Rust for memory safety and zero-cost abstractions
- Provides extensibility hooks but no application logic
- Extension Foundation (Layer 3) is the outermost boundary of the kernel

> **Note:** Layers 4-5 (Application Plugins and Application Layer) are **outside the kernel**. The kernel provides infrastructure for applications to build upon, but application-specific logic and plugin systems are implemented by applications themselves.

### Key Design Choices

**1. Rust for Core (Layers 0-3)**
- Memory safety without GC
- Zero-cost abstractions
- Cross-platform compilation
- Strong async/await support via Tokio

**2. Extension Foundation as Kernel Boundary (Layer 3)**
- Hot-swappable without restart
- Multiple language options (Lua/TS/Python)
- Provides extension interface for applications
- Applications implement self-evolution via scripts, not kernel features

**3. Dedicated PAL Layer**
- Forces platform-agnostic thinking
- Makes platform gaps visible
- Enables per-platform optimization

**4. Self-Evolution is NOT in Kernel**

Self-evolution (the ability for agents to modify their own behavior) is intentionally **outside the kernel**. The kernel provides the infrastructure (hot-loading, script runtime) that applications can use to implement self-evolution. The rationale:

- **Separation of Concerns**: Kernel provides extensibility primitives; evolution logic belongs to applications
- **Stability**: Core kernel should remain minimal and stable
- **Flexibility**: Different applications may want different self-evolution strategies
- **Safety**: Evolution code runs in script runtime with proper sandboxing, not in privileged kernel space
- **Innovation**: Application developers can experiment with evolution algorithms without kernel changes

The kernel's responsibility ends at providing a robust extension mechanism (Layer 3). How that mechanism is used—including for self-evolution—is an application concern.

---

## Consequences

### Positive

- **Code reuse:** Single implementation of provider/tool/loop primitives
- **Cross-platform:** Linux/macOS/Windows equality by design
- **Extensibility:** Scripts can be generated and hot-loaded at application layer (using kernel infrastructure)
- **Type safety:** Rust core catches errors at compile time
- **Performance:** No GC pauses, predictable latency
- **Stability:** Minimal kernel reduces attack surface and maintenance burden
- **Clear boundaries:** Kernel scope is well-defined (Layers 0-3 only)

### Negative

- **Build complexity:** Multiple engines (Lua/V8/Python) complicate builds
- **Learning curve:** Contributors need Rust knowledge for core changes
- **Binary size:** V8 engine adds ~100MB (mitigated by feature flags)

### Neutral

- **Script debugging:** Requires tooling for Lua/TS/Python debugging

---

## Alternatives Considered

### Alternative 1: Pure TypeScript (like OpenClaw)

**Rejected:** Single-threaded, memory-heavy (>1GB), difficult to sandbox

### Alternative 2: Pure Rust (no scripting)

**Rejected:** No extensibility capability, requires recompile for new tools

### Alternative 3: WASM instead of scripts

**Considered:** Better sandboxing, but tooling immature, harder to debug

### Alternative 4: Self-Evolution in Kernel

**Rejected:** Violates separation of concerns; kernel should be minimal and provide primitives, not implement high-level application behaviors

### Alternative 5: No PAL, platform code scattered

**Rejected:** Would lead to same fragmentation we're solving

### Alternative 6: Including Layers 4-5 in Kernel

**Rejected:** Would make the kernel too large and opinionated. Application plugins and application logic should be handled by applications built on top of the kernel, not be part of the kernel itself.

---

## References

- [Architecture Overview](../architecture/overview.md)
- [Platform Abstraction Layer](../architecture/pal.md)
- [Crate Map](../architecture/crate-map.md)

---

<a name="chinese"></a>
# ADR 001: 五层架构与 PAL

**状态：** 已接受  
**日期：** 2024-01-15  
**决策者：** claw-kernel 核心团队

---

## 背景

Claw 生态系统有 8 个以上的实现（OpenClaw、ZeroClaw、PicoClaw、Nanobot 等），每个都独立实现：

- LLM 提供商 HTTP 调用
- 工具使用协议解析
- 智能体循环管理
- 内存系统
- 通道集成

这导致：
- 工程工作浪费
- 各实现之间行为不一致
- 难以共享改进

我们需要一个共享基础：
1. 消除重复代码
2. 支持跨平台部署
3. 提供扩展性以支持应用创新
4. 保持高性能

---

## 决策

我们将采用**五层架构**，在第 0.5 层设有专用的**平台抽象层（PAL）》。

```
第 3 层：扩展基础 (Extension Foundation / Script Runtime)  ← 扩展接口（内核边界）
-----------------------------------------------------------
第 2 层：智能体内核协议                                    ← 核心内核
第 1 层：系统运行时                                        ← 系统原语
第 0.5 层：平台抽象层（PAL）                               ← 平台桥接
第 0 层：Rust 硬核核心                                     ← 基础层
```

### 架构边界

**内核核心（第 0-3 层）：**
- 最小化、稳定、高性能的基础
- 使用 Rust 编写，确保内存安全和零成本抽象
- 提供扩展性钩子，但不包含应用逻辑
- 扩展基础（第 3 层）是内核的最外层边界

> **注意：** 第 4-5 层（应用插件和应用层）**不在内核范围内**。内核为应用提供基础设施，但应用特定逻辑和插件系统由应用自行实现。

### 关键设计选择

**1. 核心使用 Rust（第 0-3 层）**
- 无需 GC 的内存安全
- 零成本抽象
- 跨平台编译
- 通过 Tokio 强大的异步/等待支持

**2. 扩展基础作为内核边界（第 3 层）**
- 无需重启即可热插拔
- 多种语言选项（Lua/TS/Python）
- 为应用提供扩展接口
- 应用通过脚本实现自进化，而非内核功能

**3. 专用 PAL 层**
- 强制平台无关思维
- 使平台差异可见
- 支持每平台优化

**4. 自进化不在内核中**

自进化（智能体修改自身行为的能力）被有意设计为**内核之外**。内核提供基础设施（热加载、脚本运行时、扩展基础），应用可以使用这些设施来实现自进化。理由如下：

- **关注点分离**：内核提供扩展性原语；进化逻辑属于应用
- **稳定性**：核心内核应保持最小化和稳定
- **灵活性**：不同应用可能需要不同的自进化策略
- **安全性**：进化代码在脚本运行时中执行，具有适当的沙箱隔离，而非在特权内核空间中
- **创新性**：应用开发者可以在无需修改内核的情况下试验进化算法

内核的职责止于提供健壮的扩展机制（第 3 层）。如何使用该机制——包括用于自进化——是应用层面的问题。

---

## 后果

### 积极方面

- **代码复用：** 统一的 provider/tool/loop 原语实现
- **跨平台：** 按设计实现 Linux/macOS/Windows 平等
- **扩展性：** 脚本可以在应用层生成和热加载（使用内核基础设施）
- **类型安全：** Rust 核心在编译时捕获错误
- **性能：** 无 GC 暂停，可预测延迟
- **稳定性：** 最小化内核减少攻击面和维护负担
- **边界清晰：** 内核范围定义明确（仅第 0-3 层）

### 消极方面

- **构建复杂性：** 多引擎（Lua/V8/Python）使构建复杂化
- **学习曲线：** 贡献者需要 Rust 知识才能修改核心
- **二进制大小：** V8 引擎增加约 100MB（通过特性标志缓解）

### 中性方面

- **脚本调试：** 需要 Lua/TS/Python 调试工具

---

## 考虑的替代方案

### 替代方案 1：纯 TypeScript（如 OpenClaw）

**已拒绝：** 单线程，内存占用高（>1GB），难以沙箱化

### 替代方案 2：纯 Rust（无脚本）

**已拒绝：** 无扩展性能力，新工具需要重新编译

### 替代方案 3：WASM 代替脚本

**已考虑：** 更好的沙箱化，但工具不成熟，更难调试

### 替代方案 4：自进化在内核中

**已拒绝：** 违反关注点分离原则；内核应保持最小化并提供原语，而非实现高级应用行为

### 替代方案 5：无 PAL，平台代码分散

**已拒绝：** 会导致我们正在解决的碎片化问题

### 替代方案 6：在内核中包含第 4-5 层

**已拒绝：** 会使内核过于庞大和固执己见。应用插件和应用逻辑应由基于内核构建的应用处理，而非内核的一部分。

---

## 参考

- [架构概览](../architecture/overview.md)
- [平台抽象层](../architecture/pal.md)
- [Crate 映射](../architecture/crate-map.md)
