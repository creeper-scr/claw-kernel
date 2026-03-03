---
title: claw-channel
description: "Channel abstraction layer: ChannelMessage protocol, platform adapters for Discord, HTTP Webhook, and Stdin"
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](claw-channel.zh.md)

# claw-channel

Platform-agnostic channel abstraction for connecting agents to external messaging systems.

---

## Overview

`claw-channel` is the Layer 2.5 channel subsystem. It defines a unified `Channel` trait and `ChannelMessage` type that decouple agent logic from specific platform APIs.

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
    pub content: String,
    pub platform: Platform,
    pub timestamp_ms: u64,
}

/// Channel identifier (platform-specific).
pub struct ChannelId {
    pub platform: Platform,
    pub id: String,
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
    /// Receive the next message from this channel.
    async fn recv(&mut self) -> Result<ChannelMessage, ChannelError>;

    /// Send a message through this channel.
    async fn send(&mut self, message: ChannelMessage) -> Result<(), ChannelError>;

    /// Get the platform type.
    fn platform(&self) -> Platform;
}
```

## Usage

```rust
use claw_channel::{Channel, StdinChannel, Platform};

// Create a stdin channel for CLI interaction
let mut channel = StdinChannel::new();

// Receive message
let msg = channel.recv().await?;
assert_eq!(msg.platform, Platform::Stdin);
```

See [Extension Capabilities](../guides/extension-capabilities.md) for platform configuration details.

## Features

```toml
[features]
default = ["webhook"]
webhook = ["axum"]      # HTTP webhook server
discord = ["twilight"]  # Discord gateway (planned)
```
