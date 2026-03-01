---
title: "ADR-005: IPC and Multi-Agent"
description: "Inter-process communication and multi-agent coordination"
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-01"
language: en
---

[中文版 →](005-ipc-multi-agent.zh.md)

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
