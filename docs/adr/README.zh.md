---
title: 架构决策记录（ADR 索引）
description: claw-kernel ADR 索引
status: active
date: 2026-02-28
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](README.md)

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
| [001](001-architecture-layers.zh.md) | 五层架构与 PAL | 已接受 |
| [002](002-script-engine-selection.zh.md) | 多引擎脚本支持（Lua 默认） | 已接受 |
| [003](003-security-model.zh.md) | 双模式安全（安全/强力） | 已接受 |
| [004](004-hot-loading-mechanism.zh.md) | 工具热加载作为扩展基础设施 | 已接受 |
| [005](005-ipc-multi-agent.zh.md) | IPC 和多智能体协调 | 已接受 |
| [006](006-message-format-abstraction.zh.md) | LLM Provider 消息格式抽象 | 已接受 |
| [007](007-eventbus-implementation.zh.md) | EventBus 实现策略 | 已接受 |
| [008](008-hot-loading-file-watcher.zh.md) | 热加载文件监视器策略 | 已接受 |

## 贡献

提议新的 ADR：

1. 在 GitHub Discussions 中创建带有 `adr` 标签的讨论
2. 与维护者达成共识
3. 创建 PR 添加 ADR 文件
4. 更新此索引

更多信息请参见[贡献指南](../../CONTRIBUTING.md)。
