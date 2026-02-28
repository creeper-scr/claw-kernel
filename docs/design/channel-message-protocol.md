---
title: "Channel Message Protocol"
type: design
status: accepted
date: "2026-02-28"
crate: claw-channel
phase: 6
---

[English](#english) | [中文](#chinese)

<a name="english"></a>
# Channel Message Protocol

**Status:** Accepted  
**Date:** 2026-02-28  
**Crate:** `claw-channel` (Phase 6)

---

## 1. Overview

`claw-channel` provides a unified abstraction over external communication channels: Telegram bots, Discord bots, and HTTP webhooks. The `Channel` trait (defined in Phase 6) sends and receives `ChannelMessage` values. This document defines the complete `ChannelMessage` type, its serialization format, error handling, rate limiting, and retry behavior, plus the mapping rules for each supported platform.

The design follows the same principles as `claw-provider` (ADR-006): a thin, platform-neutral envelope that carries all the information a channel adapter needs, without leaking platform-specific details into the agent loop.

### Scope

This document covers:

- The `ChannelMessage` Rust type and all supporting types
- JSON serialization format and field naming conventions
- `ChannelError` variants and their semantics
- Rate limiting strategy (token bucket, per channel)
- Retry strategy (exponential backoff with jitter)
- Platform mapping for Telegram, Discord, and HTTP Webhook
- Extension guide for adding new channels

---

## 2. ChannelMessage Type Definition

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque identifier for a single message.
/// Wraps a UUID v4 to ensure global uniqueness across channels.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub Uuid);

impl MessageId {
    /// Generate a new random message ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Opaque identifier for a channel instance.
/// Typically set by the application when constructing a channel adapter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub String);

/// The top-level message envelope passed to and from the `Channel` trait.
///
/// Every message flowing through claw-channel — inbound from a user or
/// outbound from the agent — is wrapped in this struct. Platform-specific
/// details live in `metadata.raw` and are never required for routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    /// Globally unique message identifier (UUID v4).
    pub id: MessageId,

    /// Which channel instance this message belongs to.
    pub channel_id: ChannelId,

    /// Whether this message is arriving from a user (Inbound) or
    /// being sent by the agent (Outbound).
    pub direction: Direction,

    /// When the message was created or received, in UTC.
    /// Serialized as ISO 8601 (e.g. "2026-02-28T12:00:00Z").
    pub timestamp: DateTime<Utc>,

    /// The actual payload of the message.
    pub content: MessageContent,

    /// Platform-specific metadata (sender, chat, thread, raw payload).
    pub metadata: ChannelMetadata,

    /// If this message is a reply, the ID of the message being replied to.
    pub reply_to: Option<MessageId>,
}

/// The direction of a message relative to the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// A message arriving from a user or external system.
    Inbound,
    /// A message the agent is sending to a user or external system.
    Outbound,
}

/// The payload of a channel message.
///
/// Variants cover the common content types across all supported platforms.
/// Use `Structured` for webhook payloads or any platform-specific data
/// that doesn't fit the other variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    /// Plain UTF-8 text. No markup is applied.
    Text(String),

    /// Markdown-formatted text.
    /// The exact dialect (CommonMark, Telegram MarkdownV2, Discord markdown)
    /// is determined by the channel adapter when sending.
    Markdown(String),

    /// A file attachment.
    File {
        /// Original filename, including extension.
        name: String,
        /// MIME type (e.g. "application/pdf", "text/csv").
        mime_type: String,
        /// Raw file bytes.
        data: Vec<u8>,
    },

    /// An image.
    Image {
        /// Raw image bytes.
        data: Vec<u8>,
        /// MIME type (e.g. "image/png", "image/jpeg").
        mime_type: String,
        /// Optional description for accessibility or alt text.
        alt_text: Option<String>,
    },

    /// An audio clip.
    Audio {
        /// Raw audio bytes.
        data: Vec<u8>,
        /// MIME type (e.g. "audio/ogg", "audio/mpeg").
        mime_type: String,
        /// Duration in seconds. Use 0.0 if unknown.
        duration_secs: f32,
    },

    /// Arbitrary JSON, used for HTTP webhook payloads or
    /// platform-specific structured data.
    Structured(serde_json::Value),
}

/// Platform-specific metadata attached to every message.
///
/// Fields are optional because not every platform provides all of them.
/// The `raw` field preserves the original platform payload for adapters
/// that need to access fields not covered by the standard envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMetadata {
    /// Which platform this message came from or is going to.
    pub platform: Platform,

    /// The platform's identifier for the sender (user ID, bot ID, etc.).
    /// `None` for outbound messages or anonymous webhook calls.
    pub sender_id: Option<String>,

    /// The platform's identifier for the conversation (chat ID, channel ID, etc.).
    pub chat_id: Option<String>,

    /// The platform's identifier for a thread or topic within a chat.
    /// `None` on platforms that don't support threading.
    pub thread_id: Option<String>,

    /// The original platform payload, preserved verbatim.
    /// Useful for accessing platform-specific fields not covered above.
    pub raw: Option<serde_json::Value>,
}

/// The external platform a channel adapter connects to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Telegram,
    Discord,
    Webhook,
    /// Any platform not listed above. The string is a short identifier
    /// chosen by the adapter author (e.g. "slack", "matrix").
    Custom(String),
}
```

### Design Notes

- `MessageId` uses UUID v4 to avoid coordination between channel adapters.
- `MessageContent` uses `#[serde(tag = "type")]` so the JSON representation is self-describing (e.g. `{"type": "text", ...}`).
- Binary fields (`data: Vec<u8>`) serialize as base64 strings in JSON. This is handled automatically by serde with the `serde_with` crate's `base64` helper, or by a custom serializer.
- `ChannelMetadata.raw` is intentionally `Option<serde_json::Value>` rather than a typed struct. Platform APIs change frequently; keeping the raw payload avoids breaking changes.

---

## 3. Serialization Format

All `ChannelMessage` values serialize to JSON. The rules are:

