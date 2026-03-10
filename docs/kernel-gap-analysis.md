# claw-kernel 架构边界分析记录

> 本文档记录 v1.3.0 开发过程中对内核边界的分析与决策，作为设计历史的持久化依据。

---

## 分析背景

在 v1.2.0 的功能全集梳理过程中，团队对以下几个能力的归属（内核 vs 应用层）进行了深入讨论。

---

## 决策 D1：记忆系统边界重划（v1.3.0）

### 问题陈述

claw-kernel v1.2.0 的 F2 章节将 `MemoryStore Trait`、`hybrid_search`、`NgramEmbedder` 定义为**内核职责**，并通过 `memory.search` / `memory.store` IPC 端点暴露给客户端。

这一设计带来以下问题：

1. **依赖耦合**：内核因此依赖 SQLite + sqlite-vec，无法在嵌入式/无文件系统环境中运行
2. **策略泄漏**："记什么"、"如何检索"本质是应用策略，内核不应硬编码
3. **接口膨胀**：IPC 协议每加一个记忆操作就要增加端点，破坏"内核只提供机制"原则
4. **版本耦合**：上层应用无法自由升级记忆后端，必须随内核版本走

### 决策

**内核 F2 仅保留 `HistoryManager` Trait（短期上下文窗口管理）。**

- `memory.search` / `memory.store` IPC 端点直接删除（不保留降级 stub）
- `claw-memory` crate 保持独立，作为**可选的应用层依赖**（不是内核依赖）
- 应用可自由选择：`claw-memory`、Qdrant、Pinecone、自定义实现

### 影响范围

| 组件 | 变更 |
|------|------|
| `docs/kernel-features.md` F2 | 重写为"对话上下文管理"，仅含 HistoryManager |
| `crates/claw-server/src/protocol.rs` | 删除 `MemorySearchParams` / `MemoryStoreParams` |
| `crates/claw-server/src/handler.rs` | 删除 `memory.search` / `memory.store` 路由和 handler 函数 |
| `crates/claw-memory/` | crate 保留，降级为应用层可选组件 |
| 功能边界速查表 | "记忆"行重命名为"上下文管理" |

---

## 决策 D2：IPC 认证机制（v1.3.0）

### 问题陈述

v1.2.0 的 IPC 协议无认证，任何能访问 Unix socket 文件的进程都可以调用内核 API。在多用户系统或容器环境中存在安全风险。

### 决策

**采用连接级 token 认证（非逐帧签名）。**

设计原则：简单、安全性足够单主机场景：

1. daemon 启动时生成随机 256-bit token（hex 编码）
2. token 写入 `~/.local/share/claw-kernel/kernel.token`，权限 `0o600`
3. 客户端连接后，第一帧**必须**是握手帧：
   ```json
   {"jsonrpc":"2.0","method":"kernel.auth","params":{"token":"<token>"},"id":0}
   ```
4. 握手成功后连接进入已认证状态，后续所有方法正常处理
5. 握手失败或连接首帧不是 `kernel.auth`，立即返回 `-32001` 错误并关闭连接

### 不选择逐帧 HMAC 的原因

- Unix socket 本身是本机进程间通信，路径权限（`0o700` 目录）已是第一道防线
- 逐帧签名增加每次调用的 CPU 开销，对高频 streaming 场景不友好
- 连接级 token 在功能安全性上等价，实现复杂度低 10 倍

---

## 决策 D3：ChannelRegistry 作为内核组件（v1.3.0）

### 问题陈述

v1.2.0 的 `channel.register` IPC 端点是 stub，实际没有存储渠道信息。消息路由无法工作。

### 决策

在 `claw-server` crate 内新建 `ChannelRegistry`，作为内核运行时的核心组件：

- 使用 `DashMap` 存储已注册渠道（线程安全）
- 支持 `register / unregister / list` 操作
- 去重缓存：`message_id → Instant`（60s TTL，防重投）
- 支持类型：`"webhook" | "stdin" | "discord"` 及任意自定义类型

`KernelServer` 持有 `Arc<ChannelRegistry>`，在每个连接的 handler 中共享。

---

## 决策 D4：AgentOrchestrator 注入 KernelServer（v1.3.0）

### 问题陈述

v1.2.0 的 `agent.spawn / kill / steer / list` 端点是 stub，没有真正调用 `claw_runtime::AgentOrchestrator`。

### 决策

`KernelServer::new()` 内部创建 `Arc<AgentOrchestrator>`，通过 `handle_connection` 传入每个连接的 dispatch 函数，实现真正的多 Agent 生命周期管理。

---

*本文档随每个版本迭代更新，决策日期：2026-03-10。*

## 实施状态追踪（v1.3.0）

| 决策 | 文档 | protocol.rs | handler.rs | 测试 |
|------|------|-------------|------------|------|
| D1: 删除 memory.search/store 端点 | ✅ F2 已重写 | ✅ 已删除 | ✅ 已删除 | ⬜ |
| D2: IPC 连接级 token 认证 | ✅ kernel-gap-analysis.md | ✅ 握手帧已定义 | ✅ kernel.auth 已实现 | ⬜ |
| D3: 版本号升级至 1.3.0 | ✅ CHANGELOG | ✅ Cargo.toml | — | — |
