[English](#english) | [中文](#chinese)

<a name="english"></a>
# claw-kernel

> The shared foundation for the Claw ecosystem — a cross-platform Agent Kernel built in Rust with embedded scripting and hot-loading capabilities.

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Platform: Linux | macOS | Windows](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey)](docs/platform/)

> 🚧 **Project Status: Design/Planning Phase**  
> This project is in early design stage. The `crates/` directory is currently empty —  
> no implementation has been started yet. See [BUILD_PLAN.md](BUILD_PLAN.md) for the roadmap.  
> 
> **Requirements**: Rust 1.83+

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

**claw-kernel** extracts these primitives into a single, well-tested, cross-platform library that any Claw-family project can build upon.

---

## What is claw-kernel?

claw-kernel is a **shared infrastructure library** — not a standalone agent. Think of it as the Linux kernel to your agent's userspace: a minimal, stable core that handles the hard systems-level work so you don't have to.

### Design Principles

- **Rust kernel, script logic** — The Rust core is stable and never hot-patched; all extensible logic lives in scripts
- **Extensible by design** — Provides hot-loading, script execution, and extension points for dynamic capabilities
- **Cross-platform first** — Linux, macOS, and Windows are equal first-class targets
- **Two execution modes** — Safe Mode (sandboxed, default) and Power Mode (full system access, explicit opt-in)
- **Minimal core, plugin ecosystem** — Inspired by Unix philosophy: do one thing well, compose for the rest

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│              Layer 3: Extension Foundation                 │
│    Lua (default) · Deno/V8 · PyO3                   │
├─────────────────────────────────────────────────────┤
│              Layer 2: Agent Kernel Protocol          │
│    Provider · ToolRegistry · AgentLoop · History     │
├─────────────────────────────────────────────────────┤
│              Layer 1: System Runtime                 │
│    Event bus · IPC Routing · Process daemon · Tokio  │
├═════════════════════════════════════════════════════╡
│              Layer 0.5: Platform Abstraction (PAL)   │
│    Sandbox · IPC Transport · Config dirs             │
├─────────────────────────────────────────────────────┤
│              Layer 0: Rust Hard Core                 │
│    Memory safety · OS abstraction · Trust root       │
└─────────────────────────────────────────────────────┘
```

→ Full architecture documentation: [docs/architecture/overview.md](docs/architecture/overview.md)

---

## Crate Ecosystem

| Crate | Description | Cross-platform |
|-------|-------------|:--------------:|
| [`claw-pal`](docs/crates/claw-pal.md) | Platform Abstraction Layer (sandbox, IPC, process) | ✅ |
| [`claw-provider`](docs/crates/claw-provider.md) | LLM provider trait + Anthropic/OpenAI/Ollama implementations | ✅ |
| [`claw-tools`](docs/crates/claw-tools.md) | Tool-use protocol, registry, schema gen, hot-loading | ✅ |
| [`claw-loop`](docs/crates/claw-loop.md) | Agent loop engine, history management, stop conditions | ✅ |
| [`claw-runtime`](docs/crates/claw-runtime.md) | Event bus, async runtime, multi-agent orchestration | ✅ |
| [`claw-script`](docs/crates/claw-script.md) | Embedded script engines (Lua default, Deno/V8, PyO3) | ✅ |

---

## Platform Support

| Platform | Status | Sandbox Backend | Notes |
|----------|:------:|-----------------|-------|
| Linux | ✅ Full | seccomp-bpf + Namespaces | Strongest isolation |
| macOS | ✅ Full | sandbox(7) profile | Apple official API |
| Windows | ✅ Full | AppContainer + Job Objects | MSVC toolchain required |

See platform-specific guides: [Linux](docs/platform/linux.md) · [macOS](docs/platform/macos.md) · [Windows](docs/platform/windows.md)

---

## Quick Start

> ⚠️ **Note**: The following shows the target API design. These crates are not yet implemented.

### Add to your project

```toml
[dependencies]
claw-provider = "0.1"
claw-tools    = "0.1"
claw-loop     = "0.1"

# Optional: full kernel with script engine
claw-kernel = { version = "0.1", features = ["engine-lua"] }
```

### Minimal agent example

```rust
use claw_provider::AnthropicProvider;
use claw_tools::ToolRegistry;
use claw_loop::AgentLoop;