| Rule | Value |
|------|-------|
| Field naming | `snake_case` (enforced by `#[serde(rename_all = "snake_case")]`) |
| Timestamp format | ISO 8601 with UTC offset: `"2026-02-28T12:00:00Z"` |
| Enum variants | `snake_case` strings (e.g. `"inbound"`, `"outbound"`) |
| Tagged enums | `type` field as discriminant (e.g. `{"type": "text", ...}`) |
| Binary data | Base64-encoded string (standard alphabet, no line breaks) |
| Null fields | Omitted when `None` (use `#[serde(skip_serializing_if = "Option::is_none")]`) |
| Unknown fields | Ignored on deserialization (use `#[serde(deny_unknown_fields)]` only in tests) |

### Example: Inbound Text Message

```json
{
  "id": "01234567-89ab-cdef-0123-456789abcdef",
  "channel_id": "telegram-main",
  "direction": "inbound",
  "timestamp": "2026-02-28T12:00:00Z",
  "content": {
    "type": "text",
    "value": "Hello, agent!"
  },
  "metadata": {
    "platform": "telegram",
    "sender_id": "123456789",
    "chat_id": "-1001234567890",
    "raw": {
      "update_id": 100000001,
      "message": { "message_id": 42, "text": "Hello, agent!" }
    }
  }
}
```

### Example: Outbound Markdown Message

```json
{
  "id": "fedcba98-7654-3210-fedc-ba9876543210",
  "channel_id": "discord-support",
  "direction": "outbound",
  "timestamp": "2026-02-28T12:00:05Z",
  "content": {
    "type": "markdown",
    "value": "**Result:** `cargo test` passed with 42 tests."
  },
  "metadata": {
    "platform": "discord",
    "chat_id": "987654321098765432"
  },
  "reply_to": "01234567-89ab-cdef-0123-456789abcdef"
}
```

---

## 4. ChannelError Definition

```rust
use std::time::Duration;
use thiserror::Error;

/// All errors that can occur when interacting with a channel.
#[derive(Debug, Error)]
pub enum ChannelError {
    /// The platform's rate limit was hit. The adapter should wait
    /// `retry_after` before retrying.
    #[error("rate limited: retry after {retry_after:?}")]
    RateLimited {
        /// How long to wait before the next attempt.
        retry_after: Duration,
    },

    /// The bot token or webhook secret was rejected by the platform.
    /// Retrying without fixing credentials will not help.
    #[error("authentication failed")]
    AuthenticationFailed,

    /// The message payload exceeds the platform's size limit.
    #[error("message too large: max {max_bytes} bytes, got {actual_bytes} bytes")]
    MessageTooLarge {
        /// The platform's maximum allowed size in bytes.
        max_bytes: usize,
        /// The actual size of the rejected payload in bytes.
        actual_bytes: usize,
    },

    /// A network-level error (DNS failure, connection refused, TLS error, timeout).
    /// The inner string is the underlying error message.
    #[error("network error: {0}")]
    NetworkError(String),

    /// The platform returned an error response with a numeric code.
    /// Consult the platform's API documentation for the meaning of `code`.
    #[error("platform error {code}: {message}")]
    PlatformError {
        /// Platform-specific error code (e.g. Telegram error code, Discord error code).
        code: i32,
        /// Human-readable error message from the platform.
        message: String,
    },

    /// The channel was not started before `send` or `on_message` was called.
    #[error("channel not started")]
    NotStarted,

    /// The channel has already been stopped and cannot be reused.
    #[error("channel stopped")]
    Stopped,
}

impl ChannelError {
    /// Returns `true` if this error is transient and the operation can be retried.
    ///
    /// `AuthenticationFailed`, `MessageTooLarge`, `NotStarted`, and `Stopped`
    /// are permanent errors. All others are considered transient.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ChannelError::RateLimited { .. }
                | ChannelError::NetworkError(_)
                | ChannelError::PlatformError { .. }
        )
    }

    /// If the platform told us exactly how long to wait, return that duration.
    /// Otherwise return `None` and let the retry strategy decide.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            ChannelError::RateLimited { retry_after } => Some(*retry_after),
            _ => None,
        }
    }
}
```

### Error Handling by Channel

| Error Variant | Telegram | Discord | HTTP Webhook |
|---------------|----------|---------|--------------|
| `RateLimited` | HTTP 429 with `retry_after` field | HTTP 429 with `X-RateLimit-Reset-After` header | HTTP 429 (if server sends it) |
| `AuthenticationFailed` | `{"ok": false, "error_code": 401}` | HTTP 401 | HTTP 401 or 403 |
| `MessageTooLarge` | Text > 4096 chars or file > 50 MB | Text > 2000 chars or file > 25 MB | Configurable (default: no limit) |
| `NetworkError` | reqwest error | reqwest error | reqwest error |
| `PlatformError` | `{"ok": false, "error_code": N}` | `{"code": N, "message": "..."}` | Any non-2xx HTTP status |

---

## 5. Rate Limiting Strategy

Each channel instance maintains an independent token bucket. The bucket is not shared across channel instances, even if they connect to the same platform.

### Token Bucket Parameters

| Parameter | Default | Notes |
|-----------|---------|-------|
| Capacity | 30 tokens | Maximum burst size |
| Refill rate | 30 tokens/minute (0.5/sec) | Matches Telegram's documented bot limit |
| Initial tokens | 30 | Bucket starts full |
| Scope | Per `ChannelId` | Each channel has its own bucket |

### Behavior

1. Before calling the platform API, the adapter tries to consume one token.
2. If a token is available, the call proceeds immediately.
3. If no token is available, the adapter waits until one refills (up to 2 seconds), then retries.
4. If the platform returns `HTTP 429`, the adapter respects the `retry_after` value from the response, regardless of the local bucket state, and refills the bucket to 0 to force a pause.

### Configuration

```rust
/// Rate limiter configuration for a channel instance.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum number of messages that can be sent in a burst.
    pub capacity: u32,
    /// How many tokens are added per second.
    pub refill_per_second: f64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            capacity: 30,
            refill_per_second: 0.5, // 30 per minute
        }
    }
}
```

