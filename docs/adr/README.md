[English](#english) | [中文](#chinese)

<a name="english"></a>
# Architecture Decision Records (ADRs)

This directory contains Architecture Decision Records for claw-kernel.

## What is an ADR?

An Architecture Decision Record (ADR) captures an important architectural decision made along with its context and consequences. ADRs help new contributors understand why the codebase is structured the way it is.

## Format

Each ADR follows this structure:

```markdown
# ADR XXX: Title

**Status:** [Proposed | Accepted | Deprecated | Superseded by ADR-YYY]
**Date:** YYYY-MM-DD
**Deciders:** ...

## Context
What is the issue that we're seeing that is motivating this decision?

## Decision
What is the change that we're proposing or have agreed to implement?

## Consequences
What becomes easier or more difficult to do because of this change?
```

## Index

| ADR | Title | Status |
|-----|-------|--------|
| [001](001-architecture-layers.md) | Five-Layer Architecture with PAL | Accepted |
| [002](002-script-engine-selection.md) | Multi-Engine Script Support (Lua Default) | Accepted |
| [003](003-security-model.md) | Dual-Mode Security (Safe/Power) | Accepted |
| [004](004-hot-loading-mechanism.md) | Tool Hot-Loading as Extension Infrastructure | Accepted |
| [005](005-ipc-multi-agent.md) | IPC and Multi-Agent Coordination | Accepted |
| [006](006-message-format-abstraction.md) | Message Format Abstraction for LLM Providers | Accepted |

## Contributing

To propose a new ADR:

1. Open a GitHub Discussion with the `adr` label
2. Reach consensus with maintainers
3. Create a PR adding the ADR file
4. Update this index

See [Contributing Guide](../../CONTRIBUTING.md) for more.

---

<a name="chinese"></a>
# 架构决策记录 (ADRs)

本目录包含 claw-kernel 的架构决策记录。

## 什么是 ADR？

架构决策记录（ADR）记录重要的架构决策及其背景和后果。ADR 帮助新贡献者理解代码库的架构设计原因。

## 格式

每个 ADR 遵循以下结构：

```markdown
# ADR XXX: 标题

**状态：** [提议中 | 已接受 | 已弃用 | 被 ADR-YYY 取代]
**日期：** YYYY-MM-DD
**决策者：** ...

## 背景
是什么问题促使我们做出这个决策？

## 决策
我们提议或同意实施什么变更？

## 后果
因为这个变更，什么变得更容易或更困难？
```

## 索引

| ADR | 标题 | 状态 |
|-----|------|------|
| [001](001-architecture-layers.md) | 五层架构与 PAL | 已接受 |
| [002](002-script-engine-selection.md) | 多引擎脚本支持（Lua 默认） | 已接受 |
| [003](003-security-model.md) | 双模式安全（安全/强力） | 已接受 |
| [004](004-hot-loading-mechanism.md) | 工具热加载作为扩展基础设施 | 已接受 |
| [005](005-ipc-multi-agent.md) | IPC 和多智能体协调 | 已接受 |
| [006](006-message-format-abstraction.md) | LLM Provider 消息格式抽象 | 已接受 |

## 贡献

提议新的 ADR：

1. 在 GitHub Discussions 中创建带有 `adr` 标签的讨论
2. 与维护者达成共识
3. 创建 PR 添加 ADR 文件
4. 更新此索引

更多信息请参见[贡献指南](../../CONTRIBUTING.md)。
