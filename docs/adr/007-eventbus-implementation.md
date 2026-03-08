---
title: "ADR-007: EventBus Implementation Strategy"
description: "EventBus implementation strategy for agent runtime"
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-01"
language: en
---


# ADR 007: EventBus Implementation Strategy

**Status:** Accepted  
**Date:** 2026-02-28  
**Deciders:** claw-kernel maintainers

---

## Context

Phase 2 of the build plan introduces `claw-runtime` (Layer 1), whose central component is `EventBus`. The BUILD_PLAN.md spec leaves the internal implementation deliberately open:

```rust
pub struct EventBus {
    // 内部实现  ← gap to fill
}

impl EventBus {
    pub fn emit(&self, event: Event);
    pub fn subscribe(&self, filter: EventFilter) -> Receiver<Event>;
}
```

`EventBus` must support:

- **Fan-out delivery** — multiple independent subscribers each receive every event
- **Filtered subscriptions** — callers pass an `EventFilter` to receive only relevant event variants
- **Non-blocking emit** — `emit` must never block the caller, even when a subscriber is slow
- **Lag detection** — slow subscribers should be warned and dropped rather than silently blocking the bus
- **IpcRouter integration** — cross-process events arriving via PAL IPC transport must flow into the same bus

Three Tokio primitives were evaluated:

| Primitive | Fan-out | Backpressure | Lag detection | Notes |
|-----------|:-------:|:------------:|:-------------:|-------|
| `tokio::sync::broadcast` | Native | Drop oldest | Built-in `RecvError::Lagged` | Requires `Event: Clone` |
| `tokio::sync::mpsc` | No (single consumer) | Bounded queue | Manual | Needs a dispatch loop per subscriber |
| `tokio::sync::watch` | Native | Latest-only | None | Unsuitable for event streams |

A fourth option, `crossbeam-channel`, was considered but rejected: it is synchronous and would require `spawn_blocking` wrappers throughout, adding unnecessary complexity in an async-first codebase.

---

## Decision

**Use `tokio::sync::broadcast` as the EventBus backbone, with capacity 1024.**

The `Event` enum is `Clone`-able by design (all variants carry owned data). `broadcast` delivers every message to every active receiver natively, without a manual dispatch loop. Lag detection is built in via `RecvError::Lagged(n)`, which tells a slow subscriber exactly how many messages it missed.

### Internal struct layout

```rust
use tokio::sync::broadcast;

/// In-process event bus. All events are cloned to each subscriber.
/// Capacity 1024 means up to 1024 unread events can queue before
/// the oldest is dropped and lagging subscribers are notified.
pub struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl EventBus {
    /// Create a new bus. The broadcast channel is created here;
    /// the initial Receiver is dropped immediately because
    /// subscribers are created on demand via `subscribe()`.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1024);
        Self { sender }
    }

    /// Emit an event to all active subscribers.
    /// Returns the number of receivers that received the event.
    /// Never blocks; if the channel is full, the oldest message
    /// is dropped and lagging receivers are marked.
    pub fn emit(&self, event: Event) -> usize {
        // send() returns Err only when there are zero receivers,
        // which is a normal condition (no subscribers yet).
        self.sender.send(event).unwrap_or(0)
    }

    /// Subscribe to events matching the given filter.
    /// Returns a filtered receiver wrapping a broadcast::Receiver.
    pub fn subscribe(&self, filter: EventFilter) -> FilteredReceiver {
        FilteredReceiver {
            inner: self.sender.subscribe(),
            filter,
        }
    }
}

/// A broadcast receiver that skips events not matching the filter.
pub struct FilteredReceiver {
    inner: broadcast::Receiver<Event>,
    filter: EventFilter,
}

impl FilteredReceiver {
    /// Receive the next matching event.
    /// Skips non-matching events transparently.
    /// Returns `Err(RecvError::Lagged(n))` if this subscriber fell
    /// behind by `n` messages; the caller should log a warning and
    /// decide whether to continue or unsubscribe.
    pub async fn recv(&mut self) -> Result<Event, broadcast::error::RecvError> {
        loop {
            let event = self.inner.recv().await?;
            if self.filter.matches(&event) {
                return Ok(event);
            }
        }
    }
}
```

### Capacity choice: 1024

1024 slots is large enough to absorb short bursts (a tool-call storm, rapid agent lifecycle transitions) without unbounded memory growth. At roughly 200 bytes per `Event` variant (conservative estimate), a full buffer consumes ~200 KB per bus instance. This is acceptable for a daemon process.

If a subscriber falls more than 1024 events behind, `broadcast` drops the oldest message and sets the `Lagged` error on the next `recv()`. The subscriber must handle this explicitly:

```rust
match rx.recv().await {
    Ok(event) => handle(event),
    Err(broadcast::error::RecvError::Lagged(n)) => {
        tracing::warn!("EventBus subscriber lagged by {} events, some events lost", n);
        // Continue receiving from the current position.
    }
    Err(broadcast::error::RecvError::Closed) => break,
}
```