Platform-specific defaults (override `RateLimitConfig` when constructing the adapter):

| Platform | Capacity | Refill rate | Source |
|----------|----------|-------------|--------|
| Telegram | 30 | 0.5/sec | Telegram Bot API docs (30 msg/sec group limit) |
| Discord | 5 | 0.2/sec | Discord rate limit: 5 messages per 5 seconds per channel |
| HTTP Webhook | 100 | 10/sec | Conservative default; override per deployment |

---

## 6. Retry Strategy

The retry strategy applies to all `ChannelError` variants where `is_retryable()` returns `true`.

### Parameters

| Parameter | Value |
|-----------|-------|
| Initial delay | 1 second |
| Multiplier | 2x per attempt |
| Maximum delay | 60 seconds |
| Jitter | ±25% of computed delay (uniform random) |
| Maximum attempts | 3 |
| Non-retryable errors | Fail immediately, no retry |

### Delay Schedule (without jitter)

| Attempt | Delay before attempt |
|---------|---------------------|
| 1 (first try) | 0 ms (immediate) |
| 2 | 1 second |
| 3 | 2 seconds |
| 4+ | Not attempted |

If the error is `RateLimited` with a `retry_after` value, that value replaces the computed delay for that attempt (but jitter is still applied on top).

### Implementation Sketch

```rust
pub async fn send_with_retry(
    channel: &dyn Channel,
    message: ChannelMessage,
    config: &RetryConfig,
) -> Result<(), ChannelError> {
    let mut delay = config.initial_delay;
    for attempt in 0..config.max_attempts {
        match channel.send(message.clone()).await {
            Ok(()) => return Ok(()),
            Err(err) if !err.is_retryable() => return Err(err),
            Err(err) => {
                if attempt + 1 == config.max_attempts {
                    return Err(err);
                }
                // Respect platform-provided retry_after if present.
                let base = err.retry_after().unwrap_or(delay);
                let jitter = base.mul_f64(rand::random::<f64>() * 0.5 - 0.25);
                tokio::time::sleep(base + jitter).await;
                delay = (delay * 2).min(config.max_delay);
            }
        }
    }
    unreachable!()
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub max_attempts: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            max_attempts: 3,
        }
    }
}
```

---

## 7. Platform Mapping: Telegram

### Constraints

| Constraint | Value |
|------------|-------|
| Max text length | 4096 characters per message |
| Max file size | 50 MB (via Bot API) |
| Markup support | MarkdownV2, HTML, plain text |
| Threading | Reply chains only (no named threads) |
| Bot API version | 7.x |

### Inbound: Telegram Update → ChannelMessage

When the Telegram adapter receives an `Update` object from the Bot API:

| Telegram field | ChannelMessage field | Notes |
|----------------|---------------------|-------|
| `update_id` | `metadata.raw` | Stored verbatim |
| `message.message_id` | `metadata.raw` | Stored verbatim |
| `message.from.id` | `metadata.sender_id` | Stringified integer |
| `message.chat.id` | `metadata.chat_id` | Stringified integer |
| `message.reply_to_message.message_id` | `reply_to` | Mapped to a `MessageId` if tracked |
| `message.text` | `content: Text(...)` | Plain text messages |
| `message.text` (with entities) | `content: Markdown(...)` | When `parse_mode` entities are present |
| `message.photo` (largest size) | `content: Image { ... }` | Adapter downloads the file |
| `message.document` | `content: File { ... }` | Adapter downloads the file |
| `message.voice` / `message.audio` | `content: Audio { ... }` | Adapter downloads the file |
| `message.date` | `timestamp` | Unix timestamp converted to `DateTime<Utc>` |

The full `Update` JSON is stored in `metadata.raw`.

### Outbound: ChannelMessage → Telegram sendMessage

| ChannelMessage field | Telegram API call | Notes |
|---------------------|-------------------|-------|
| `content: Text(s)` | `sendMessage` with `text=s` | No `parse_mode` |
| `content: Markdown(s)` | `sendMessage` with `text=s, parse_mode=MarkdownV2` | Adapter escapes special chars |
| `content: Image { data, .. }` | `sendPhoto` with `photo=<bytes>` | |
| `content: File { data, name, .. }` | `sendDocument` with `document=<bytes>` | |
| `content: Audio { data, .. }` | `sendAudio` or `sendVoice` | `sendVoice` for ogg/opus, `sendAudio` otherwise |
| `content: Structured(v)` | `sendMessage` with `text=serde_json::to_string(v)` | Fallback; prefer explicit variants |
| `reply_to` | `reply_parameters.message_id` | Requires the original Telegram message ID from `metadata.raw` |
| `metadata.chat_id` | `chat_id` parameter | Required |

**Text splitting:** If `Text` or `Markdown` content exceeds 4096 characters, the adapter splits it into multiple sequential `sendMessage` calls. Each part is sent with the same `chat_id`. Only the first part carries `reply_parameters`.

---

## 8. Platform Mapping: Discord

### Constraints

| Constraint | Value |
|------------|-------|
| Max text length | 2000 characters per message |
| Max file size | 25 MB (standard), 500 MB (Nitro) |
| Markup support | Discord markdown (subset of CommonMark) |
| Threading | Forum channels, thread channels, reply chains |
| API version | Discord API v10 |

### Inbound: Discord Message Event → ChannelMessage

When the Discord adapter receives a `MESSAGE_CREATE` gateway event:

| Discord field | ChannelMessage field | Notes |
|---------------|---------------------|-------|
| `id` | `metadata.raw` | Stored verbatim (snowflake string) |
| `author.id` | `metadata.sender_id` | Snowflake string |
| `channel_id` | `metadata.chat_id` | Snowflake string |
| `thread.id` (if present) | `metadata.thread_id` | Snowflake string |
| `message_reference.message_id` | `reply_to` | Mapped to `MessageId` if tracked |
| `content` | `content: Text(...)` or `content: Markdown(...)` | `Markdown` when content contains Discord markdown syntax |
| `attachments[0]` (image) | `content: Image { ... }` | Adapter downloads from `url` |
| `attachments[0]` (other) | `content: File { ... }` | Adapter downloads from `url` |
| `timestamp` | `timestamp` | ISO 8601 string, parsed to `DateTime<Utc>` |
| Full event JSON | `metadata.raw` | Stored verbatim |

