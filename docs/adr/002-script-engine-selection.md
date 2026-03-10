---
title: "ADR-002: Multi-Engine Script Support"
description: "Multi-engine script support design with Lua as default engine"
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-09"
language: en
---


# ADR 002: Multi-Engine Script Support (Lua Default)

**Status:** Accepted  
**Date:** 2024-01-20  
**Deciders:** claw-kernel core team

---

## Context

We need a scripting layer that:
1. Supports **extensibility** and **hot-loading** for user customization
2. Is cross-platform
3. Has minimal dependencies for quick builds
4. Can leverage existing ecosystems (ML, web)

No single engine satisfies all requirements.

---

## Decision

Support **multiple script engines** with **Lua as default**:

| Engine | Status | Use Case |
|--------|--------|----------|
| **Lua (mlua)** | ✅ Default, always available | Simple tools, fast builds |
| **Deno/V8** | ✅ Optional feature | Complex agents, full JS/TS |

### Lua as Default Rationale

```toml
[features]
default = ["engine-lua"]
engine-lua = ["mlua"]          # ✅ Implemented
engine-v8 = ["deno_core"]      # ✅ Implemented
```

**Why Lua:**
- Pure Rust binding (mlua), **zero system dependencies**
- **Lightweight**: runtime <500KB, compiles in <1 minute
- Sufficient for most tool logic
- Excellent C FFI if needed
- Provides a solid foundation for **application extensibility** — users can customize and extend functionality without recompiling

**Why V8:**
- Full ES2022+ and TypeScript support
- Strong sandboxing via V8 isolates
- Familiar language for web developers
- Same Bridge API as Lua (portable scripts)

**Trade-offs:**
- Lua: Less familiar than JS, but simple
- V8: ~100MB binary size, slower startup

### Unified Bridge API

All engines expose the same `RustBridge` interface:

```typescript
// Same API regardless of engine (simplified view - see claw-script.md for full definition)
interface RustBridge {
  llm: { complete(messages: Message[]): Promise<Response> };
  tools: { register(def: ToolDef): void; call(name: string, params: any): Promise<any>; list(): ToolMeta[] };
  memory: { get(key: string): Promise<any>; set(key: string, value: any): Promise<void>; search(query: string, topK: number): Promise<MemoryItem[]> };
  events: { emit(event: string, data: any): void; on(event: string, handler: Function): void };
  fs: { read(path: string): Promise<Buffer>; write(path: string, data: Buffer): Promise<void> };
}
```

---

## Consequences

### Positive

- **Fast default builds:** Lua only, no heavy deps
- **Flexibility:** Users choose engine by feature flag
- **Ecosystem access:** JS for web
- **Migration path:** Start with Lua, upgrade to V8 if needed
- **Extensibility:** Users can customize behavior via scripts without modifying core code

### Negative

- **Maintenance burden:** Multiple engine implementations to maintain
- **Behavior differences:** Edge cases may differ between engines
- **Documentation complexity:** Must document all engines

### Mitigations

- Comprehensive test suite runs against all engines
- Bridge API is strictly typed and tested
- Users can lock to one engine in production

---

## Alternatives Considered

### Alternative 1: Deno/V8 Only

**Rejected as default:** 100MB+ binary, complex Windows build, slow compilation

**Accepted as optional:** Available via `engine-v8` feature flag

### Alternative 2: Python Only

**Rejected:** GIL limits concurrency, sandboxing difficult

### Alternative 3: WASM (Wasmer/Wasmtime)

**Considered:** Best sandboxing, but:
- Language tooling immature (debugging, stack traces)
- Memory overhead per instance
- Complex host function binding

**Decision:** Revisit WASM for plugin isolation in future, not for main engine.

---

## Implementation Status

| Engine | Status | Version |
|--------|--------|---------|
| Lua | ✅ Implemented | v0.1.0 |
| V8/TypeScript | ✅ Implemented | v0.1.0 |

## Implementation Notes

### Engine Selection at Runtime

```rust
use claw_script::{LuaEngine, V8Engine, ScriptEngine};

// Lua engine (always available)
let lua = LuaEngine::new();

// V8 engine (requires "engine-v8" feature)
let v8 = V8Engine::new();

// Or with custom options
let v8 = V8Engine::with_options(V8EngineOptions {
    timeout: Duration::from_secs(60),
    heap_limit_mb: 256,
    typescript: true,
    max_recursion_depth: 64,
});
```

### Per-Engine Permissions

Different engines have different sandboxing capabilities:

| Engine | Sandboxing | Permission Model |
|--------|-----------|------------------|
| Lua | Limited (code can crash host) | Runtime checks |
| Deno | Strong (V8 isolate) | Deno permissions |

Recommendation: Use Safe Mode OS sandbox for all engines; Deno's built-in sandbox is additional defense.

---

## References

- [claw-script crate docs](../crates/claw-script.md)
- [mlua documentation](https://github.com/khvzak/mlua)
- [deno_core documentation](https://docs.rs/deno_core)

---
