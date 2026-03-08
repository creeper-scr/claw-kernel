---
title: "ADR-010: Memory System Boundary"
description: "The kernel owns only short-term memory (HistoryManager); mid/long-term memory is the application's responsibility"
status: accepted
date: 2026-03-08
type: adr
---

# ADR-010: Memory System Boundary

**Status:** Accepted
**Date:** 2026-03-08
**Deciders:** claw-kernel core team

---

## Context

claw-kernel provides two distinct memory subsystems:

- **`claw-loop`**: `HistoryManager` trait — manages the in-context conversation window fed to the LLM on every call
- **`claw-memory`**: `MemoryStore` trait — persistent semantic/episodic storage (SQLite + Ngram embedder)

The question arose: should `AgentLoop` automatically wire `MemoryStore` into its lifecycle (archive conversations on overflow, inject retrieved memories before each LLM call)?

### The three memory tiers

| Tier | Description | Example |
|------|-------------|---------|
| Short-term | LLM context window, in-flight messages | `InMemoryHistory`, `SqliteHistory` |
| Mid-term | Episodic log, daily journal | Archived conversation turns |
| Long-term | Semantic memory, persistent facts | Vector search over past sessions |

### The confusion

`AgentLoop` currently has no `MemoryStore` field. Early analysis flagged this as a "missing connection". Re-examination revealed this is **intentional and correct**.

---

## Decision

**The kernel is responsible only for short-term memory.**

1. `HistoryManager` is the kernel's sole memory abstraction. It is `Box<dyn HistoryManager>` inside `AgentLoop` — injectable, replaceable, required.

2. `AgentLoop` will **not** receive a `MemoryStore` field. Mid/long-term memory lifecycle is the application's responsibility.

3. `claw-memory` is retained as an **optional reference implementation**, not a core kernel dependency. It is a convenience crate users may choose to adopt.

4. The `overflow_callback` on `HistoryManager` is the intended extension point. When context approaches the limit, the application decides what to do: archive to SQLite, call a vector DB, write a markdown file, or discard.

```
Kernel responsibility
  └── HistoryManager (context window)
        └── overflow_callback  ←── hook for application to bridge short→mid term

Application responsibility
  ├── Archive logic (claw-memory, Redis, files, API…)
  └── Memory injection (prepend retrieved context before run())
```

---

## Rationale

### Kernel philosophy

claw-kernel is infrastructure ("Linux kernel to your agent's userspace"). Infrastructure provides primitives; applications build policies on top.

- Short-term memory (context window) is universal — every LLM agent needs it. ✅ Kernel.
- Mid/long-term memory strategy varies widely — some agents need none, some use vector DBs, some use plain files. ✗ Not kernel.

### Avoiding forced coupling

If `AgentLoop` held a `MemoryStore`, every user would pay for `rusqlite` + `sqlite-vec` even if they want Redis or a remote API. Keeping the dependency optional preserves the minimal default build.

### The overflow_callback is sufficient

The hook fires when `token_estimate() >= limit`, giving the application full control:

```rust
history.set_overflow_callback(Box::new(|current, limit| {
    // Application code: archive, summarize, inject — anything
}));
```

---

## Consequences

### Easier

- Users who don't need persistence start with zero extra dependencies
- Users who do need persistence choose their own backend freely (SQLite, Redis, Qdrant, flat files…)
- `AgentLoop` remains minimal and composable
- `claw-memory` can evolve independently without breaking the loop API

### Harder

- Users must write their own overflow→archive wiring (a few lines of code)
- No built-in "memory-aware agent" out of the box — must be assembled from parts

### Reference pattern (for documentation / examples)

```rust
// Application assembles the memory-aware agent
let store = Arc::new(SqliteMemoryStore::open("./db").await?);
let store_clone = store.clone();

let mut history = InMemoryHistory::new(8192);
history.set_overflow_callback(Box::new(move |_current, _limit| {
    // archive overflow messages to store_clone
}));

let mut agent = AgentLoopBuilder::new()
    .with_provider(provider)
    .with_history(Box::new(history))
    .build()?;

// Inject relevant memories before run
let relevant = store.semantic_search(&query_embedding, 5).await?;
// prepend as system context...
agent.run(user_input).await?;
```

---

## Related

- [ADR-001](001-architecture-layers.md) — five-layer architecture
- [`claw-loop` traits](../../crates/claw-loop/src/traits.rs) — `HistoryManager` definition
- [`claw-memory` traits](../../crates/claw-memory/src/traits.rs) — `MemoryStore` definition