When a message has both text and attachments, the adapter creates one `ChannelMessage` per attachment, plus one for the text if non-empty.

### Outbound: ChannelMessage → Discord Create Message

| ChannelMessage field | Discord API field | Notes |
|---------------------|-------------------|-------|
| `content: Text(s)` | `content: s` | |
| `content: Markdown(s)` | `content: s` | Discord renders markdown natively |
| `content: Image { data, .. }` | `files[0]` multipart | |
| `content: File { data, name, .. }` | `files[0]` multipart | |
| `content: Audio { data, .. }` | `files[0]` multipart | |
| `content: Structured(v)` | `embeds[0]` | Adapter converts JSON to a Discord embed |
| `reply_to` | `message_reference.message_id` | Requires original Discord snowflake from `metadata.raw` |
| `metadata.chat_id` | URL path parameter | Required |
| `metadata.thread_id` | `thread_id` query parameter | Optional |

**Text splitting:** If `Text` or `Markdown` content exceeds 2000 characters, the adapter splits it into multiple sequential messages. Only the first carries `message_reference`.

---

## 9. Platform Mapping: HTTP Webhook

### Constraints

| Constraint | Value |
|------------|-------|
| Max text length | Configurable (default: no limit) |
| Max file size | Configurable (default: no limit) |
| Markup support | None (raw JSON) |
| Threading | Not applicable |
| Protocol | HTTP/HTTPS POST |

The HTTP Webhook adapter is intentionally generic. It sends and receives arbitrary JSON payloads. The application configures the endpoint URL and optional authentication headers.

### Inbound: HTTP POST → ChannelMessage

The adapter listens on a configured port and path. When a POST request arrives:

| HTTP element | ChannelMessage field | Notes |
|--------------|---------------------|-------|
| Request body (JSON) | `content: Structured(body)` | Full body parsed as `serde_json::Value` |
| `X-Sender-Id` header (optional) | `metadata.sender_id` | Custom header; adapter can be configured to read any header |
| `X-Chat-Id` header (optional) | `metadata.chat_id` | Custom header |
| Request arrival time | `timestamp` | Set by the adapter at receipt time |
| Full request body | `metadata.raw` | Same as `content` for webhooks |

If the request body is not valid JSON, the adapter wraps it as `content: Text(body_string)`.

### Outbound: ChannelMessage → HTTP POST

The adapter POSTs to the configured endpoint URL:

| ChannelMessage field | HTTP request | Notes |
|---------------------|--------------|-------|
| `content: Structured(v)` | Body: `v` serialized as JSON | Preferred for webhooks |
| `content: Text(s)` | Body: `{"text": s}` | Wrapped in a JSON object |
| `content: Markdown(s)` | Body: `{"markdown": s}` | Wrapped in a JSON object |
| `content: Image { .. }` | Body: `{"mime_type": "...", "data": "<base64>"}` | |
| `content: File { .. }` | Body: `{"name": "...", "mime_type": "...", "data": "<base64>"}` | |
| `content: Audio { .. }` | Body: `{"mime_type": "...", "duration_secs": N, "data": "<base64>"}` | |
| `metadata.chat_id` | `X-Chat-Id` header (optional) | Only sent if non-null |

The adapter sets `Content-Type: application/json` on all outbound requests. Authentication headers (e.g. `Authorization: Bearer <token>`) are configured at adapter construction time and added to every request.

---

## 10. Extension Guide

Adding a new channel requires three steps.

### Step 1: Implement the `Channel` trait

```rust
use async_trait::async_trait;
use claw_channel::{Channel, ChannelError, ChannelMessage};

pub struct SlackChannel {
    bot_token: String,
    // ... other config
}

#[async_trait]
impl Channel for SlackChannel {
    async fn start(&self) -> Result<(), ChannelError> {
        // Connect to Slack's Events API or Socket Mode
        todo!()
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        // Disconnect and clean up
        todo!()
    }

    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError> {
        // Map ChannelMessage to Slack's chat.postMessage API
        todo!()
    }

    fn on_message(&self, handler: Box<dyn Fn(ChannelMessage) + Send + Sync>) {
        // Register handler for incoming Slack events
        todo!()
    }
}
```

### Step 2: Define the platform mapping

Document (or implement) the mapping between your platform's native event/message format and `ChannelMessage`. Follow the same table format used in Sections 7, 8, and 9:

- Inbound: native event fields → `ChannelMessage` fields
- Outbound: `ChannelMessage` fields → native API parameters
- Set `metadata.platform = Platform::Custom("slack".to_string())`
- Store the full native payload in `metadata.raw`

### Step 3: Handle platform-specific errors

Map your platform's error responses to `ChannelError` variants:

- HTTP 429 → `ChannelError::RateLimited { retry_after }`
- HTTP 401/403 → `ChannelError::AuthenticationFailed`
- Payload too large → `ChannelError::MessageTooLarge { max_bytes, actual_bytes }`
- Network failures → `ChannelError::NetworkError(err.to_string())`
- Other API errors → `ChannelError::PlatformError { code, message }`

The retry and rate limiting infrastructure in `claw-channel` works automatically once you return the correct `ChannelError` variants.

---

---

<a name="chinese"></a>
# 渠道消息协议

**状态：** 已接受  
**日期：** 2026-02-28  
**Crate：** `claw-channel`（第 6 阶段）

---

## 1. 概述

`claw-channel` 为外部通信渠道提供统一抽象：Telegram 机器人、Discord 机器人和 HTTP Webhook。第 6 阶段定义的 `Channel` trait 通过 `ChannelMessage` 值进行消息收发。本文档定义完整的 `ChannelMessage` 类型、序列化格式、错误处理、限流策略和重试行为，以及每个支持平台的映射规则。

