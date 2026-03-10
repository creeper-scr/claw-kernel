---
id: ADR-013
title: "Runtime v1.0 — Evolution from ADR-005 IPC Design"
status: accepted
date: 2026-03-09
deciders: [claw-kernel maintainers]
language: en
---

# ADR 013: Runtime v1.0 — Evolution from ADR-005 IPC Design

**Status:** Accepted
**Date:** 2026-03-09
**Deciders:** claw-kernel maintainers

---

## Context

ADR-005 defined the initial IPC multi-agent architecture. During v1.0 implementation, several design decisions evolved from the original proposal. This ADR documents those changes and their rationale to help contributors understand why the current implementation differs from the original design.

---

## Changes from ADR-005

### 1. Message Format

| Aspect | ADR-005 Design | v1.0 Implementation |
|--------|---------------|---------------------|
| Correlation ID | UUID-based `correlation_id` field only | `message_id` (String) + optional `correlation_id` (String) |
| Payload | Raw `Vec<u8>` | Typed `A2AMessagePayload` enum (Request, Response, Event, Command, Discovery, etc.) |
| Routing | Distributed event bus | Centralized `SimpleRouter` + optional IPC transport |

**Rationale**: The centralized router is simpler to reason about for single-host deployments and avoids the complexity of a distributed event bus while still supporting multi-process scenarios via IPC. Typed payloads improve correctness at compile time and eliminate manual serialization errors.

### 2. Agent Discovery

| Aspect | ADR-005 Design | v1.0 Implementation |
|--------|---------------|---------------------|
| Discovery | Filesystem registry (`~/.local/share/claw-kernel/agents/`) | Dynamic in-memory `AgentId` registry + `DiscoveryRequest`/`DiscoveryResponse` A2A messages |

**Rationale**: In-memory discovery is sufficient for the target single-host deployment model and avoids filesystem contention under concurrent agent registration. The `DiscoveryRequest`/`DiscoveryResponse` message pair provides equivalent capability over the existing IPC channel without requiring a separate filesystem layer.

### 3. Authentication / Security

| Aspect | ADR-005 Design | v1.0 Implementation |
|--------|---------------|---------------------|
| Socket permissions | Unix socket `600` | Unix socket `700` (owner read/write/execute only) |
| Message authentication | Not specified | OS-level implicit trust (no per-message signing) |

**Rationale**: Permission upgraded from `600` to `700` to prevent group-member access. Per-message authentication was deferred to v1.1+. The current single-user deployment model is adequately protected by socket file permissions. See [SECURITY.md](../../SECURITY.md#ipc-trust-model) for the full trust model and known limitations.

### 4. Orchestrator Interface

The `AgentOrchestrator` concrete type is now backed by a `pub trait Orchestrator`, enabling dependency injection and mock testing. This was not present in ADR-005 but follows naturally from the layered architecture principles defined in ADR-001.

| Aspect | ADR-005 Design | v1.0 Implementation |
|--------|---------------|---------------------|
| Orchestrator | Not specified (implied concrete type) | `pub trait Orchestrator` + `AgentOrchestrator` as default impl |

**Rationale**: The trait boundary enables unit testing with mock orchestrators and allows future alternative implementations (e.g., priority-scheduled, distributed) without breaking the public API.

### 5. Transport Architecture

| Aspect | ADR-005 Design | v1.0 Implementation |
|--------|---------------|---------------------|
| Transports | TokioChannel, UnixSocket, NamedPipe, TcpLoopback | `IpcTransportFactory` trait + `DefaultIpcTransport` (Unix Domain Socket) |
| Windows support | Named Pipes planned | Deferred; Unix socket used on all supported platforms |

**Rationale**: The factory pattern (`IpcTransportFactory`) provides clean pluggability without requiring all transport variants to be implemented upfront. Windows Named Pipe support is deferred to a future release.

---

## Consequences

### Positive

- Simpler implementation with fewer moving parts (centralized router vs. distributed bus)
- Compile-time safety via typed `A2AMessagePayload` enum
- `Orchestrator` trait enables future orchestrator implementations and improves testability
- No filesystem dependency for agent discovery reduces operational complexity

### Negative / Known Gaps

- IPC authentication (per-message signing) is a known security gap — tracked in [ROADMAP.md](../../ROADMAP.md) and [SECURITY.md](../../SECURITY.md#ipc-trust-model)
- Windows Named Pipe transport not yet implemented
- Network-transparent A2A (TCP/TLS) deferred to v2+

---

## Related

- [ADR-001](001-architecture-layers.md) — Five-Layer Architecture with PAL
- [ADR-005](005-ipc-multi-agent.md) — Original IPC Multi-Agent Design (superseded by this ADR for implementation details)
- [ADR-011](011-multi-language-ipc-daemon.md) — Multi-Language Support via IPC Daemon
- [SECURITY.md — IPC Trust Model](../../SECURITY.md#ipc-trust-model)
- [ROADMAP.md](../../ROADMAP.md)
