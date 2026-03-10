---
id: ADR-014
title: "Channel Message Protocol v2 — Promote sender_id and thread_id to Top-Level Fields"
status: proposed
date: 2026-03-10
deciders: [claw-kernel maintainers]
language: en
---

# ADR-014: Channel Message Protocol v2 — Promote sender_id and thread_id to Top-Level Fields

**Status:** Proposed (planned for v1.5.0, Sprint 2)
**Date:** 2026-03-10
**Deciders:** claw-kernel maintainers

---

## Context

The current `ChannelMessage` struct (defined in `crates/claw-channel/src/types.rs`) encodes sender information exclusively inside a freeform `metadata: HashMap<String, String>` field, and has no concept of thread identity at all:

```
ChannelMessage {
    id: String,
    channel_id: String,
    content: String,
    metadata: HashMap<String, String>,  // "sender_id" buried here
    timestamp: DateTime<Utc>,
}
```

This design has two concrete failure modes identified in `docs/v1.5-gap-report.md` (GAP-04):

1. **Routing fragility** — `ChannelRouter` and downstream components that need to route or filter by sender must perform string key lookups (`metadata.get("sender_id")`) with no type safety and no compile-time guarantee that the key exists or is consistently named. Different channel adapters (Discord, Webhook, Slack) already use inconsistent key names in practice.

2. **Thread blindness** — Multi-threaded channels (Discord Threads, Slack Threads, linear conversation threads) have no place in the protocol to record a thread identity. A Discord bot receiving a message in `Thread #42` and a message in the main channel cannot distinguish them without adapter-specific hacks in metadata. This makes it impossible for `ChannelRouter` to implement correct thread-scoped fan-out.

These gaps become more critical in v1.5.0 as we close the inbound → EventBus pipeline (GAP-05) and implement `ChannelRouter::broadcast()` (GAP-02), both of which require reliable per-message routing context.

---

## Decision

Promote `sender_id` and `thread_id` to first-class, top-level `Option<String>` fields on `ChannelMessage`:

```rust
pub struct ChannelMessage {
    pub id: String,
    pub channel_id: String,
    pub content: String,
    pub sender_id: Option<String>,    // NEW — promoted from metadata
    pub thread_id: Option<String>,    // NEW — no prior representation
    pub metadata: HashMap<String, String>,
    pub timestamp: DateTime<Utc>,
}
```

Both fields are `Option<String>` because:

- Not all channel types have a meaningful sender identity (e.g., anonymous webhook POST).
- Not all messages belong to a thread (e.g., a top-level Discord channel message).

`ChannelMessageBuilder` will be updated to provide ergonomic setter methods for both fields, with `None` as the default so existing builder call sites compile without changes.

`serde(default)` will be applied to both fields so that persisted `ChannelMessage` JSON from before this change (which lacks these keys) deserializes correctly with `None` values, preserving backward compatibility for stored messages.

Any `sender_id` value previously written to `metadata` by a channel adapter will continue to be written there by that adapter for one release cycle, and the router will prefer the top-level field if present, falling back to `metadata.get("sender_id")`. This transitional behavior will be removed in v1.6.0.

---

## Consequences

### Positive

- **Type safety** — `msg.sender_id` replaces `msg.metadata.get("sender_id")` throughout the codebase. The compiler enforces the presence of the field; no silent `None` from a missing string key.
- **Thread-aware routing** — `ChannelRouter` can dispatch messages into thread-scoped queues or EventBus topics without adapter-specific logic.
- **Consistent protocol** — All channel adapters share a single, canonical location for sender and thread identity. New adapters cannot accidentally omit them without a compiler warning (they appear in the struct literal).
- **Cleaner EventBus events** — `ChannelEvent::Inbound` wrapping `ChannelMessage` automatically carries thread context, which benefits GAP-05 (inbound → EventBus pipeline) without additional changes.

### Negative / Trade-offs

- **Breaking change (semver minor bump)** — Any code that constructs `ChannelMessage` directly (rather than via `ChannelMessageBuilder`) will fail to compile until `sender_id` and `thread_id` fields are added. Callers using the builder are unaffected.
- **Adapter migration** — Each channel adapter (`DiscordChannel`, `WebhookChannel`, `StdinChannel`) must be updated to populate `sender_id` and `thread_id` from its native event type. This is mechanical but required.
- **One-release transitional fallback** — The metadata fallback for `sender_id` adds minor complexity to `ChannelRouter` for one release. It is removed cleanly in v1.6.0.

### Migration Guide

**For code that constructs `ChannelMessage` directly:**

```rust
// Before (v1.4.x)
let msg = ChannelMessage {
    id: "123".into(),
    channel_id: "ch-1".into(),
    content: "hello".into(),
    metadata: HashMap::new(),
    timestamp: Utc::now(),
};

// After (v1.5.0+)
let msg = ChannelMessage {
    id: "123".into(),
    channel_id: "ch-1".into(),
    content: "hello".into(),
    sender_id: Some("user-42".into()),
    thread_id: None,
    metadata: HashMap::new(),
    timestamp: Utc::now(),
};
```

**For code using `ChannelMessageBuilder` (no changes required):**

```rust
// Existing builder call sites continue to compile unchanged.
// New setters are available but optional:
let msg = ChannelMessage::builder()
    .id("123")
    .channel_id("ch-1")
    .content("hello")
    .sender_id("user-42")      // new, optional
    .thread_id("thread-99")    // new, optional
    .build();
```

**For deserialization of persisted messages (no changes required):**

```json
// Old stored JSON (no sender_id / thread_id keys)
{ "id": "123", "channel_id": "ch-1", "content": "hello", ... }
// Deserializes to: sender_id = None, thread_id = None  (via serde(default))
```

**For channel adapters — example Discord:**

```rust
// Before: metadata insertion
let mut metadata = HashMap::new();
metadata.insert("sender_id".to_string(), author_id.to_string());

// After: top-level field
ChannelMessage {
    sender_id: Some(author_id.to_string()),
    thread_id: msg.thread_id.map(|id| id.to_string()),
    ..
}
```

---

## Related

- [GAP-04 — v1.5-gap-report.md](../v1.5-gap-report.md)
- [ADR-006](006-message-format-abstraction.md) — Original message format abstraction
- [ROADMAP.md — v1.5.0](../../ROADMAP.md#active--v150-planned) — Sprint 2 work item
- `crates/claw-channel/src/types.rs` — Current `ChannelMessage` definition
- `crates/claw-channel/src/router.rs` — `ChannelRouter` to be updated with top-level field routing
