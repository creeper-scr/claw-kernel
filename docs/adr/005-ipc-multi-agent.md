---
title: "ADR-005: IPC and Multi-Agent"
type: adr
status: accepted
date: "2026-02-28"
---

[English](#english) | [中文](#chinese)

<a name="english"></a>
# ADR 005: IPC and Multi-Agent Coordination

**Status:** Accepted  
**Date:** 2024-02-10  
**Deciders:** claw-kernel maintainers

---

## Context

As the ecosystem evolves, we need:
1. Multiple agents running concurrently
2. Communication between agents (A2A)
3. Parent-child agent relationships
4. Coordination without central orchestrator

---

## Decision (Proposed)

Implement a **distributed event bus** with:
- Local: In-process channels (Tokio mpsc)
- Cross-process: Platform-native IPC (UDS/Named Pipes)
- Discovery: Filesystem-based registry

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Event Bus                                │
├─────────────────────────────────────────────────────────────┤
│  Router                                                     │
│  ├── Local subscribers (same process)                       │
│  ├── Remote subscribers (IPC)                               │
│  └── Message routing logic                                  │
├─────────────────────────────────────────────────────────────┤
│  Transports                                                 │
│  ├── TokioChannel (in-process, Layer 1)                     │
│  ├── UnixSocket (Linux/macOS, Layer 0.5 PAL)                │
│  ├── NamedPipe (Windows, Layer 0.5 PAL)                     │
│  └── TcpLoopback (fallback, Layer 0.5 PAL)                  │
└─────────────────────────────────────────────────────────────┘
```

### Agent Discovery

Agents register themselves in a filesystem directory:

```
~/.local/share/claw-kernel/agents/
├── agent-main/
│   ├── info.json       # Agent metadata
│   ├── stdin.pipe      # Input pipe
│   └── stdout.pipe     # Output pipe
├── agent-searcher/
│   └── ...
└── agent-coder/
    └── ...
```

### Message Protocol

```rust
pub struct A2AMessage {
    pub from: AgentId,
    pub to: Option<AgentId>,  // None = broadcast
    pub message_type: MessageType,
    pub payload: Vec<u8>,
    pub correlation_id: Option<Uuid>,
    pub timeout: Option<Duration>,
}

pub enum A2AMessageType {
    Request,   // Expects response
    Response,  // Response to request
    Event,     // Fire-and-forget
    Command,   // Directive (parent to child)
}
```

---

## Consequences

### Positive

- **Decentralized:** No single point of failure
- **Language agnostic:** IPC works across language boundaries
- **Scalable:** Can extend to network in future

### Negative

- **Complexity:** Distributed systems are hard
- **Debugging:** Tracing messages across agents
- **Security:** A2A communication needs authentication

---

## Open Questions (Resolved)

| Question | Resolution |
|----------|------------|
| 1. Network-transparent A2A? | **Deferred to future version.** Initial implementation only supports local IPC (Unix Domain Socket / Named Pipe). Network support may be added in v2. |
| 2. Prevent agent impersonation? | **Unix socket file permissions (600)** - Only the owner can access the socket file. For Windows Named Pipes, use ACL to restrict access to the owner. No additional token-based authentication needed for local-only use case. |
| 3. Parent-child lifecycle contract? | **Parent owns child lifecycle.** When parent exits, all child agents are automatically terminated (process group behavior). Child cannot outlive parent. |

---

## References

- [claw-runtime crate docs](../crates/claw-runtime.md)
- [Platform Abstraction Layer](../architecture/pal.md) (IPC section)

---

<a name="chinese"></a>
# ADR 005: IPC 和多智能体协调

**状态：** 已接受  
**日期：** 2024-02-10  
**决策者：** claw-kernel 维护者

---

## 背景

随着生态系统发展，我们需要：
1. 多个智能体并发运行
2. 智能体间通信（A2A）
3. 父子智能体关系
4. 无需中央协调器的协调

---

## 决策（提议）

实现**分布式事件总线**，包括：
- 本地：进程内通道（Tokio mpsc）
- 跨进程：平台原生 IPC（UDS/命名管道）
- 发现：基于文件系统的注册表

### 架构

```
┌─────────────────────────────────────────────────────────────┐
│                    事件总线                                  │
├─────────────────────────────────────────────────────────────┤
│  路由器                                                      │
│  ├── 本地订阅者（同进程）                                     │
│  ├── 远程订阅者（IPC）                                        │
│  └── 消息路由逻辑                                             │
├─────────────────────────────────────────────────────────────┤
│  传输层                                                      │
│  ├── TokioChannel（进程内）                                   │
│  ├── UnixSocket（Linux/macOS）                                │
│  ├── NamedPipe（Windows）                                     │
│  └── TcpLoopback（回退）                                      │
└─────────────────────────────────────────────────────────────┘
```

### 智能体发现

智能体在文件系统目录中注册自己：

```
~/.local/share/claw-kernel/agents/
├── agent-main/
│   ├── info.json       # 智能体元数据
│   ├── stdin.pipe      # 输入管道
│   └── stdout.pipe     # 输出管道
├── agent-searcher/
│   └── ...
└── agent-coder/
    └── ...
```

### 消息协议

```rust
pub struct A2AMessage {
    pub from: AgentId,
    pub to: Option<AgentId>,  // None = 广播
    pub message_type: MessageType,
    pub payload: Vec<u8>,
    pub correlation_id: Option<Uuid>,
    pub timeout: Option<Duration>,
}

pub enum A2AMessageType {
    Request,   // 期望响应
    Response,  // 对请求的响应
    Event,     // 即发即弃
    Command,   // 指令（父到子）
}
```

---

## 后果

### 积极方面

- **去中心化：** 无单点故障
- **语言无关：** IPC 跨语言边界工作
- **可扩展：** 将来可扩展到网络

### 消极方面

- **复杂性：** 分布式系统很难
- **调试：** 跨智能体跟踪消息
- **安全性：** A2A 通信需要身份验证

---

## 待解决问题（已解决）

| 问题 | 解决方案 |
|------|----------|
| 1. 网络透明的 A2A？ | **推迟到未来版本。** 初始实现仅支持本地 IPC（Unix 域套接字 / 命名管道）。网络支持可能在 v2 中添加。 |
| 2. 防止智能体冒充？ | **Unix 套接字文件权限 (600)** - 只有所有者能访问套接字文件。对于 Windows 命名管道，使用 ACL 限制只有所有者可访问。本地使用场景不需要额外的基于 token 的身份验证。 |
| 3. 父子生命周期契约？ | **父进程拥有子进程生命周期。** 当父进程退出时，所有子智能体自动终止（进程组行为）。子进程不能比父进程活得更久。 |

---

## 参考

- [claw-runtime crate 文档](../crates/claw-runtime.md)
- [平台抽象层](../architecture/pal.md)（IPC 部分）
