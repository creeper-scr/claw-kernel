---
title: "ADR-012: V8 Engine Implementation"
description: "Implementation of JavaScript/TypeScript engine using deno_core"
status: accepted
date: 2026-03-09
type: adr
last_updated: "2026-03-09"
language: en
---

# ADR 012: V8 Engine Implementation

**Status:** Accepted  
**Date:** 2026-03-09  
**Deciders:** claw-kernel core team

---

## Context

Following ADR-002 (Multi-Engine Script Support), we need to implement the V8 JavaScript/TypeScript engine as an optional feature. This provides:

1. Full ES2022+ support for complex scripts
2. TypeScript transpilation without external tools
3. Stronger sandboxing via V8 isolates
4. Familiar language for web developers

---

## Decision

Implement V8 engine using `deno_core` crate with the following architecture:

```
┌─────────────────────────────────────────────────────────────┐
│                    V8 Engine Architecture                    │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  V8Engine                                               ││
│  │  - Per-execution isolate creation                       ││
│  │  - Configurable heap limits                             ││
│  │  - Timeout support                                      ││
│  └─────────────────────────────────────────────────────────┘│
│                        ↓                                     │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  V8 Isolate (per execution)                             ││
│  │  - Fresh context for isolation                          ││
│  │  - Memory limits enforced                               ││
│  └─────────────────────────────────────────────────────────┘│
│                        ↓                                     │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  Bridge Bindings (claw.* namespace)                     ││
│  │  - fs: File system access                               ││
│  │  - net: HTTP requests                                   ││
│  │  - tools: Tool registry                                 ││
│  │  - memory: Memory store                                 ││
│  │  - events: Event bus                                    ││
│  │  - agent: Agent orchestration                           ││
│  │  - dirs: Directory paths                                ││
│  │  - json: JSON utilities                                 ││
│  └─────────────────────────────────────────────────────────┘│
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Key Design Decisions

**1. deno_core as Foundation**

- Uses V8 JavaScript engine via Rust bindings
- Provides TypeScript transpilation out of the box
- Used by Deno project, actively maintained
- ~100MB binary size impact (acceptable for optional feature)

**2. Per-Execution Isolate Creation**

```rust
// Each execute() call creates a fresh isolate
let params = v8::Isolate::create_params()
    .heap_limits(0, options.heap_limit_mb * 1024 * 1024);
let mut isolate = v8::Isolate::new(params);
```

Rationale:
- Guarantees complete isolation between executions
- Prevents state leakage
- Simplifies resource cleanup
- Trade-off: Higher overhead per execution (~10-50ms)

**3. Unified Bridge API**

Same API as Lua engine (`claw.*` namespace):

```javascript
// JavaScript usage (same as Lua but JS syntax)
const files = claw.fs.listDir("/tmp");
const result = await claw.tools.call("summarize", { text: "..." });
claw.events.emit("task_done", { success: true });
```

**4. Feature Flag Gating**

```toml
[features]
default = ["engine-lua"]
engine-lua = ["dep:mlua"]
engine-v8 = ["dep:deno_core"]  # Optional, ~100MB
```

---

## Implementation Details

### V8EngineOptions

```rust
pub struct V8EngineOptions {
    pub timeout: Duration,           // Default: 30s
    pub heap_limit_mb: usize,        // Default: 128MB
    pub typescript: bool,            // Default: true
    pub max_recursion_depth: u32,    // Default: 32
}
```

### Usage Example

```rust
use claw_script::{ScriptEngine, V8Engine};
use claw_script::types::{Script, ScriptContext};

// Create engine
let engine = V8Engine::new();

// Or with custom options
let engine = V8Engine::with_options(V8EngineOptions {
    timeout: Duration::from_secs(60),
    heap_limit_mb: 256,
    typescript: true,
    max_recursion_depth: 64,
});

// Execute JavaScript
let script = Script::javascript("my-script", r#"
    const data = { message: "Hello from V8!", timestamp: Date.now() };
    claw.events.emit("data_ready", data);
    return data.message;
"#);

let ctx = ScriptContext::new("agent-1")
    .with_event_bus(event_bus);

let result = engine.execute(&script, &ctx).await?;
println!("Result: {}", result); // "Hello from V8!"
```

### TypeScript Support

```rust
let script = Script::typescript("my-tool", r#"
    interface Config {
        timeout: number;
        retries: number;
    }
    
    const config: Config = {
        timeout: 5000,
        retries: 3
    };
    
    // TypeScript is transpiled to JavaScript automatically
    return config;
"#);
```

---

## Consequences

### Positive

- Full JavaScript/TypeScript ecosystem available
- Strong sandboxing via V8 isolates
- TypeScript without build step
- Familiar syntax for web developers
- Same Bridge API as Lua (portable scripts)

### Negative

- Binary size: ~100MB additional (V8 + ICU data)
- Build time: Slower compilation
- Memory: Higher baseline memory usage
- Startup: ~10-50ms per execution (isolate creation)

### Mitigations

- Feature flag allows opting out
- Lua remains default for simple tools
- Per-execution isolates prevent memory leaks
- Configurable heap limits

---

## Comparison: Lua vs V8

| Aspect | Lua | V8 |
|--------|-----|-----|
| Binary size | ~500KB | ~100MB |
| Startup time | <1ms | ~10-50ms |
| Memory per execution | ~1MB | ~10-50MB |
| Language features | Minimal | Full ES2022+ |
| TypeScript | No | Yes |
| Sandboxing | Limited | Strong (isolate) |
| Ecosystem | Small | Massive (npm) |
| Use case | Simple tools | Complex agents |

---

## References

- [deno_core documentation](https://docs.rs/deno_core)
- [V8 Engine documentation](https://v8.dev/docs)
- [ADR-002: Multi-Engine Script Support](./002-script-engine-selection.md)
- [claw-script crate docs](../crates/claw-script.md)

---