设计遵循与 `claw-provider`（ADR-006）相同的原则：一个轻薄的、平台中立的信封，携带渠道适配器所需的全部信息，而不将平台特定细节泄漏到 Agent 循环中。

### 范围

本文档涵盖：

- `ChannelMessage` Rust 类型及所有支持类型
- JSON 序列化格式和字段命名约定
- `ChannelError` 变体及其语义
- 限流策略（令牌桶，按渠道）
- 重试策略（带抖动的指数退避）
- Telegram、Discord 和 HTTP Webhook 的平台映射
- 添加新渠道的扩展指南

---

## 2. ChannelMessage 类型定义

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 单条消息的不透明标识符。
/// 封装 UUID v4 以确保跨渠道的全局唯一性。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub Uuid);

impl MessageId {
    /// 生成新的随机消息 ID。
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// 渠道实例的不透明标识符。
/// 通常由应用程序在构建渠道适配器时设置。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub String);

/// 传递给 `Channel` trait 和从其接收的顶层消息信封。
///
/// 流经 claw-channel 的每条消息——来自用户的入站消息或
/// Agent 发出的出站消息——都被包装在此结构体中。
/// 平台特定细节存储在 `metadata.raw` 中，路由时不需要它们。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    /// 全局唯一消息标识符（UUID v4）。
    pub id: MessageId,

    /// 此消息所属的渠道实例。
    pub channel_id: ChannelId,

    /// 此消息是来自用户（入站）还是由 Agent 发送（出站）。
    pub direction: Direction,

    /// 消息创建或接收的时间，UTC。
    /// 序列化为 ISO 8601（例如 "2026-02-28T12:00:00Z"）。
    pub timestamp: DateTime<Utc>,

    /// 消息的实际载荷。
    pub content: MessageContent,

    /// 平台特定元数据（发送者、聊天、线程、原始载荷）。
    pub metadata: ChannelMetadata,

    /// 如果此消息是回复，则为被回复消息的 ID。
    pub reply_to: Option<MessageId>,
}

/// 消息相对于 Agent 的方向。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// 来自用户或外部系统的消息。
    Inbound,
    /// Agent 发送给用户或外部系统的消息。
    Outbound,
}

/// 渠道消息的载荷。
///
/// 变体涵盖所有支持平台的常见内容类型。
/// 对于 Webhook 载荷或不适合其他变体的平台特定数据，使用 `Structured`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    /// 纯 UTF-8 文本。不应用任何标记。
    Text(String),

    /// Markdown 格式文本。
    /// 具体方言（CommonMark、Telegram MarkdownV2、Discord markdown）
    /// 由渠道适配器在发送时确定。
    Markdown(String),

    /// 文件附件。
    File {
        /// 原始文件名，包含扩展名。
        name: String,
        /// MIME 类型（例如 "application/pdf"、"text/csv"）。
        mime_type: String,
        /// 原始文件字节。
        data: Vec<u8>,
    },

    /// 图片。
    Image {
        /// 原始图片字节。
        data: Vec<u8>,
        /// MIME 类型（例如 "image/png"、"image/jpeg"）。
        mime_type: String,
        /// 可选描述，用于无障碍访问或替代文本。
        alt_text: Option<String>,
    },

    /// 音频片段。
    Audio {
        /// 原始音频字节。
        data: Vec<u8>,
        /// MIME 类型（例如 "audio/ogg"、"audio/mpeg"）。
        mime_type: String,
        /// 时长（秒）。未知时使用 0.0。
        duration_secs: f32,
    },

    /// 任意 JSON，用于 HTTP Webhook 载荷或平台特定结构化数据。
    Structured(serde_json::Value),
}

/// 附加到每条消息的平台特定元数据。
///
/// 字段是可选的，因为不是每个平台都提供所有字段。
/// `raw` 字段保留原始平台载荷，供需要访问标准信封未涵盖字段的适配器使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMetadata {
    /// 此消息来自或发往的平台。
    pub platform: Platform,

    /// 平台对发送者的标识符（用户 ID、机器人 ID 等）。
    /// 出站消息或匿名 Webhook 调用时为 `None`。
    pub sender_id: Option<String>,

    /// 平台对会话的标识符（聊天 ID、频道 ID 等）。
    pub chat_id: Option<String>,

    /// 平台对聊天内线程或话题的标识符。
    /// 不支持线程的平台上为 `None`。
    pub thread_id: Option<String>,

    /// 原始平台载荷，原样保留。
    /// 用于访问上述字段未涵盖的平台特定字段。
    pub raw: Option<serde_json::Value>,
}

/// 渠道适配器连接的外部平台。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Telegram,
    Discord,
    Webhook,
    /// 上述未列出的任何平台。字符串是适配器作者选择的简短标识符
    /// （例如 "slack"、"matrix"）。
    Custom(String),
}
```

### 设计说明

- `MessageId` 使用 UUID v4，避免渠道适配器之间的协调。
- `MessageContent` 使用 `#[serde(tag = "type")]`，使 JSON 表示自描述（例如 `{"type": "text", ...}`）。
- 二进制字段（`data: Vec<u8>`）在 JSON 中序列化为 base64 字符串。这由 `serde_with` crate 的 `base64` 辅助工具或自定义序列化器自动处理。
- `ChannelMetadata.raw` 故意使用 `Option<serde_json::Value>` 而非类型化结构体。平台 API 变化频繁；保留原始载荷可避免破坏性变更。

---

## 3. 序列化格式

所有 `ChannelMessage` 值序列化为 JSON。规则如下：

| 规则 | 值 |
|------|-----|
| 字段命名 | `snake_case`（由 `#[serde(rename_all = "snake_case")]` 强制执行） |
| 时间戳格式 | 带 UTC 偏移的 ISO 8601：`"2026-02-28T12:00:00Z"` |
| 枚举变体 | `snake_case` 字符串（例如 `"inbound"`、`"outbound"`） |
| 标记枚举 | `type` 字段作为判别符（例如 `{"type": "text", ...}`） |
| 二进制数据 | Base64 编码字符串（标准字母表，无换行符） |
| 空字段 | `None` 时省略（使用 `#[serde(skip_serializing_if = "Option::is_none")]`） |
| 未知字段 | 反序列化时忽略（仅在测试中使用 `#[serde(deny_unknown_fields)]`） |

