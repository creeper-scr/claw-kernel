---
title: claw-channel
description: "Channel abstraction layer: ChannelMessage protocol, platform adapters for Discord, HTTP Webhook, and Stdin"
status: implemented
version: "0.1.0"
last_updated: "2026-03-09"
language: en
---


# claw-channel

Platform-agnostic channel abstraction for connecting agents to external messaging systems.

---

## Overview

`claw-channel` is the Layer 2 channel subsystem (part of Agent Kernel Protocol). It defines a unified `Channel` trait and `ChannelMessage` type that decouple agent logic from specific platform APIs.

## Components

- **`Channel` trait**: Async send/receive interface for all platforms
- **`ChannelMessage`**: Unified message envelope (text, attachments, metadata)
- **`Platform` enum**: Platform identifier (Discord, Webhook, Stdin)
- **Platform adapters**:
  - `DiscordChannel`: Discord bot integration (requires `discord` feature)
  - `WebhookChannel`: HTTP webhook push/pull (requires `webhook` feature)
  - `StdinChannel`: CLI / pipe-based interaction (always available)

## Types

```rust
/// Unified message envelope for all channel platforms.
pub struct ChannelMessage {
    pub id: String,
    pub channel_id: ChannelId,
    pub direction: MessageDirection,
    pub platform: Platform,
    /// Plain-text content.
    pub content: String,
    /// Optional structured metadata (author, guild, etc.).
    pub metadata: serde_json::Value,
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: u64,
}

/// Channel identifier (platform-specific).
pub struct ChannelId(pub String);

impl ChannelId {
    pub fn new(id: impl Into<String>) -> Self;
    pub fn as_str(&self) -> &str;
}

/// Message direction.
pub enum MessageDirection {
    Inbound,
    Outbound,
}

/// Supported platforms.
pub enum Platform {
    Discord,
    Webhook,
    Stdin,
}
```

## Channel Trait

```rust
#[async_trait]
pub trait Channel: Send + Sync {
    /// Platform name (e.g., "discord", "webhook", "stdin").
    fn platform(&self) -> &str;

    /// Unique channel identifier.
    fn channel_id(&self) -> &ChannelId;

    /// Send a message through this channel.
    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError>;

    /// Receive the next message from this channel.
    async fn recv(&self) -> Result<ChannelMessage, ChannelError>;

    /// Connect / authenticate with the external platform.
    async fn connect(&self) -> Result<(), ChannelError>;

    /// Gracefully disconnect.
    async fn disconnect(&self) -> Result<(), ChannelError>;
}
```

## Usage

```rust
use claw_channel::{Channel, ChannelId, StdinChannel, Platform};

// Create a stdin channel for CLI interaction.
// StdinChannel::new() requires a ChannelId; construct one with ChannelId::new().
let channel = StdinChannel::new(ChannelId::new("cli"));

// Receive message
let msg = channel.recv().await?;
assert_eq!(msg.platform, Platform::Stdin);
```

### Creating Messages

```rust
use claw_channel::{ChannelMessage, ChannelId, Platform};

// Create an inbound message
let msg = ChannelMessage::inbound(
    ChannelId::new("channel-1"),
    Platform::Stdin,
    "Hello, agent!"
);

// Create an outbound message
let reply = ChannelMessage::outbound(
    ChannelId::new("channel-1"),
    Platform::Stdin,
    "Hello, user!"
);
```

See [Extension Capabilities](../guides/extension-capabilities.md) for platform configuration details.

### WebhookChannel

```toml
# Cargo.toml
[dependencies]
claw-channel = { version = "1.0", features = ["webhook"] }
```

```rust
use claw_channel::{Channel, ChannelId, WebhookChannel};
use std::net::SocketAddr;

let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
let channel = WebhookChannel::new(ChannelId::new("wh-1"), addr, None);
channel.connect().await?;

let msg = channel.recv().await?;
println!("Received: {}", msg.content);

channel.disconnect().await?;
```

### DiscordChannel

```toml
# Cargo.toml
[dependencies]
claw-channel = { version = "1.0", features = ["discord"] }
```

```rust
use claw_channel::{Channel, ChannelId, DiscordChannel};

let channel = DiscordChannel::new(ChannelId::new("discord-1"), std::env::var("DISCORD_TOKEN")?);
channel.connect().await?;

let msg = channel.recv().await?;
println!("Received: {}", msg.content);

channel.disconnect().await?;
```

## Features

```toml
[features]
webhook = ["dep:axum", "dep:reqwest"]  # HTTP webhook server
discord = ["dep:twilight-gateway", "dep:twilight-model", "dep:twilight-http"]  # Discord gateway
```

> **Note:** No default features are enabled. Enable the features you need:
> - `webhook` — HTTP webhook push/pull support
> - `discord` — Discord bot integration