#[tokio::main]
async fn main() {
    let provider = AnthropicProvider::from_env();
    let tools = ToolRegistry::new();

    let mut agent = AgentLoop::builder()
        .provider(provider)
        .tools(tools)
        .build();

    agent.run("Hello, world!").await.unwrap();
}
```

→ More examples in [examples/](examples/)

---

## Execution Modes

### Safe Mode (default)
- File system access limited to allowlisted directories
- Network access restricted by domain/port rules
- No subprocess spawning
- All script operations sandboxed

### Power Mode
Grants the agent full system access. Requires explicit opt-in:

```bash
claw-kernel --power-mode --power-key <your-key>
```

> ⚠️ Power Mode removes most restrictions. Use only when you need the agent to manage system resources, install software, or perform unrestricted automation.

→ Details: [Safe Mode Guide](docs/guides/safe-mode.md) · [Power Mode Guide](docs/guides/power-mode.md)

→ Full Power Key mechanism (including three activation methods, key requirements, downgrade constraints) see [AGENTS.md](AGENTS.md)

---

## Documentation

| Section | Description |
|---------|-------------|
| [Architecture Overview](docs/architecture/overview.md) | Full 5-layer architecture (including Platform Abstraction sublayer) |
| [Crate Map](docs/architecture/crate-map.md) | Dependency graph of all crates |
| [Getting Started](docs/guides/getting-started.md) | Build your first agent |
| [Writing Tools](docs/guides/writing-tools.md) | Create custom tools with scripts |
| [Extension Capabilities Guide](docs/guides/extension-capabilities.md) | Extension points and runtime evolution |
| [Architecture Decisions](docs/adr/) | Why we made key design choices |
| [AGENTS.md](AGENTS.md) | Power Key mechanism, audit logs, full security model |

---

## Who Is This For?

- **Claw ecosystem developers** building a new Claw-family agent and tired of rewriting the same provider/loop/tool code
- **Rust developers** who want a solid, async, cross-platform foundation for agent systems
- **Researchers** who need a scriptable, extensible agent runtime for experimentation

---

## Contributing

We welcome contributions! Please read [CONTRIBUTING.md](CONTRIBUTING.md) first.

Key areas where help is most needed:
- Windows sandbox hardening
- New LLM provider implementations
- Script engine Lua/Deno bridge improvements
- Platform-specific test coverage

---

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

---

<a name="chinese"></a>
# claw-kernel（爪核）

> Claw 生态系统的共享基础 —— 一个用 Rust 构建的跨平台 Agent 内核，支持嵌入式脚本和热加载能力。

[![CI](https://github.com/claw-project/claw-kernel/actions/workflows/ci.yml/badge.svg)](https://github.com/claw-project/claw-kernel/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Crates.io](https://img.shields.io/crates/v/claw-kernel.svg)](https://crates.io/crates/claw-kernel)
[![docs.rs](https://docs.rs/claw-kernel/badge.svg)](https://docs.rs/claw-kernel)
[![Platform: Linux | macOS | Windows](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey)](docs/platform/)

---

## 问题背景

Claw 生态系统中的每个项目都在独立重复实现相同的基础功能：

| 基础功能 | OpenClaw | ZeroClaw | PicoClaw | NanoClaw | ... |
|-----------|:--------:|:--------:|:--------:|:--------:|:---:|
| LLM 提供商 HTTP 调用 | ✓ | ✓ | ✓ | ✓ | ✓ |
| 工具使用协议 | ✓ | ✓ | ✓ | ✓ | ✓ |
| Agent 循环 | ✓ | ✓ | ✓ | ✓ | ✓ |
| 记忆系统 | ✓ | ✓ | ✓ | ✓ | ✓ |
| 渠道集成 | ✓ | ✓ | ✓ | ✓ | ✓ |

**claw-kernel** 将这些基础功能提取到一个经过充分测试的跨平台库中，任何 Claw 家族项目都可以在此基础上构建。

---

## 什么是 claw-kernel？

claw-kernel 是一个**共享基础设施库** —— 而不是一个独立的 Agent。把它想象成你 Agent 用户空间的 Linux 内核：一个最小、稳定的核心，处理艰难的系统级工作，让你无需操心。

### 设计原则

- **Rust 内核，脚本逻辑** —— Rust 核心稳定且从不热更新；所有可扩展逻辑都存在于脚本中
- **可扩展设计** —— 提供热加载、脚本执行和扩展点，支持动态能力扩展
- **跨平台优先** —— Linux、macOS 和 Windows 是平等的一等目标平台
- **两种执行模式** —— 安全模式（沙箱化，默认）和强力模式（完全系统访问，需显式选择）
- **最小核心，插件生态** —— 受 Unix 哲学启发：做好一件事，其余通过组合实现

---

## 架构概览

```
┌─────────────────────────────────────────────────────┐
│              第 3 层：扩展基础                         │
│    Lua（默认）· Deno/V8 · PyO3                       │
├─────────────────────────────────────────────────────┤
│              第 2 层：Agent 内核协议                   │
│    提供商 · 工具注册表 · Agent 循环 · 历史记录         │
├─────────────────────────────────────────────────────┤
│              第 1 层：系统运行时                       │
│    事件总线 · IPC 传输 · 进程守护 · Tokio              │
├═════════════════════════════════════════════════════╡
│              第 0.5 层：平台抽象层（PAL）              │
│    沙箱 · IPC 传输层 · 配置目录                        │
├─────────────────────────────────────────────────────┤
│              第 0 层：Rust 硬核核心                    │
│    内存安全 · 操作系统抽象 · 信任根                    │
└─────────────────────────────────────────────────────┘
```

→ 完整架构文档：[docs/architecture/overview.md](docs/architecture/overview.md)

---

## Crate 生态系统

| Crate | 描述 | 跨平台支持 |
|-------|-------------|:--------------:|
| [`claw-pal`](docs/crates/claw-pal.md) | 平台抽象层（沙箱、进程间通信、进程） | ✅ |
| [`claw-provider`](docs/crates/claw-provider.md) | LLM 提供商 trait + Anthropic/OpenAI/Ollama 实现 | ✅ |
| [`claw-tools`](docs/crates/claw-tools.md) | 工具使用协议、注册表、模式生成、热加载 | ✅ |
| [`claw-loop`](docs/crates/claw-loop.md) | Agent 循环引擎、历史管理、停止条件 | ✅ |
| [`claw-runtime`](docs/crates/claw-runtime.md) | 事件总线、异步运行时、多 Agent 编排 | ✅ |
| [`claw-script`](docs/crates/claw-script.md) | 嵌入式脚本引擎（Lua 默认、Deno/V8、PyO3） | ✅ |

---

## 平台支持

| 平台 | 状态 | 沙箱后端 | 备注 |
|----------|:------:|-----------------|-------|
| Linux | ✅ 完全支持 | seccomp-bpf + 命名空间 | 最强隔离 |
| macOS | ✅ 完全支持 | sandbox(7) 配置文件 | Apple 官方 API |
| Windows | ✅ 完全支持 | AppContainer + Job 对象 | 需要 MSVC 工具链 |

查看平台特定指南：[Linux](docs/platform/linux.md) · [macOS](docs/platform/macos.md) · [Windows](docs/platform/windows.md)

---

## 快速开始

> ⚠️ **注意**：以下代码展示的是目标 API 设计，这些 crate 尚未实现。

### 添加到你的项目

```toml
[dependencies]
claw-provider = "0.1"
claw-tools    = "0.1"
claw-loop     = "0.1"

