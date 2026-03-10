---
title: "ADR-001: Five-Layer Architecture with PAL"
description: "Five-layer architecture with Platform Abstraction Layer (PAL) design decision"
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-01"
language: en
---


# ADR 001: Five-Layer Architecture with PAL

> 中文：五层架构与 PAL

**Status:** Accepted  
**Date:** 2024-01-15  
**Deciders:** claw-kernel core team

---

## Context

The Claw ecosystem has 8+ implementations (OpenClaw, ZeroClaw, PicoClaw, Nanobot, etc.) each independently implementing:

- LLM provider HTTP calls
- Tool-use protocol parsing
- Agent loop management
- Memory systems
- Channel integrations

This leads to:
- Wasted engineering effort
- Inconsistent behavior across implementations
- Difficulty sharing improvements

We need a shared foundation that:
1. Eliminates duplicate code
2. Supports cross-platform deployment
3. Provides extensibility for application innovations
4. Maintains high performance

---

## Decision

We will adopt a **five-layer architecture** with a dedicated **Platform Abstraction Layer (PAL)** at Layer 0.5.

```
Layer 3: Extension Foundation (Script Runtime)  ← Extension interface (Kernel boundary)
-------------------------------------------------
Layer 2: Agent Kernel Protocol                   ← Core kernel
Layer 1: System Runtime                          ← System primitives
Layer 0.5: Platform Abstraction (PAL)            ← Platform bridge
Layer 0: Rust Hard Core                          ← Foundation
```

### Architecture Boundary

**Kernel Core (Layers 0-3):**
- Minimal, stable, high-performance foundation
- Written in Rust for memory safety and zero-cost abstractions
- Provides extensibility hooks but no application logic
- Extension Foundation (Layer 3) is the outermost boundary of the kernel

> **Note:** Layers 4-5 (Application Plugins and Application Layer) are **outside the kernel**. The kernel provides infrastructure for applications to build upon, but application-specific logic and plugin systems are implemented by applications themselves.

### Key Design Choices

**1. Rust for Core (Layers 0-3)**
- Memory safety without GC
- Zero-cost abstractions
- Cross-platform compilation
- Strong async/await support via Tokio

**2. Extension Foundation as Kernel Boundary (Layer 3)**
- Hot-swappable without restart
- Multiple language options (Lua/TS)
- Provides extension interface for applications
- Applications implement self-evolution via scripts, not kernel features

**3. Dedicated PAL Layer**
- Forces platform-agnostic thinking
- Makes platform gaps visible
- Enables per-platform optimization

**4. Self-Evolution is NOT in Kernel**

Self-evolution (the ability for agents to modify their own behavior) is intentionally **outside the kernel**. The kernel provides the infrastructure (hot-loading, script runtime) that applications can use to implement self-evolution. The rationale:

- **Separation of Concerns**: Kernel provides extensibility primitives; evolution logic belongs to applications
- **Stability**: Core kernel should remain minimal and stable
- **Flexibility**: Different applications may want different self-evolution strategies
- **Safety**: Evolution code runs in script runtime with proper sandboxing, not in privileged kernel space
- **Innovation**: Application developers can experiment with evolution algorithms without kernel changes

The kernel's responsibility ends at providing a robust extension mechanism (Layer 3). How that mechanism is used—including for self-evolution—is an application concern.

---

## Consequences

### Positive

- **Code reuse:** Single implementation of provider/tool/loop primitives
- **Cross-platform:** Linux/macOS/Windows equality by design
- **Extensibility:** Scripts can be generated and hot-loaded at application layer (using kernel infrastructure)
- **Type safety:** Rust core catches errors at compile time
- **Performance:** No GC pauses, predictable latency
- **Stability:** Minimal kernel reduces attack surface and maintenance burden
- **Clear boundaries:** Kernel scope is well-defined (Layers 0-3 only)

### Negative

- **Build complexity:** Multiple engines (Lua/V8) complicate builds
- **Learning curve:** Contributors need Rust knowledge for core changes
- **Binary size:** V8 engine adds ~100MB (mitigated by feature flags)

### Neutral

- **Script debugging:** Requires tooling for Lua/TS debugging

---

## Alternatives Considered

### Alternative 1: Pure TypeScript (like OpenClaw)

**Rejected:** Single-threaded, memory-heavy (>1GB), difficult to sandbox

### Alternative 2: Pure Rust (no scripting)

**Rejected:** No extensibility capability, requires recompile for new tools

### Alternative 3: WASM instead of scripts

**Considered:** Better sandboxing, but tooling immature, harder to debug

### Alternative 4: Self-Evolution in Kernel

**Rejected:** Violates separation of concerns; kernel should be minimal and provide primitives, not implement high-level application behaviors

### Alternative 5: No PAL, platform code scattered

**Rejected:** Would lead to same fragmentation we're solving

### Alternative 6: Including Layers 4-5 in Kernel

**Rejected:** Would make the kernel too large and opinionated. Application plugins and application logic should be handled by applications built on top of the kernel, not be part of the kernel itself.

---

## References

- [Architecture Overview](../architecture/overview.md)
- [Platform Abstraction Layer](../architecture/pal.md)
- [Crate Map](../architecture/crate-map.md)

---
