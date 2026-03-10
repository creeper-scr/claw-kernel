---
title: claw-channel
description: "Channel abstraction layer: ChannelMessage protocol, platform adapters for Discord, HTTP Webhook, and Stdin"
status: implemented
version: "1.4.1"
last_updated: "2026-03-10"
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

---

## ChannelEventPublisher (v1.4.0+)

`ChannelEventPublisher` 是 `claw-channel` 中定义的注入接口，用于将 channel 层事件桥接到 `EventBus`，从而不引入 `claw-channel → claw-runtime` 的循环依赖。

```rust
/// 定义在 claw-channel 内
#[async_trait]
pub trait ChannelEventPublisher: Send + Sync {
    async fn publish(&self, event: ChannelEvent) -> Result<(), ChannelError>;
}

/// 可发布的事件类型
pub enum ChannelEvent {
    MessageReceived { agent_id: String, channel: String, platform: Platform, content_preview: String },
    MessageSent     { agent_id: String, channel: String, platform: Platform, success: bool },
    ConnectionState { channel: String, platform: Platform, connected: bool },
}
```

**默认实现**：`NoopChannelEventPublisher`（丢弃所有事件，适用于不需要 EventBus 集成的场景）。

**生产实现**：`RuntimeChannelEventPublisher`（定义在 `claw-server`），将 `ChannelEvent` 转发到运行时 `EventBus`：

| `ChannelEvent` variant   | 转发为 `Event`                              |
|--------------------------|---------------------------------------------|
| `MessageReceived`        | `Event::MessageReceived`                    |
| `MessageSent`            | `Event::Custom("channel.message_sent")`     |
| `ConnectionState`        | `Event::Custom("channel.connection_state")` |

> **已知 GAP-05**：inbound channel message → EventBus → EventTrigger 的完整链路已由 `RuntimeChannelEventPublisher` 实现，但 `ChannelRegistry` 与 `AgentOrchestrator` 之间的自动路由（即收到消息后自动分配给某个 agent）尚未实现；目前需要应用层手动订阅 `EventBus` 并处理 `Event::MessageReceived`。

---

## ChannelRegistry (v1.3.0+, 定义在 claw-server)

> **注意**：`ChannelRegistry` 定义在 `claw-server`（而非 `claw-channel`），是 kernel IPC server 的组件，负责追踪已注册的 channel 实例。

```rust
// claw_server::channel_registry
pub struct ChannelRegistry {
    channels: DashMap<String, RegisteredChannel>,
    seen_ids: Arc<Mutex<HashMap<String, Instant>>>,  // 60s TTL dedup cache
    thread_sessions: DashMap<String, String>,        // thread_id → session_id
}

pub struct RegisteredChannel {
    pub channel_id: String,
    pub channel_type: String,
    pub config: serde_json::Value,
    pub registered_at: Instant,
    /// Outbound sender back to the adapter process. None for inbound-only channels.
    pub outbound_tx: Option<mpsc::Sender<Vec<u8>>>,
}
```

### 主要方法

```rust
// 注册 channel（同一 channel_id 重复注册返回 Err）
registry.register(channel_type, channel_id, config, outbound_tx)?;

// 注销
registry.unregister(&channel_id);

// 列出所有已注册 channel
let channels: Vec<RegisteredChannel> = registry.list();

// 发送数据到 adapter 进程（通过 outbound_tx）
registry.send_outbound(&channel_id, frame).await?;

// 消息去重（60s TTL）
let is_dup = registry.is_duplicate(&message_id).await;

// Thread → Session 映射（用于多轮对话会话复用）
registry.set_thread_session(thread_id, session_id);
let session_id = registry.get_thread_session(&thread_id);
```

### 设计要点

- **DashMap**：所有方法均为 lock-free 读（注册/注销除外），适合高并发 channel 查询
- **60s TTL dedup cache**：防止 webhook 重试或网络分区导致的重复消息投递
- **thread_sessions**：为 Discord Thread / Slack thread_ts 等多线程 channel 提供会话连续性
