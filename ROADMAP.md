---
title: claw-kernel Roadmap
description: Milestone-based implementation roadmap for claw-kernel
status: design-phase
version: "0.1.0"
last_updated: "2026-02-28"
language: bilingual
---

[English](#english) | [中文](#chinese)

<a name="english"></a>

# claw-kernel Roadmap

> **Current Status: Pre-v0.1.0 — Design/Planning Phase**
>
> The `crates/` directory is empty. No code has been implemented yet. All milestones below are planned targets, not commitments.

---

## Current Status

| Item | Status |
|------|--------|
| Architecture design | Complete |
| ADRs (001-008) | All accepted |
| Design specs | In progress |
| Implementation | Not started |
| Published on crates.io | No |

See [docs/adr/](docs/adr/) for all Architecture Decision Records.

---

## Milestones

### v0.1.0-alpha — Foundation (Phase 1 + 2)

**Target:** 2026 Q2

Establishes the cross-platform base that everything else builds on.

**claw-pal (Platform Abstraction Layer):**
- [ ] `SandboxBackend` trait definition
- [ ] Linux sandbox (seccomp-bpf + namespaces)
- [ ] macOS sandbox (sandbox(7) profile / Seatbelt)
- [ ] Windows sandbox (AppContainer + Job Objects)
- [ ] Cross-platform IPC abstraction (Unix Domain Socket / Named Pipe)
- [ ] Process lifecycle management

**claw-runtime (System Runtime):**
- [ ] `EventBus` implementation
- [ ] `Runtime` struct with Tokio integration
- [ ] IPC router
- [ ] `AgentOrchestrator` for multi-agent coordination
- [ ] A2A (Agent-to-Agent) message protocol

---

### v0.2.0-alpha — Core Protocols (Phase 3)

**Target:** 2026 Q3

Adds the LLM provider abstraction and tool registry.

**claw-provider:**
- [ ] `LLMProvider` trait (three-layer: MessageFormat, HttpTransport, LLMProvider)
- [ ] `EmbeddingProvider` trait
- [ ] Anthropic (Claude) implementation
- [ ] OpenAI-compatible implementation
- [ ] DeepSeek implementation
- [ ] Moonshot / Qwen / Grok implementations
- [ ] Azure OpenAI implementation
- [ ] Ollama (local) implementation

**claw-tools:**
- [ ] `Tool` trait with permission declarations
- [ ] `ToolRegistry` with hot-loading support
- [ ] JSON Schema generation via `schemars`
- [ ] Script-based tool loading (Lua)
- [ ] File watcher for automatic reload (`notify`)

---

### v0.3.0-alpha — Agent Loop (Phase 4)

**Target:** 2026 Q3

The core agent loop engine with history and stop conditions.

**claw-loop:**
- [ ] `AgentLoop` struct and builder pattern
- [ ] `AgentLoopConfig` (max turns, token budget, streaming, timeouts)
- [ ] `StopCondition` trait + built-in conditions (MaxTurns, TokenBudget, NoToolCall)
- [ ] `HistoryManager` trait
- [ ] In-memory history implementation
- [ ] SQLite history backend (optional feature `sqlite`)
- [ ] `Summarizer` trait for context compression

---

### v0.4.0-alpha — Script Runtime (Phase 5)

**Target:** 2026 Q4

Embedded scripting with hot-reload and the Rust bridge API.

**claw-script:**
- [ ] `ScriptEngine` trait and `EngineType` enum
- [ ] Lua engine (default, via `mlua`)
- [ ] `RustBridge` API (llm, tools, memory, events, fs, net)
- [ ] Hot-reload mechanism (file watching + registry update)
- [ ] Deno/V8 engine (optional feature `engine-v8`)
- [ ] Python engine (optional feature `engine-py`)

---

### v0.5.0-alpha — Channel Integrations (Phase 6)

**Target:** 2026 Q4

External communication interfaces.

**claw-channel:**
- [ ] `Channel` trait
- [ ] Telegram integration
- [ ] Discord integration
- [ ] HTTP webhook

---

### v0.9.0-beta — Examples and Documentation (Phase 7)

**Target:** 2027 Q1

Runnable examples and complete API documentation.

- [ ] `simple-agent` example
- [ ] `custom-tool` example
- [ ] `self-evolving-agent` example
- [ ] Full rustdoc API documentation
- [ ] Architecture Decision Records updated
- [ ] Migration guide (for when breaking changes occur)

---

### v1.0.0 — Stable Release (Phase 8)

**Target:** 2027 Q1

Meta-crate, integration tests, and stability guarantee.

**claw-kernel (meta-crate):**
- [ ] Re-exports all crates with unified feature flags
- [ ] Cross-platform integration test suite
- [ ] Performance benchmarks
- [ ] Stable API guarantee (semver)
- [ ] Published on crates.io

---

## Design Decisions

Key architectural choices are recorded as ADRs in [docs/adr/](docs/adr/):

| ADR | Decision |
|-----|----------|
| 001 | 5-layer architecture |
| 002 | Script engine selection (Lua default, V8/Python optional) |
| 003 | Security model (Safe/Power dual-mode) |
| 004 | Hot-loading mechanism |
| 005 | IPC and multi-agent protocol |
| 006-008 | Additional accepted decisions |

---

## Contributing

Want to help build this? Check [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

Priority areas right now (design phase):
- Review and improve architecture documentation
- Windows sandbox research and design
- New LLM provider interface design
- Script engine bridge API design

---

<a name="chinese"></a>

# claw-kernel 路线图

> **当前状态：v0.1.0 之前 —— 设计/规划阶段**
>
> `crates/` 目录为空，尚未实现任何代码。以下所有里程碑均为计划目标，不构成承诺。

---

## 当前状态

| 项目 | 状态 |
|------|------|
| 架构设计 | 已完成 |
| ADR（001-008） | 全部已接受 |
| 设计规范 | 进行中 |
| 实现 | 未开始 |
| 发布到 crates.io | 否 |

所有架构决策记录请参见 [docs/adr/](docs/adr/)。

---

## 里程碑

### v0.1.0-alpha — 基础层（第 1 + 2 阶段）

**目标时间：** 2026 年 Q2

建立跨平台基础，其他所有内容都在此之上构建。

**claw-pal（平台抽象层）：**
- [ ] `SandboxBackend` trait 定义
- [ ] Linux 沙箱（seccomp-bpf + namespaces）
- [ ] macOS 沙箱（sandbox(7) profile / Seatbelt）
- [ ] Windows 沙箱（AppContainer + Job Objects）
- [ ] 跨平台 IPC 抽象（Unix Domain Socket / Named Pipe）
- [ ] 进程生命周期管理

**claw-runtime（系统运行时）：**
- [ ] `EventBus` 实现
- [ ] 集成 Tokio 的 `Runtime` 结构
- [ ] IPC 路由器
- [ ] 多 Agent 协调的 `AgentOrchestrator`
- [ ] A2A（Agent 间）消息协议

---

### v0.2.0-alpha — 核心协议（第 3 阶段）

**目标时间：** 2026 年 Q3

添加 LLM Provider 抽象和工具注册表。

**claw-provider：**
- [ ] `LLMProvider` trait（三层架构：MessageFormat、HttpTransport、LLMProvider）
- [ ] `EmbeddingProvider` trait
- [ ] Anthropic（Claude）实现
- [ ] OpenAI 兼容实现
- [ ] DeepSeek 实现
- [ ] Moonshot / Qwen / Grok 实现
- [ ] Azure OpenAI 实现
- [ ] Ollama（本地）实现

**claw-tools：**
- [ ] 带权限声明的 `Tool` trait
- [ ] 支持热加载的 `ToolRegistry`
- [ ] 通过 `schemars` 生成 JSON Schema
- [ ] 基于脚本的工具加载（Lua）
- [ ] 自动重载的文件监视器（`notify`）

---

### v0.3.0-alpha — Agent 循环（第 4 阶段）

**目标时间：** 2026 年 Q3

核心 Agent 循环引擎，包含历史管理和停止条件。

**claw-loop：**
- [ ] `AgentLoop` 结构和构建器模式
- [ ] `AgentLoopConfig`（最大轮次、Token 预算、流式传输、超时）
- [ ] `StopCondition` trait + 内置条件（MaxTurns、TokenBudget、NoToolCall）
- [ ] `HistoryManager` trait
- [ ] 内存历史实现
- [ ] SQLite 历史后端（可选特性 `sqlite`）
- [ ] 用于上下文压缩的 `Summarizer` trait

---

### v0.4.0-alpha — 脚本运行时（第 5 阶段）

**目标时间：** 2026 年 Q4

支持热加载和 Rust 桥接 API 的嵌入式脚本。

**claw-script：**
- [ ] `ScriptEngine` trait 和 `EngineType` 枚举
- [ ] Lua 引擎（默认，通过 `mlua`）
- [ ] `RustBridge` API（llm、tools、memory、events、fs、net）
- [ ] 热加载机制（文件监视 + 注册表更新）
- [ ] Deno/V8 引擎（可选特性 `engine-v8`）
- [ ] Python 引擎（可选特性 `engine-py`）

---

### v0.5.0-alpha — 渠道集成（第 6 阶段）

**目标时间：** 2026 年 Q4

外部通信接口。

**claw-channel：**
- [ ] `Channel` trait
- [ ] Telegram 集成
- [ ] Discord 集成
- [ ] HTTP Webhook

---

### v0.9.0-beta — 示例与文档（第 7 阶段）

**目标时间：** 2027 年 Q1

可运行的示例和完整的 API 文档。

- [ ] `simple-agent` 示例
- [ ] `custom-tool` 示例
- [ ] `self-evolving-agent` 示例
- [ ] 完整的 rustdoc API 文档
- [ ] 架构决策记录更新
- [ ] 迁移指南（用于发生破坏性变更时）

---

### v1.0.0 — 稳定版发布（第 8 阶段）

**目标时间：** 2027 年 Q1

元 crate、集成测试和稳定性保证。

**claw-kernel（元 crate）：**
- [ ] 统一特性标志重导出所有 crate
- [ ] 跨平台集成测试套件
- [ ] 性能基准测试
- [ ] 稳定 API 保证（语义化版本）
- [ ] 发布到 crates.io

---

## 设计决策

关键架构选择记录为 [docs/adr/](docs/adr/) 中的 ADR：

| ADR | 决策 |
|-----|------|
| 001 | 5 层架构 |
| 002 | 脚本引擎选择（Lua 默认，V8/Python 可选） |
| 003 | 安全模型（Safe/Power 双模式） |
| 004 | 热加载机制 |
| 005 | IPC 和多 Agent 协议 |
| 006-008 | 其他已接受的决策 |

---

## 贡献

想参与构建？请查看 [CONTRIBUTING.md](CONTRIBUTING.md) 了解指南。

当前（设计阶段）优先领域：
- 审查和改进架构文档
- Windows 沙箱研究与设计
- 新 LLM Provider 接口设计
- 脚本引擎桥接 API 设计