### 示例：入站文本消息

```json
{
  "id": "01234567-89ab-cdef-0123-456789abcdef",
  "channel_id": "telegram-main",
  "direction": "inbound",
  "timestamp": "2026-02-28T12:00:00Z",
  "content": {
    "type": "text",
    "value": "你好，Agent！"
  },
  "metadata": {
    "platform": "telegram",
    "sender_id": "123456789",
    "chat_id": "-1001234567890",
    "raw": {
      "update_id": 100000001,
      "message": { "message_id": 42, "text": "你好，Agent！" }
    }
  }
}
```

### 示例：出站 Markdown 消息

```json
{
  "id": "fedcba98-7654-3210-fedc-ba9876543210",
  "channel_id": "discord-support",
  "direction": "outbound",
  "timestamp": "2026-02-28T12:00:05Z",
  "content": {
    "type": "markdown",
    "value": "**结果：** `cargo test` 通过了 42 个测试。"
  },
  "metadata": {
    "platform": "discord",
    "chat_id": "987654321098765432"
  },
  "reply_to": "01234567-89ab-cdef-0123-456789abcdef"
}
```

---

## 4. ChannelError 定义

```rust
use std::time::Duration;
use thiserror::Error;

/// 与渠道交互时可能发生的所有错误。
#[derive(Debug, Error)]
pub enum ChannelError {
    /// 触发了平台的限流。适配器应在 `retry_after` 后重试。
    #[error("rate limited: retry after {retry_after:?}")]
    RateLimited {
        /// 下次尝试前需要等待的时间。
        retry_after: Duration,
    },

    /// 机器人令牌或 Webhook 密钥被平台拒绝。
    /// 不修复凭据就重试没有帮助。
    #[error("authentication failed")]
    AuthenticationFailed,

    /// 消息载荷超过平台的大小限制。
    #[error("message too large: max {max_bytes} bytes, got {actual_bytes} bytes")]
    MessageTooLarge {
        /// 平台允许的最大字节数。
        max_bytes: usize,
        /// 被拒绝载荷的实际字节数。
        actual_bytes: usize,
    },

    /// 网络级错误（DNS 失败、连接被拒、TLS 错误、超时）。
    /// 内部字符串是底层错误消息。
    #[error("network error: {0}")]
    NetworkError(String),

    /// 平台返回了带有数字代码的错误响应。
    /// 请查阅平台 API 文档了解 `code` 的含义。
    #[error("platform error {code}: {message}")]
    PlatformError {
        /// 平台特定错误代码（例如 Telegram 错误代码、Discord 错误代码）。
        code: i32,
        /// 来自平台的人类可读错误消息。
        message: String,
    },

    /// 在调用 `send` 或 `on_message` 之前未启动渠道。
    #[error("channel not started")]
    NotStarted,

    /// 渠道已停止，无法重用。
    #[error("channel stopped")]
    Stopped,
}

impl ChannelError {
    /// 如果此错误是暂时性的且操作可以重试，返回 `true`。
    ///
    /// `AuthenticationFailed`、`MessageTooLarge`、`NotStarted` 和 `Stopped`
    /// 是永久性错误。其他所有错误被视为暂时性错误。
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ChannelError::RateLimited { .. }
                | ChannelError::NetworkError(_)
                | ChannelError::PlatformError { .. }
        )
    }

    /// 如果平台告知了确切的等待时间，返回该时长。
    /// 否则返回 `None`，让重试策略决定。
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            ChannelError::RateLimited { retry_after } => Some(*retry_after),
            _ => None,
        }
    }
}
```

### 各渠道的错误处理

| 错误变体 | Telegram | Discord | HTTP Webhook |
|----------|----------|---------|--------------|
| `RateLimited` | HTTP 429 带 `retry_after` 字段 | HTTP 429 带 `X-RateLimit-Reset-After` 头 | HTTP 429（如果服务器发送） |
| `AuthenticationFailed` | `{"ok": false, "error_code": 401}` | HTTP 401 | HTTP 401 或 403 |
| `MessageTooLarge` | 文本 > 4096 字符或文件 > 50 MB | 文本 > 2000 字符或文件 > 25 MB | 可配置（默认：无限制） |
| `NetworkError` | reqwest 错误 | reqwest 错误 | reqwest 错误 |
| `PlatformError` | `{"ok": false, "error_code": N}` | `{"code": N, "message": "..."}` | 任何非 2xx HTTP 状态 |

---

## 5. 限流策略

每个渠道实例维护独立的令牌桶。即使连接到同一平台，令牌桶也不在渠道实例之间共享。

### 令牌桶参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| 容量 | 30 个令牌 | 最大突发大小 |
| 补充速率 | 30 个/分钟（0.5 个/秒） | 与 Telegram 文档的机器人限制一致 |
| 初始令牌 | 30 | 桶初始为满 |
| 范围 | 每个 `ChannelId` | 每个渠道有自己的桶 |

### 行为

1. 在调用平台 API 之前，适配器尝试消耗一个令牌。
2. 如果令牌可用，调用立即进行。
3. 如果没有令牌，适配器等待直到补充一个（最多 2 秒），然后重试。
4. 如果平台返回 `HTTP 429`，适配器遵守响应中的 `retry_after` 值，无论本地桶状态如何，并将桶重置为 0 以强制暂停。

### 配置

```rust
/// 渠道实例的限流配置。
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// 可以突发发送的最大消息数。
    pub capacity: u32,
    /// 每秒添加的令牌数。
    pub refill_per_second: f64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            capacity: 30,
            refill_per_second: 0.5, // 每分钟 30 条
        }
    }
}
```

各平台默认值（构建适配器时覆盖 `RateLimitConfig`）：

| 平台 | 容量 | 补充速率 | 来源 |
|------|------|----------|------|
| Telegram | 30 | 0.5/秒 | Telegram Bot API 文档（群组限制 30 条/秒） |
| Discord | 5 | 0.2/秒 | Discord 限流：每个频道每 5 秒 5 条消息 |
| HTTP Webhook | 100 | 10/秒 | 保守默认值；按部署覆盖 |

