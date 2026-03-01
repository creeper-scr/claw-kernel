---
title: "ADR-002: Multi-Engine Script Support"
description: "Multi-engine script support design with Lua as default engine"
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-01"
language: en
---

[中文版 →](002-script-engine-selection.zh.md)

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
| **Lua (mlua)** | Default, always available | Simple tools, fast builds |
| **Deno/V8** | Optional feature | Complex agents, full JS/TS |
| **Python (PyO3)** | Optional feature | ML ecosystem integration |

### Lua as Default Rationale

```toml
[features]
default = ["engine-lua"]
engine-lua = ["mlua"]
engine-v8 = ["deno_core"]
engine-py = ["pyo3"]
```

**Why Lua:**
- Pure Rust binding (mlua), **zero system dependencies**
- **Lightweight**: runtime <500KB, compiles in <1 minute
- Sufficient for most tool logic
- Excellent C FFI if needed
- Provides a solid foundation for **application extensibility** — users can customize and extend functionality without recompiling

**Trade-off:** Less familiar than JS/Python, but simple enough to learn quickly.

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
- **Ecosystem access:** Python for ML, JS for web
- **Migration path:** Start with Lua, upgrade to V8 if needed
- **Extensibility:** Users can customize behavior via scripts without modifying core code

### Negative

- **Maintenance burden:** Three engine implementations to maintain
- **Behavior differences:** Edge cases may differ between engines
- **Documentation complexity:** Must document all three

### Mitigations

- Comprehensive test suite runs against all engines
- Bridge API is strictly typed and tested
- Users can lock to one engine in production

---

## Alternatives Considered

### Alternative 1: Deno/V8 Only

**Rejected:** 100MB+ binary, complex Windows build, slow compilation

### Alternative 2: Python Only

**Rejected:** GIL limits concurrency, sandboxing difficult

### Alternative 3: WASM (Wasmer/Wasmtime)

**Considered:** Best sandboxing, but:
- Language tooling immature (debugging, stack traces)
- Memory overhead per instance
- Complex host function binding

**Decision:** Revisit WASM for plugin isolation in future, not for main engine.

---

## Implementation Notes

### Engine Selection at Runtime

```rust
/// Engine type selector for runtime engine selection
pub enum EngineType {
    Lua,
    #[cfg(feature = "engine-v8")]
    V8,
    #[cfg(feature = "engine-py")]
    Python,
}

/// Script engine wrapper (actual engine instance)
pub enum ScriptEngine {
    Lua(LuaEngine),
    #[cfg(feature = "engine-v8")]
    V8(V8Engine),
    #[cfg(feature = "engine-py")]
    Python(PythonEngine),
}

impl ScriptEngine {
    pub fn new(engine_type: EngineType) -> Result<Self> {
        match engine_type {
            EngineType::Lua => Ok(Self::Lua(LuaEngine::new()?)),
            #[cfg(feature = "engine-v8")]
            EngineType::V8 => Ok(Self::V8(V8Engine::new()?)),
            #[cfg(feature = "engine-py")]
            EngineType::Python => Ok(Self::Python(PythonEngine::new()?)),
            _ => Err(Error::EngineNotAvailable),
        }
    }
}
```

### Per-Engine Permissions

Different engines have different sandboxing capabilities:

| Engine | Sandboxing | Permission Model |
|--------|-----------|------------------|
| Lua | Limited (code can crash host) | Runtime checks |
| Deno | Strong (V8 isolate) | Deno permissions |
| Python | Weak (GIL doesn't isolate) | OS-level only |

Recommendation: Use Safe Mode OS sandbox for all engines; Deno's built-in sandbox is additional defense.

---

## References

- [claw-script crate docs](../crates/claw-script.md)
- [mlua documentation](https://github.com/khvzak/mlua)
- [deno_core documentation](https://docs.rs/deno_core)
- [PyO3 documentation](https://pyo3.rs)

---
