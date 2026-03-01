---
title: claw-kernel（爪核）
description: Claw Agent 生态系统的共享 Rust 基础库
status: active
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](../README.md)

> Claw Agent 生态系统的共享 Rust 基础库 —— 跨平台、沙箱化、支持热加载。

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)
[![Rust](https://img.shields.io/badge/rust-1.83%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg)](platform/)
[![Tests](https://img.shields.io/badge/tests-389%20passing-brightgreen.svg)](#)
[![Version](https://img.shields.io/badge/version-0.1.0-blue.svg)](../CHANGELOG.md)

---

Claw 生态系统中的每个项目都在独立重复实现相同的基础功能：LLM Provider 调用、工具协议、Agent 循环、记忆系统、渠道集成。**claw-kernel** 将这些提取到一个经过充分测试的跨平台 Rust 库中。

它是**共享基础设施库**，不是独立 Agent——就像 Linux 内核之于用户空间程序。

## 架构

五层架构：Rust 硬核（信任根） → 平台抽象层（PAL）→ 系统运行时 → Agent 内核协议 → 扩展基础（Lua/V8/Python），外加 Layer 2.5 渠道层（Discord / Webhook / Stdin）。

完整图示见 [architecture/overview.zh.md](architecture/overview.zh.md)。

## 快速开始

```toml
[dependencies]
claw-kernel = { git = "https://github.com/claw-project/claw-kernel", features = ["engine-lua"] }
```

```rust
use claw_provider::AnthropicProvider;
use claw_tools::ToolRegistry;
use claw_loop::AgentLoop;

#[tokio::main]
async fn main() {
    let agent = AgentLoop::builder()
        .provider(AnthropicProvider::from_env())
        .tools(ToolRegistry::new())
        .max_turns(10)
        .build();
    agent.run("你好，世界！").await.unwrap();
}
```

更多示例见 [`examples/`](../examples/)：`simple-agent`、`custom-tool`、`self-evolving-agent`。

## Crate 列表

| Crate | 所属层 | 描述 |
|-------|--------|------|
| [`claw-pal`](crates/claw-pal.zh.md) | 0.5 | 平台抽象层：沙箱、IPC、进程管理 |
| [`claw-runtime`](crates/claw-runtime.zh.md) | 1 | EventBus（广播 1024）、AgentOrchestrator、IpcRouter |
| [`claw-provider`](crates/claw-provider.zh.md) | 2 | LLM Provider：Anthropic、OpenAI、Ollama、DeepSeek、Moonshot |
| [`claw-tools`](crates/claw-tools.zh.md) | 2 | 工具注册表、JSON Schema 生成、热加载（50ms 防抖） |
| [`claw-loop`](crates/claw-loop.zh.md) | 2 | Agent 循环引擎、历史管理、停止条件 |
| [`claw-memory`](crates/claw-memory.zh.md) | 2 | Ngram 嵌入、SQLite 存储、安全记忆（50 MB 配额） |
| [`claw-channel`](crates/claw-channel.zh.md) | 2.5 | Channel trait：Discord、HTTP Webhook、Stdin |
| [`claw-script`](crates/claw-script.zh.md) | 3 | 脚本引擎：Lua（默认）、Deno/V8、PyO3 |
| `claw-kernel` | meta | 重导出所有子 crate + prelude 模块 |

## 平台支持

| 平台 | 沙箱技术 | 隔离强度 |
|------|----------|----------|
| Linux | seccomp-bpf + Namespaces | 最强 |
| macOS | sandbox(7) / Seatbelt | 中等 |
| Windows | AppContainer + Job Objects | 中等 |

平台指南：[Linux](platform/linux.md) · [macOS](platform/macos.md) · [Windows](platform/windows.md)

## 构建

**环境要求：** Rust 1.83+；Linux 需要 `libseccomp-dev`。

```bash
git clone https://github.com/claw-project/claw-kernel.git
cd claw-kernel
cargo build                          # 默认构建（仅 Lua）
cargo test --workspace               # 运行 389 个测试
```

可选 feature：`engine-v8`（需 Node.js ≥ 20）、`engine-py`（需 Python ≥ 3.10）。完整特性矩阵见 [CONTRIBUTING.md](../CONTRIBUTING.md#feature-matrix)。

> 所有构建配置均已设置 `panic = "unwind"`（mlua 要求），`Cargo.toml` 中已配置。

## 执行模式

**安全模式（默认）：** 文件系统白名单、网络规则、禁止子进程。

**强力模式：** 完全系统访问，需显式启用（Power Key 最少 12 位）：
```bash
claw-kernel --power-mode --power-key <your-key>
```

详见 [guides/safe-mode.md](guides/safe-mode.zh.md) · [guides/power-mode.md](guides/power-mode.zh.md)

## 文档导航

| | |
|--|--|
| [架构概览](architecture/overview.md) | 五层设计与组件关系 |
| [Crate 依赖图](architecture/crate-map.zh.md) | 所有 crate 的依赖关系 |
| [入门指南](guides/getting-started.zh.md) | 构建你的第一个 Agent |
| [编写工具](guides/writing-tools.zh.md) | 用 Lua 脚本创建自定义工具 |
| [架构决策记录](adr/README.zh.md) | ADR 001–008（全部已接受） |
| [更新日志](../CHANGELOG.md) | 版本历史 |
| [路线图](../ROADMAP.md) | 未来里程碑 |

## 贡献

请先阅读 [CONTRIBUTING.md](../CONTRIBUTING.md)。优先领域：新 LLM Provider（Gemini、Mistral、本地 GGUF）· Windows 沙箱加固 · Deno/V8 引擎桥接。

- **问题与讨论：** [GitHub Discussions](https://github.com/claw-project/claw-kernel/discussions)
- **Bug 报告：** [GitHub Issues](https://github.com/claw-project/claw-kernel/issues)
- **安全漏洞：** [SECURITY.md](../SECURITY.md)

## 许可证

[Apache-2.0](../LICENSE-APACHE) 或 [MIT](../LICENSE-MIT)，由你选择。