---

## 6. 重试策略

重试策略适用于 `is_retryable()` 返回 `true` 的所有 `ChannelError` 变体。

### 参数

| 参数 | 值 |
|------|-----|
| 初始延迟 | 1 秒 |
| 倍数 | 每次尝试 2 倍 |
| 最大延迟 | 60 秒 |
| 抖动 | 计算延迟的 ±25%（均匀随机） |
| 最大尝试次数 | 3 次 |
| 不可重试错误 | 立即失败，不重试 |

### 延迟计划（不含抖动）

| 尝试次数 | 尝试前延迟 |
|----------|-----------|
| 第 1 次（首次尝试） | 0 毫秒（立即） |
| 第 2 次 | 1 秒 |
| 第 3 次 | 2 秒 |
| 第 4 次及以上 | 不尝试 |

如果错误是带有 `retry_after` 值的 `RateLimited`，该值替换该次尝试的计算延迟（但抖动仍然叠加）。

### 实现草图

```rust
pub async fn send_with_retry(
    channel: &dyn Channel,
    message: ChannelMessage,
    config: &RetryConfig,
) -> Result<(), ChannelError> {
    let mut delay = config.initial_delay;
    for attempt in 0..config.max_attempts {
        match channel.send(message.clone()).await {
            Ok(()) => return Ok(()),
            Err(err) if !err.is_retryable() => return Err(err),
            Err(err) => {
                if attempt + 1 == config.max_attempts {
                    return Err(err);
                }
                // 如果平台提供了 retry_after，则遵守。
                let base = err.retry_after().unwrap_or(delay);
                let jitter = base.mul_f64(rand::random::<f64>() * 0.5 - 0.25);
                tokio::time::sleep(base + jitter).await;
                delay = (delay * 2).min(config.max_delay);
            }
        }
    }
    unreachable!()
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub max_attempts: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            max_attempts: 3,
        }
    }
}
```

---

## 7. 平台映射：Telegram

### 约束

| 约束 | 值 |
|------|-----|
| 最大文本长度 | 每条消息 4096 字符 |
| 最大文件大小 | 50 MB（通过 Bot API） |
| 标记支持 | MarkdownV2、HTML、纯文本 |
| 线程支持 | 仅回复链（无命名线程） |
| Bot API 版本 | 7.x |

### 入站：Telegram Update → ChannelMessage

当 Telegram 适配器从 Bot API 接收到 `Update` 对象时：

| Telegram 字段 | ChannelMessage 字段 | 说明 |
|---------------|---------------------|------|
| `update_id` | `metadata.raw` | 原样存储 |
| `message.message_id` | `metadata.raw` | 原样存储 |
| `message.from.id` | `metadata.sender_id` | 字符串化整数 |
| `message.chat.id` | `metadata.chat_id` | 字符串化整数 |
| `message.reply_to_message.message_id` | `reply_to` | 如果已跟踪则映射到 `MessageId` |
| `message.text` | `content: Text(...)` | 纯文本消息 |
| `message.text`（带实体） | `content: Markdown(...)` | 存在 `parse_mode` 实体时 |
| `message.photo`（最大尺寸） | `content: Image { ... }` | 适配器下载文件 |
| `message.document` | `content: File { ... }` | 适配器下载文件 |
| `message.voice` / `message.audio` | `content: Audio { ... }` | 适配器下载文件 |
| `message.date` | `timestamp` | Unix 时间戳转换为 `DateTime<Utc>` |

完整的 `Update` JSON 存储在 `metadata.raw` 中。

### 出站：ChannelMessage → Telegram sendMessage

| ChannelMessage 字段 | Telegram API 调用 | 说明 |
|---------------------|-------------------|------|
| `content: Text(s)` | `sendMessage`，`text=s` | 无 `parse_mode` |
| `content: Markdown(s)` | `sendMessage`，`text=s, parse_mode=MarkdownV2` | 适配器转义特殊字符 |
| `content: Image { data, .. }` | `sendPhoto`，`photo=<bytes>` | |
| `content: File { data, name, .. }` | `sendDocument`，`document=<bytes>` | |
| `content: Audio { data, .. }` | `sendAudio` 或 `sendVoice` | ogg/opus 用 `sendVoice`，其他用 `sendAudio` |
| `content: Structured(v)` | `sendMessage`，`text=serde_json::to_string(v)` | 回退方案；优先使用明确的变体 |
| `reply_to` | `reply_parameters.message_id` | 需要 `metadata.raw` 中的原始 Telegram 消息 ID |
| `metadata.chat_id` | `chat_id` 参数 | 必填 |

**文本分割：** 如果 `Text` 或 `Markdown` 内容超过 4096 字符，适配器将其分割为多个连续的 `sendMessage` 调用。每部分使用相同的 `chat_id`。只有第一部分携带 `reply_parameters`。

---

## 8. 平台映射：Discord

### 约束

| 约束 | 值 |
|------|-----|
| 最大文本长度 | 每条消息 2000 字符 |
| 最大文件大小 | 25 MB（标准），500 MB（Nitro） |
| 标记支持 | Discord markdown（CommonMark 子集） |
| 线程支持 | 论坛频道、线程频道、回复链 |
| API 版本 | Discord API v10 |

### 入站：Discord 消息事件 → ChannelMessage

当 Discord 适配器接收到 `MESSAGE_CREATE` 网关事件时：

| Discord 字段 | ChannelMessage 字段 | 说明 |
|--------------|---------------------|------|
| `id` | `metadata.raw` | 原样存储（雪花字符串） |
| `author.id` | `metadata.sender_id` | 雪花字符串 |
| `channel_id` | `metadata.chat_id` | 雪花字符串 |
| `thread.id`（如存在） | `metadata.thread_id` | 雪花字符串 |
| `message_reference.message_id` | `reply_to` | 如果已跟踪则映射到 `MessageId` |
| `content` | `content: Text(...)` 或 `content: Markdown(...)` | 内容包含 Discord markdown 语法时用 `Markdown` |
| `attachments[0]`（图片） | `content: Image { ... }` | 适配器从 `url` 下载 |
| `attachments[0]`（其他） | `content: File { ... }` | 适配器从 `url` 下载 |
| `timestamp` | `timestamp` | ISO 8601 字符串，解析为 `DateTime<Utc>` |
| 完整事件 JSON | `metadata.raw` | 原样存储 |

