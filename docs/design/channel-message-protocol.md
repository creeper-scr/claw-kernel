---
title: Channel Message Protocol
description: "Wire format and framing protocol for claw-channel: message types, serialization, and platform adapters"
status: accepted
date: 2026-02-28
type: design
last_updated: "2026-03-01"
language: en
---

[中文版 →](channel-message-protocol.zh.md)

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
