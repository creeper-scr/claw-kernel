# Message Flow Architecture

**Version**: 1.5.0-dev
**Status**: Implemented (GAP-05 / G-12 resolved)

This document describes the complete inbound message pipeline from an external
channel adapter to the EventBus and AgentOrchestrator.

---

## Overview

The pipeline has **two variants** depending on how the channel adapter is
integrated:

| Variant | Description | Status |
|---------|-------------|--------|
| **IPC-based (primary)** | External adapter process connects via Unix socket | ✅ Production-ready |
| **In-process (secondary)** | Native `Arc<dyn Channel>` object inside the kernel | 🔧 Trait infrastructure exists; no recv loop wired in KernelServer |

---

## Primary Path: IPC-Based External Adapter

This is the production path used by Discord, Webhook, WebSocket, and any
third-party adapter that connects to the kernel via IPC.

```
External Adapter Process          KernelServer (claw-server)
─────────────────────────         ──────────────────────────────────────────

[Platform]                        handler.rs :: handle_channel_inbound()
    │                                   │
    ▼  (HTTP/WS/Bot event)              │
[Adapter recv() loop]             1. Dedup check
    │                                channel_registry.is_duplicate(msg_id)
    ▼                                   │
[IPC frame: channel.inbound]      2. Route message
    │  {channel_id, content,           channel_router.route(&channel_msg)
    │   sender_id, thread_id, ...}      │  → agent_id_hint: String
    │                                   │
    ▼                             3. Publish to EventBus  ← G-12 fix
[KernelServer IPC listener]            event_bus.publish(
    │  handle_connection()                 Event::MessageReceived {
    │  dispatch_request()                     agent_id,
    │                                          channel,
    ▼                                          message_type: "channel_inbound",
handle_channel_inbound()               }
    │                               )
    │                                   │
    │                             4. Get/create Session
    │                                get_or_create_inbound_session()
    │                                   │
    │                             5. Run AgentLoop (background task)
    │                                session.agent_loop.run_streaming(content)
    │                                   │
    ▼                             6. Deliver reply
[channel.register notify_tx]           channel_registry.send_outbound()
    │  "channel/inbound_reply"          │
    ▼                                   ▼
[Adapter → Platform]             [EventBus subscribers]
                                  (EventTriggers, AgentOrchestrator monitors)
```

### Key files

| File | Role |
|------|------|
| `crates/claw-server/src/handler.rs:2377-2480` | `handle_channel_inbound` — full pipeline |
| `crates/claw-channel/src/router.rs` | `ChannelRouter::route()` — rule matching |
| `crates/claw-runtime/src/event_bus.rs` | `EventBus::publish()` — broadcast |
| `crates/claw-server/src/channel_registry.rs` | Dedup cache + outbound sender map |

### Adapter setup sequence

Before sending `channel.inbound`, the adapter must:

1. Authenticate via `kernel.auth` (IPC token)
2. Register itself: `channel.register { channel_id, type, config }`
3. Add a routing rule: `channel.route_add { channel_id, agent_id }`
4. Send messages: `channel.inbound { channel_id, content, sender_id, ... }`

---

## Secondary Path: In-Process Channel

Used only when a `Channel` trait implementor (e.g. `StdinChannel`) is
instantiated directly inside the kernel process.

```
In-process Channel object         KernelServer
─────────────────────────         ────────────────────────────────────────

[Arc<dyn Channel>]
    │  .recv() → ChannelMessage
    │
    │  ⚠ NO recv loop in KernelServer ⚠
    │  (KernelServer stores no Arc<dyn Channel> objects)
    │
    │  If a recv loop were present:
    ▼
channel_into_stream(channel)      ChannelRouter::route(&msg)
    │  stream of ChannelMessage         │  → agent_id: String
    │                                   │
    ▼                             EventBus::publish(Event::MessageReceived)
[channel.event_publisher]
    │  RuntimeChannelEventPublisher     │
    │  (claw-server/src/               │
    │   channel_event_publisher.rs)    ▼
    │                             AgentOrchestrator / EventTrigger
    ▼
EventBus::publish(Event::MessageReceived)
```

**Current state**: `RuntimeChannelEventPublisher` is implemented and tested in
`crates/claw-server/src/channel_event_publisher.rs`, but is **not wired** into
any production code path because `KernelServer` does not store in-process
`Channel` objects. Channels are exclusively registered as external adapters via
IPC (`channel.register`).

If in-process channel support is needed in the future, the pattern would be:

```rust
// Attach the publisher when creating the channel:
let publisher = RuntimeChannelEventPublisher::new(event_bus.clone());
let channel = StdinChannel::new().with_event_publisher("agent-id", publisher);

// Start a recv loop:
let ch = Arc::new(channel) as Arc<dyn Channel>;
tokio::spawn(async move {
    use futures_util::StreamExt;
    let mut stream = claw_channel::channel_into_stream(ch);
    while let Some(msg) = stream.next().await {
        let agent_id = channel_router.route(&msg);
        // EventBus publish happens inside recv() via event_publisher
    }
});
```

---

## EventBus Events Published

| Event | Trigger |
|-------|---------|
| `Event::MessageReceived { agent_id, channel, message_type }` | Every `channel.inbound` IPC call (primary path) |
| `Event::MessageReceived { agent_id, channel, message_type }` | Every `Channel::recv()` with `event_publisher` attached (secondary path) |
| `Event::Custom("channel.message_sent")` | When `RuntimeChannelEventPublisher` publishes a `MessageSent` event |
| `Event::Custom("channel.connection_state")` | When `RuntimeChannelEventPublisher` publishes a `ConnectionState` event |

---

## Deduplication

The `ChannelRegistry` maintains a 60-second TTL dedup cache keyed on
`message_id` (if provided by the adapter). Duplicate messages are silently
dropped with `{ "status": "duplicate", "skipped": true }`.

---

## Thread / Session Affinity

`get_or_create_inbound_session()` uses `thread_id` (if present) to reuse an
existing `Session` for conversational continuity. Sessions are stored in
`SessionManager` and associated with thread IDs via
`channel_registry.set_thread_session()`.

---

## Gap History

| Gap | Description | Resolution |
|-----|-------------|------------|
| GAP-05 | `channel.inbound` did not publish to EventBus | Fixed in v1.5.0-dev: `handle_channel_inbound` now calls `event_bus.publish(Event::MessageReceived)` at line 2417 |
| G-12 | Channel→EventBus pipeline unconfirmed | Verified present (primary path); secondary path infrastructure exists but no production wiring needed |