# 可选：带脚本引擎的完整内核
claw-kernel = { version = "0.1", features = ["engine-lua"] }
```

### 最小 Agent 示例

```rust
use claw_provider::AnthropicProvider;
use claw_tools::ToolRegistry;
use claw_loop::AgentLoop;

#[tokio::main]
async fn main() {
    let provider = AnthropicProvider::from_env();
    let tools = ToolRegistry::new();

    let mut agent = AgentLoop::builder()
        .provider(provider)
        .tools(tools)
        .build();

    agent.run("Hello, world!").await.unwrap();
}
```

→ 更多示例见 [examples/](examples/)

---

## 执行模式

### 安全模式（默认）
- 文件系统访问仅限于允许列表目录
- 网络访问受域名/端口规则限制
- 禁止生成子进程
- 所有脚本操作均在沙箱中进行

### 强力模式
授予 Agent 完全系统访问权限。需要显式选择：

```bash
claw-kernel --power-mode --power-key <your-key>
```

> ⚠️ 强力模式会移除大部分限制。仅在你需要 Agent 管理系统资源、安装软件或执行无限制自动化时使用。

→ 详情：[安全模式指南](docs/guides/safe-mode.md) · [强力模式指南](docs/guides/power-mode.md)

→ 完整的 Power Key 机制请参考 [AGENTS.md](AGENTS.md)

---

## 文档

| 章节 | 描述 |
|---------|-------------|
| [架构概览](docs/architecture/overview.md) | 完整的 5 层架构（含平台抽象子层） |
| [Crate 图谱](docs/architecture/crate-map.md) | 所有 crate 的依赖图 |
| [入门指南](docs/guides/getting-started.md) | 构建你的第一个 Agent |
| [编写工具](docs/guides/writing-tools.md) | 使用脚本创建自定义工具 |
| [扩展能力指南](docs/guides/extension-capabilities.md) | 扩展点和运行时进化 |
| [架构决策](docs/adr/) | 我们做出关键设计选择的原因 |
| [AGENTS.md](AGENTS.md) | Power Key 机制、审计日志、完整安全模型 |

---

## 适用人群

- **Claw 生态系统开发者** —— 正在构建新的 Claw 家族 Agent，厌倦了重复编写相同的提供商/循环/工具代码
- **Rust 开发者** —— 想要一个坚固、异步、跨平台的 Agent 系统基础
- **研究人员** —— 需要一个可脚本化、可扩展的 Agent 运行时进行实验

---

## 贡献

我们欢迎贡献！请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。

最需要帮助的关键领域：
- Windows 沙箱加固
- 新的 LLM 提供商实现
- 脚本引擎 Lua/Deno 桥接改进
- 平台特定的测试覆盖

---

## 许可证

根据以下任一许可证授权：

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

由你选择。
