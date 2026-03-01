---
title: 渠道消息协议
description: "claw-channel 的消息协议：消息类型、序列化格式与平台适配器"
status: accepted
date: 2026-02-28
type: design
last_updated: "2026-03-01"
language: zh
---

[English →](channel-message-protocol.md)

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
