---
title: claw-kernel
description: The shared foundation for the Claw ecosystem — a cross-platform Agent Kernel built in Rust
status: design-phase
version: "0.1.0"
last_updated: "2026-02-28"
language: bilingual
license: MIT OR Apache-2.0
---

[English](#english) | [中文](#chinese)

<a name="english"></a>

# claw-kernel

> The shared Rust foundation for the Claw agent ecosystem — cross-platform, sandboxed, hot-loadable.

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Status](https://img.shields.io/badge/status-design%20phase-orange.svg)](ROADMAP.md)
[![Rust](https://img.shields.io/badge/rust-1.83%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg)](docs/platform/)

> **⚠️ Design/Planning Phase** — The `crates/` directory is empty. No code has been implemented yet.
> The documentation describes the **planned** API and architecture. See [BUILD_PLAN.md](BUILD_PLAN.md) for the implementation roadmap.

---

## The Problem

Every project in the Claw ecosystem independently reimplements the same primitives:

| Primitive | OpenClaw | ZeroClaw | PicoClaw | NanoClaw | ... |
|-----------|:--------:|:--------:|:--------:|:--------:|:---:|
| LLM Provider HTTP calls | ✓ | ✓ | ✓ | ✓ | ✓ |
| Tool-use protocol | ✓ | ✓ | ✓ | ✓ | ✓ |
| Agent loop | ✓ | ✓ | ✓ | ✓ | ✓ |
| Memory system | ✓ | ✓ | ✓ | ✓ | ✓ |
| Channel integrations | ✓ | ✓ | ✓ | ✓ | ✓ |

**claw-kernel** extracts these into a single, well-tested, cross-platform library.

---

## What is claw-kernel?

A **shared infrastructure library**, not a standalone agent. Think of it as the Linux kernel to your agent's userspace: a minimal, stable core that handles the hard systems-level work.

**It is:** a reusable foundation, a sandboxed execution environment, a hot-loadable tool registry.  
**It is NOT:** a complete agent, a framework, or published on crates.io yet.

**Design principles:** Rust kernel + script logic · Cross-platform first · Safe/Power dual modes · Minimal core (Lua default, V8/Python opt-in)

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│              Layer 3: Extension Foundation          │
│    Lua (default) · Deno/V8 · PyO3                   │
├─────────────────────────────────────────────────────┤
│              Layer 2: Agent Kernel Protocol         │
│    Provider Trait · ToolRegistry · AgentLoop        │
├─────────────────────────────────────────────────────┤
│              Layer 1: System Runtime                │
│    Event Bus · IPC Transport · Process Daemon       │
├═════════════════════════════════════════════════════╡
│              Layer 0.5: Platform Abstraction (PAL)  │
│    Sandbox Backend · IPC Primitives · Config Dirs   │
├─────────────────────────────────────────────────────┤
│              Layer 0: Rust Hard Core                │
│    Memory Safety · OS Abstraction · Trust Root      │
└─────────────────────────────────────────────────────┘
```

Full details: [docs/architecture/overview.md](docs/architecture/overview.md)

---

## Crate Ecosystem

| Crate | Description | Layer |
|-------|-------------|-------|
| [`claw-pal`](docs/crates/claw-pal.md) | Platform Abstraction Layer (sandbox, IPC, process) | 0.5 |
| [`claw-runtime`](docs/crates/claw-runtime.md) | Event bus, async runtime, multi-agent orchestration | 1 |
| [`claw-provider`](docs/crates/claw-provider.md) | LLM provider trait + Anthropic/OpenAI/Ollama/DeepSeek/Qwen | 2 |
| [`claw-tools`](docs/crates/claw-tools.md) | Tool-use protocol, registry, schema gen, hot-loading | 2 |
| [`claw-loop`](docs/crates/claw-loop.md) | Agent loop engine, history management, stop conditions | 2 |
| [`claw-script`](docs/crates/claw-script.md) | Embedded script engines (Lua default, Deno/V8, PyO3) | 3 |

---

## Platform Support

| Platform | Sandbox Backend | Isolation |
|----------|-----------------|-----------|
| Linux | seccomp-bpf + Namespaces | Strongest |
| macOS | sandbox(7) profile (Seatbelt) | Medium |
| Windows | AppContainer + Job Objects | Medium |

Platform guides: [Linux](docs/platform/linux.md) · [macOS](docs/platform/macos.md) · [Windows](docs/platform/windows.md)

---

## Quick Start

> **API design only — not yet implemented.**

```toml
[dependencies]
claw-provider = "0.1"   # not yet published
claw-tools    = "0.1"   # not yet published
claw-loop     = "0.1"   # not yet published
claw-kernel = { version = "0.1", features = ["engine-lua"] }  # optional: full kernel
```

```rust
use claw_provider::AnthropicProvider;
use claw_tools::ToolRegistry;
use claw_loop::AgentLoop;

#[tokio::main]
async fn main() {
    let provider = AnthropicProvider::from_env();
    let tools = ToolRegistry::new();
    let mut agent = AgentLoop::builder().provider(provider).tools(tools).build();
    agent.run("Hello, world!").await.unwrap();
}
```

---

## Execution Modes

**Safe Mode (default):** file system allowlist, network rules, no subprocess spawning, all scripts sandboxed.

**Power Mode:** full system access, requires explicit opt-in:

```bash
claw-kernel --power-mode --power-key <your-key>
```

Power Key: minimum 12 characters, at least two character types. See [Safe Mode](docs/guides/safe-mode.md) · [Power Mode](docs/guides/power-mode.md).

---

## Documentation

| Document | Description |
|----------|-------------|
| [Architecture Overview](docs/architecture/overview.md) | Full 5-layer architecture |
| [Crate Map](docs/architecture/crate-map.md) | Dependency graph |
| [Agent Loop State Machine](docs/design/agent-loop-state-machine.md) | Loop design spec |
| [Channel Message Protocol](docs/design/channel-message-protocol.md) | Channel integration spec |
| [Getting Started](docs/guides/getting-started.md) | Build your first agent |
| [Writing Tools](docs/guides/writing-tools.md) | Custom tools with scripts |
| [Extension Capabilities](docs/guides/extension-capabilities.md) | Hot-loading and runtime evolution |
| [Architecture Decisions](docs/adr/) | ADRs 001-008 (all accepted) |
| [Build Plan](BUILD_PLAN.md) | 8-phase implementation roadmap |

---

## Who Is This For?

- **Claw ecosystem developers** tired of rewriting the same provider/loop/tool code across projects
- **Rust developers** who want a solid, async, cross-platform foundation for agent systems
- **Researchers** who need a scriptable, extensible agent runtime

---

## Getting Help

- **Questions:** [GitHub Discussions](../../discussions)
- **Bugs and features:** [GitHub Issues](../../issues)
- **Security:** [SECURITY.md](SECURITY.md)

---

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Priority areas: Windows sandbox hardening · new LLM providers (Gemini, Mistral, local GGUF) · Lua/Deno bridge improvements · platform test coverage.

---

## License

Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE)) OR MIT License ([LICENSE-MIT](LICENSE-MIT)) — your choice.

---

<a name="chinese"></a>

# claw-kernel（爪核）

> Claw Agent 生态系统的共享 Rust 基础库 —— 跨平台、沙箱化、支持热加载。

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Status](https://img.shields.io/badge/status-design%20phase-orange.svg)](ROADMAP.md)
[![Rust](https://img.shields.io/badge/rust-1.83%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg)](docs/platform/)

> **⚠️ 设计/规划阶段** —— `crates/` 目录为空，尚未实现任何代码。
> 文档描述的是**计划中**的 API 和架构。实现路线图请参见 [BUILD_PLAN.md](BUILD_PLAN.md)。

---

## 问题背景

Claw 生态系统中的每个项目都在独立重复实现相同的基础功能（LLM Provider 调用、工具协议、Agent 循环、记忆系统、渠道集成）。**claw-kernel** 将这些提取到一个经过充分测试的跨平台库中。

---

## 什么是 claw-kernel？

**共享基础设施库**，不是独立 Agent。它是：可复用的构建基础、双模式沙箱执行环境、支持热加载的工具注册表。

**不是：** 完整 Agent、强制框架、已发布到 crates.io 的库。

**设计原则：** Rust 内核 + 脚本逻辑 · 跨平台优先 · 安全/强力双模式 · 最小核心（Lua 默认，V8/Python 可选）

---

## 架构

五层架构（第 0 层到第 3 层）：Rust 硬核 → 平台抽象层（PAL）→ 系统运行时 → Agent 内核协议 → 扩展基础（Lua/V8/Python）。

完整文档：[docs/architecture/overview.md](docs/architecture/overview.md)

---

## Crate 生态系统

| Crate | 描述 | 所属层 |
|-------|------|--------|
| [`claw-pal`](docs/crates/claw-pal.md) | 平台抽象层（沙箱、IPC、进程管理） | 0.5 |
| [`claw-runtime`](docs/crates/claw-runtime.md) | 事件总线、异步运行时、多 Agent 编排 | 1 |
| [`claw-provider`](docs/crates/claw-provider.md) | LLM Provider trait + Anthropic/OpenAI/Ollama/DeepSeek/Qwen | 2 |
| [`claw-tools`](docs/crates/claw-tools.md) | 工具协议、注册表、Schema 生成、热加载 | 2 |
| [`claw-loop`](docs/crates/claw-loop.md) | Agent 循环引擎、历史管理、停止条件 | 2 |
| [`claw-script`](docs/crates/claw-script.md) | 嵌入式脚本引擎（Lua 默认、Deno/V8、PyO3） | 3 |

---

## 平台支持

| 平台 | 沙箱后端 | 隔离强度 |
|------|---------|---------|
| Linux | seccomp-bpf + Namespaces | 最强 |
| macOS | sandbox(7) profile (Seatbelt) | 中等 |
| Windows | AppContainer + Job Objects | 中等 |

平台指南：[Linux](docs/platform/linux.md) · [macOS](docs/platform/macos.md) · [Windows](docs/platform/windows.md)

---

## 快速开始

> **仅为 API 设计，尚未实现。** 代码示例与英文部分相同，请参见上方 [Quick Start](#quick-start)。

---

## 执行模式

**安全模式（默认）：** 文件系统白名单、网络规则、禁止子进程、脚本沙箱化。

**强力模式：** 完全系统访问，需显式选择：`claw-kernel --power-mode --power-key <your-key>`

Power Key 要求：最少 12 位，包含至少两种字符类型。指南：[安全模式](docs/guides/safe-mode.md) · [强力模式](docs/guides/power-mode.md)

---

## 文档

| 文档 | 描述 |
|------|------|
| [架构概览](docs/architecture/overview.md) | 完整的 5 层架构 |
| [Crate 图谱](docs/architecture/crate-map.md) | 所有 crate 的依赖图 |
| [Agent 循环状态机](docs/design/agent-loop-state-machine.md) | 循环设计规范 |
| [渠道消息协议](docs/design/channel-message-protocol.md) | 渠道集成规范 |
| [入门指南](docs/guides/getting-started.md) | 构建你的第一个 Agent |
| [编写工具](docs/guides/writing-tools.md) | 使用脚本创建自定义工具 |
| [扩展能力](docs/guides/extension-capabilities.md) | 热加载和运行时进化 |
| [架构决策记录](docs/adr/) | ADR 001-008（全部已接受） |
| [构建计划](BUILD_PLAN.md) | 8 阶段实现路线图 |

---

## 适用人群

- **Claw 生态系统开发者** —— 厌倦了跨项目重复编写相同的 Provider/循环/工具代码
- **Rust 开发者** —— 需要坚固、异步、跨平台的 Agent 系统基础
- **研究人员** —— 需要可脚本化、可扩展的 Agent 运行时

---

## 获取帮助

- **问题与讨论：** [GitHub Discussions](../../discussions)
- **Bug 报告和功能请求：** [GitHub Issues](../../issues)
- **安全漏洞：** [SECURITY.md](SECURITY.md)

---

## 贡献

请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。优先领域：Windows 沙箱加固 · 新 LLM Provider（Gemini、Mistral、本地 GGUF）· Lua/Deno 桥接改进 · 平台测试覆盖。

---

## 许可证

Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE)) 或 MIT License ([LICENSE-MIT](LICENSE-MIT))，由你选择。