Subscribers that consistently lag (e.g., a slow logging sink) should be moved to a dedicated background task with its own bounded `mpsc` queue, fed by a thin broadcast subscriber that never blocks.

### IpcRouter integration

`EventBus` is purely in-process. Cross-process events travel over PAL IPC transport (Unix Domain Socket on Linux/macOS, Named Pipe on Windows) and are bridged into the bus by `IpcRouter`:

```
Remote agent                    Local process
    │                               │
    │  serialized Event (bincode)   │
    ├──────────────────────────────►│
    │                               │  IpcRouter::on_incoming()
    │                               │      │
    │                               │      ▼
    │                               │  event_bus.emit(event)
    │                               │      │
    │                               │      ▼
    │                               │  all local subscribers
```

`IpcRouter` holds an `Arc<EventBus>` and calls `emit()` for every deserialized incoming event. Local events are emitted directly by their producers (agent loop, tool executor, etc.) without going through IPC at all.

Outbound routing works symmetrically: `IpcRouter` subscribes to `Event::A2A(_)` variants and forwards them to the appropriate remote agent over IPC.

```rust
pub struct IpcRouter {
    event_bus: Arc<EventBus>,
    transport: Arc<dyn IpcTransport>,  // from claw-pal
}

impl IpcRouter {
    /// Called by the PAL IPC layer when a frame arrives from a remote agent.
    pub fn on_incoming(&self, raw: &[u8]) {
        if let Ok(event) = bincode::deserialize::<Event>(raw) {
            self.event_bus.emit(event);
        }
    }

    /// Background task: forward A2A events to remote agents.
    pub async fn run_outbound(&self) {
        let mut rx = self.event_bus.subscribe(EventFilter::A2A);
        loop {
            match rx.recv().await {
                Ok(Event::A2A(msg)) => {
                    let _ = self.transport.send(msg.to, &bincode::serialize(&msg).unwrap()).await;
                }
                Ok(_) => unreachable!(),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("IpcRouter outbound lagged by {} A2A events", n);
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}
```

This keeps `EventBus` free of any IPC knowledge. The bus is a pure in-process fan-out primitive; `IpcRouter` is the cross-process bridge.

---

## Consequences

### Positive

- **No dispatch loop** — `broadcast` handles fan-out natively; no extra task needed
- **Lag is observable** — `RecvError::Lagged(n)` gives precise diagnostics; slow subscribers can't silently corrupt the bus
- **Simple API** — `emit` and `subscribe` are the only public methods; `FilteredReceiver` hides the loop-and-skip logic
- **IpcRouter is decoupled** — EventBus has no IPC dependency; it can be unit-tested without any PAL code
- **Backpressure is explicit** — capacity 1024 is a documented, tunable constant, not an unbounded queue

### Negative

- **Event must be Clone** — all `Event` variants carry owned data; cloning on every emit has a cost. For high-frequency events (e.g., streaming token output), callers should batch or use a dedicated channel outside the bus.
- **Capacity is fixed at construction** — changing capacity requires restarting the bus. This is acceptable for a daemon but worth noting.
- **Lagged subscribers lose messages** — there is no replay mechanism. Subscribers that need guaranteed delivery (e.g., audit logger) must handle `Lagged` by re-reading from a persistent store, not from the bus.

---

## Open Questions (Resolved)

| Question | Resolution |
|----------|------------|
| 1. `broadcast` vs `mpsc` for fan-out? | **`broadcast`** — native fan-out, no dispatch loop, built-in lag detection. `mpsc` would require one channel per subscriber plus a manual dispatch task. |
| 2. What capacity for the broadcast channel? | **1024** — absorbs short bursts (~200 KB max), large enough for typical agent workloads, small enough to bound memory. Tunable via `EventBusConfig` in the future. |
| 3. How does `IpcRouter` integrate without coupling EventBus to IPC? | **`IpcRouter` holds `Arc<EventBus>`** and calls `emit()` on incoming frames. EventBus has no IPC knowledge. The bridge is one-directional at the type level. |
| 4. What happens to a slow subscriber? | **`RecvError::Lagged(n)` is returned** on the next `recv()`. The subscriber logs a warning and continues from the current position. Persistently slow subscribers should use a dedicated buffered task. |
| 5. `crossbeam-channel` as an alternative? | **Rejected** — synchronous API requires `spawn_blocking` wrappers in an async-first codebase. No benefit over `broadcast` for this use case. |

---

## References

- [claw-runtime crate docs](../crates/claw-runtime.md)
- [ADR-005: IPC and Multi-Agent Coordination](005-ipc-multi-agent.md)
- [Platform Abstraction Layer](../architecture/pal.md) (IPC section)
- [Tokio broadcast docs](https://docs.rs/tokio/latest/tokio/sync/broadcast/index.html)

---