当消息同时包含文本和附件时，适配器为每个附件创建一个 `ChannelMessage`，如果文本非空则再创建一个文本消息。

### 出站：ChannelMessage → Discord 创建消息

| ChannelMessage 字段 | Discord API 字段 | 说明 |
|---------------------|-----------------|------|
| `content: Text(s)` | `content: s` | |
| `content: Markdown(s)` | `content: s` | Discord 原生渲染 markdown |
| `content: Image { data, .. }` | `files[0]` 多部分 | |
| `content: File { data, name, .. }` | `files[0]` 多部分 | |
| `content: Audio { data, .. }` | `files[0]` 多部分 | |
| `content: Structured(v)` | `embeds[0]` | 适配器将 JSON 转换为 Discord embed |
| `reply_to` | `message_reference.message_id` | 需要 `metadata.raw` 中的原始 Discord 雪花 ID |
| `metadata.chat_id` | URL 路径参数 | 必填 |
| `metadata.thread_id` | `thread_id` 查询参数 | 可选 |

**文本分割：** 如果 `Text` 或 `Markdown` 内容超过 2000 字符，适配器将其分割为多个连续消息。只有第一条携带 `message_reference`。

---

## 9. 平台映射：HTTP Webhook

### 约束

| 约束 | 值 |
|------|-----|
| 最大文本长度 | 可配置（默认：无限制） |
| 最大文件大小 | 可配置（默认：无限制） |
| 标记支持 | 无（原始 JSON） |
| 线程支持 | 不适用 |
| 协议 | HTTP/HTTPS POST |

HTTP Webhook 适配器故意设计为通用的。它发送和接收任意 JSON 载荷。应用程序配置端点 URL 和可选的认证头。

### 入站：HTTP POST → ChannelMessage

适配器监听配置的端口和路径。当 POST 请求到达时：

| HTTP 元素 | ChannelMessage 字段 | 说明 |
|-----------|---------------------|------|
| 请求体（JSON） | `content: Structured(body)` | 完整请求体解析为 `serde_json::Value` |
| `X-Sender-Id` 头（可选） | `metadata.sender_id` | 自定义头；适配器可配置为读取任意头 |
| `X-Chat-Id` 头（可选） | `metadata.chat_id` | 自定义头 |
| 请求到达时间 | `timestamp` | 由适配器在接收时设置 |
| 完整请求体 | `metadata.raw` | 与 Webhook 的 `content` 相同 |

如果请求体不是有效 JSON，适配器将其包装为 `content: Text(body_string)`。

### 出站：ChannelMessage → HTTP POST

适配器向配置的端点 URL 发送 POST 请求：

| ChannelMessage 字段 | HTTP 请求 | 说明 |
|---------------------|-----------|------|
| `content: Structured(v)` | 请求体：`v` 序列化为 JSON | Webhook 的首选方式 |
| `content: Text(s)` | 请求体：`{"text": s}` | 包装在 JSON 对象中 |
| `content: Markdown(s)` | 请求体：`{"markdown": s}` | 包装在 JSON 对象中 |
| `content: Image { .. }` | 请求体：`{"mime_type": "...", "data": "<base64>"}` | |
| `content: File { .. }` | 请求体：`{"name": "...", "mime_type": "...", "data": "<base64>"}` | |
| `content: Audio { .. }` | 请求体：`{"mime_type": "...", "duration_secs": N, "data": "<base64>"}` | |
| `metadata.chat_id` | `X-Chat-Id` 头（可选） | 非空时才发送 |

适配器在所有出站请求上设置 `Content-Type: application/json`。认证头（例如 `Authorization: Bearer <token>`）在适配器构建时配置，并添加到每个请求中。

---

## 10. 扩展指南

添加新渠道需要三个步骤。

### 第 1 步：实现 `Channel` trait

```rust
use async_trait::async_trait;
use claw_channel::{Channel, ChannelError, ChannelMessage};

pub struct SlackChannel {
    bot_token: String,
    // ... 其他配置
}

#[async_trait]
impl Channel for SlackChannel {
    async fn start(&self) -> Result<(), ChannelError> {
        // 连接到 Slack 的 Events API 或 Socket Mode
        todo!()
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        // 断开连接并清理
        todo!()
    }

    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError> {
        // 将 ChannelMessage 映射到 Slack 的 chat.postMessage API
        todo!()
    }

    fn on_message(&self, handler: Box<dyn Fn(ChannelMessage) + Send + Sync>) {
        // 注册处理器以接收传入的 Slack 事件
        todo!()
    }
}
```

### 第 2 步：定义平台映射

记录（或实现）平台原生事件/消息格式与 `ChannelMessage` 之间的映射。遵循第 7、8、9 节使用的相同表格格式：

- 入站：原生事件字段 → `ChannelMessage` 字段
- 出站：`ChannelMessage` 字段 → 原生 API 参数
- 设置 `metadata.platform = Platform::Custom("slack".to_string())`
- 将完整的原生载荷存储在 `metadata.raw` 中

### 第 3 步：处理平台特定错误

将平台的错误响应映射到 `ChannelError` 变体：

- HTTP 429 → `ChannelError::RateLimited { retry_after }`
- HTTP 401/403 → `ChannelError::AuthenticationFailed`
- 载荷过大 → `ChannelError::MessageTooLarge { max_bytes, actual_bytes }`
- 网络故障 → `ChannelError::NetworkError(err.to_string())`
- 其他 API 错误 → `ChannelError::PlatformError { code, message }`

一旦你返回正确的 `ChannelError` 变体，`claw-channel` 中的重试和限流基础设施就会自动工作。
