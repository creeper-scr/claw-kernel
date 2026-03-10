---
title: claw-channels
description: Official channel implementations — Discord (Twilight) and WebSocket (multi-client fan-out)
status: implemented
version: "1.4.1"
last_updated: "2026-03-10"
language: en
---


# claw-channels

> ⚠️ **Pre-release notice:** v0.4.0 is a beta and may be unstable. APIs are subject to change without notice.

Official channel implementations for the Claw ecosystem — Discord and WebSocket.

---

## Overview

`claw-channels` provides production-ready channel implementations that build on the `Channel` trait defined in `claw-channel`. While `claw-channel` defines the core abstractions and simpler adapters (Stdin, basic Webhook), `claw-channels` delivers the full-featured, officially maintained implementations.

### Architecture Position

```
Layer 2: claw-channels ← You are here
    ↓ depends on
Layer 2: claw-channel (Channel trait, ChannelMessage types)
```

---

## Features

```toml
[dependencies]
claw-channels = { version = "1.0", features = ["discord", "websocket"] }
```

| Feature | Description | Dependencies |
|---------|-------------|-------------|
| `discord` | Discord bot via Twilight gateway | `twilight-gateway`, `twilight-model`, `twilight-http` |
| `websocket` | WebSocket server with multi-client fan-out | `tokio-tungstenite`, `futures-util` |

> **Note:** No features are enabled by default. Enable the features you need.

---

## DiscordChannel

Full Discord bot integration using [Twilight](https://twilight.rs/) — a modular, async Discord library.

```rust
use claw_channels::DiscordChannel;
use claw_channel::{Channel, ChannelId};

let channel = DiscordChannel::new(
    ChannelId::new("discord-guild-main"),
    std::env::var("DISCORD_TOKEN")?,
);
channel.connect().await?;

// Receive messages from Discord
let msg = channel.recv().await?;
println!("From Discord: {}", msg.content);

// Send a reply
use claw_channel::{ChannelMessage, Platform};
channel.send(ChannelMessage::outbound(
    ChannelId::new("discord-guild-main"),
    Platform::Discord,
    "Hello from claw-kernel!",
)).await?;
```

### Features

- **Gateway integration**: Uses `twilight-gateway` for real-time message events
- **HTTP client**: Uses `twilight-http` for sending messages (2000-char limit enforced)
- **Message mapping**: Discord `Message` → `ChannelMessage` with metadata preservation
- **Reconnection**: Twilight's built-in reconnection and session resumption

---

## WebSocketChannel

Bidirectional WebSocket channel with **multi-client fan-out** — all connected clients receive all messages.

```rust
use claw_channels::WebSocketChannel;
use claw_channel::{Channel, ChannelId, ChannelMessage, Platform};

// Create a WebSocket channel listening on port 9001
let channel = WebSocketChannel::new(
    ChannelId::new("ws-agent"),
    "127.0.0.1:9001".parse()?,
);
channel.connect().await?;

// Wait for a message from any connected client
let msg = channel.recv().await?;
println!("WebSocket message: {}", msg.content);

// Broadcast to ALL connected clients
channel.send(ChannelMessage::outbound(
    ChannelId::new("ws-agent"),
    Platform::Webhook,  // WebSocket uses Webhook platform variant
    "Broadcast to all clients",
)).await?;
```

### Fan-out Architecture

```
Client A ──┐
Client B ──┼── WebSocketChannel ── Agent
Client C ──┘        │
                     └── All outbound messages fan-out to A, B, C
```

- **DashMap-backed client registry**: Lock-free concurrent access for client management
- **Inbound**: First message from any client is delivered to `recv()`
- **Outbound**: `send()` fans out to all currently connected clients
- **Graceful disconnect**: Disconnected clients are automatically removed from the registry

---

## Comparison: claw-channel vs claw-channels

| Feature | `claw-channel` | `claw-channels` |
|---------|---------------|----------------|
| `Channel` trait definition | ✅ | ❌ (re-exports from claw-channel) |
| `StdinChannel` | ✅ | ❌ |
| `WebhookChannel` (basic) | ✅ | ❌ |
| `DiscordChannel` (Twilight) | ✅ (moved here) | ✅ (authoritative) |
| `WebSocketChannel` (fan-out) | ❌ | ✅ |
| `ChannelRouter` | ✅ | ❌ |
| `RetryableChannel` | ✅ | ❌ |

---

## See Also

- [claw-channel](claw-channel.md) — Channel trait, ChannelMessage, ChannelRouter
- [claw-server](claw-server.md) — ChannelRegistry (IPC channel tracking)
- [Extension Capabilities Guide](../guides/extension-capabilities.md)
