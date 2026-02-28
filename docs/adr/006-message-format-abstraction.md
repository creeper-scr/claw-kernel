[English](#english) | [中文](#chinese)

<a name="english"></a>
# ADR 006: Message Format Abstraction for LLM Providers

**Status:** Accepted  
**Date:** 2026-02-28  
**Deciders:** claw-kernel core team

---

## Context

The LLM provider landscape has consolidated around two dominant API formats:

1. **OpenAI Format** — Used by OpenAI, DeepSeek, Moonshot, Qwen, Grok, Azure, and 50+ cloud providers
2. **Anthropic Format** — Used by Anthropic (Claude) and AWS Bedrock

Our original design had each provider implementing `LLMProvider` trait directly, resulting in:
- ~300 lines of duplicated HTTP/serialization code per provider
- Inconsistent handling of streaming, errors, and token counting
- High barrier to adding new providers (even when they use the same protocol)

### Problem Analysis

| Provider | Format | Original LOC | Duplication |
|----------|--------|--------------|-------------|
| OpenAI | OpenAI | ~300 | Baseline |
| DeepSeek | OpenAI | ~280 | 93% identical |
| Moonshot | OpenAI | ~290 | 97% identical |
| Qwen | OpenAI | ~285 | 95% identical |
| Grok | OpenAI | ~280 | 93% identical |
| Anthropic | Anthropic | ~320 | Baseline |
| Bedrock | Anthropic | ~350 | 85% identical (AWS auth differs) |

**Key insight:** Provider differentiation is primarily in **configuration** (base URL, auth headers), not **protocol logic**.

---

## Decision

We introduce a **three-tier abstraction** within Layer 2 (Agent Kernel Protocol) for the provider system:

> **Note**: The following Tier 1/2/3 refers to internal architecture within `claw-provider` (Layer 2 of the system architecture), distinct from the global 5-layer architecture (Layer 0-3).

```
┌─────────────────────────────────────────────────────────┐
│  Tier 3: Provider Configuration                         │
│  Structs holding base_url, api_key, model, client       │
├─────────────────────────────────────────────────────────┤
│  Tier 2: HttpTransport (Reusable Trait)                 │
│  Generic request/stream logic, delegates to Format      │
├─────────────────────────────────────────────────────────┤
│  Tier 1: MessageFormat (Protocol Abstraction)           │
│  AnthropicFormat, OpenAIFormat                          │
│  Request/response serialization, token counting         │
└─────────────────────────────────────────────────────────┘
```

### Core Traits

```rust
/// Tier 1: Protocol-specific serialization (within Layer 2)
#[async_trait]
pub trait MessageFormat: Send + Sync {
    type Request: Serialize;
    type Response: DeserializeOwned;
    type StreamChunk: DeserializeOwned;
    type Error: std::error::Error;
    
    fn build_request(messages: &[Message], opts: &Options) -> Self::Request;
    fn parse_response(raw: Self::Response) -> Result<CompletionResponse, Self::Error>;
    fn parse_stream_chunk(chunk: &[u8]) -> Result<Option<Delta>, Self::Error>;
    fn token_count(messages: &[Message]) -> usize;
    fn endpoint() -> &'static str;
}

/// Layer 2: Reusable HTTP logic
#[async_trait]
pub trait HttpTransport: Send + Sync {
    fn base_url(&self) -> &str;
    fn auth_headers(&self) -> HeaderMap;
    fn http_client(&self) -> &Client;
    
    async fn request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> Result<CompletionResponse, ProviderError>;
    
    async fn stream_request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
}

/// Layer 3: User-facing trait (unchanged interface)
#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError>;
    async fn stream_complete(&self, messages: &[Message], opts: &Options) 
        -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
    fn token_count(&self, messages: &[Message]) -> usize;
}

/// Separate trait for embedding capability (not all providers support this)
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, ProviderError>;
}
```

### Implementation Pattern

A new OpenAI-compatible provider now requires only **configuration**:

```rust
pub struct DeepSeekProvider {
    api_key: String,
    model: String,
    client: Client,
}

#[async_trait]
impl LLMProvider for DeepSeekProvider {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError> {
        // Delegate to generic transport with OpenAI format
        self.request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn stream_complete(&self, messages: &[Message], opts: &Options) 
        -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError> {
        self.stream_request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, ProviderError> {
        // Implementation using OpenAI's embedding API
        todo!("Implement embedding")
    }
    
    fn token_count(&self, messages: &[Message]) -> usize {
        OpenAIFormat::token_count(messages)
    }
}

impl HttpTransport for DeepSeekProvider {
    fn base_url(&self) -> &str { "https://api.deepseek.com/v1" }
    fn auth_headers(&self) -> HeaderMap { /* Bearer token */ }
    fn http_client(&self) -> &Client { &self.client }
}
```

---

## Consequences

### Positive

- **Dramatic code reduction:** New OpenAI-compatible provider = ~20 lines (was ~300)
- **Protocol consistency:** All providers using OpenAIFormat handle streaming/errors identically
- **Easier maintenance:** OpenAI API changes require modifying one file, not N
- **Better testing:** Format layer tested once, reused everywhere
- **Extensibility:** Users can add providers without upstream changes (just config)

### Neutral

- **Additional trait complexity:** Three traits instead of one (mitigated by clear layering)
- **Learning curve:** Contributors must understand the three-layer model

### Negative

- **Generic code complexity:** Heavy use of `<F: MessageFormat>` type parameters
- **Documentation effort:** Must clearly explain when to use existing format vs. create new

---

## Alternatives Considered

### Alternative 1: Macro-based code generation

**Rejected:** Macros hide complexity but don't improve actual abstraction. Debugging macro-generated code is harder than understanding the trait layer.

### Alternative 2: Runtime format selection

**Rejected:**
```rust
// Discarded approach
pub struct GenericProvider {
    format: Box<dyn MessageFormat>, // Dynamic dispatch
    base_url: String,
}
```
Dynamic dispatch loses performance and type safety. Compile-time generics preferred.

### Alternative 3: HTTP client middleware

**Rejected:** Middleware approach conflates transport concerns (auth, URL) with protocol concerns (message serialization). Our separation is cleaner.

---

## Migration Path

Existing providers in the ecosystem can migrate incrementally:

1. **Phase 1:** Implement `MessageFormat` for your protocol (if unique)
2. **Phase 2:** Implement `HttpTransport` for your auth/URL patterns
3. **Phase 3:** Implement `LLMProvider` by delegating to `HttpTransport`

---

## References

- [claw-provider crate documentation](../crates/claw-provider.md)
- [Architecture Overview](../architecture/overview.md)

---

<a name="chinese"></a>
# ADR 006: LLM Provider 消息格式抽象

**状态：** 已接受  
**日期：** 2026-02-28  
**决策者：** claw-kernel 核心团队

---

## 背景

LLM provider 生态系统已围绕两种主导 API 格式整合：

1. **OpenAI 格式** — OpenAI、DeepSeek、Moonshot、Qwen、Grok、Azure 及 50+ 云服务商使用
2. **Anthropic 格式** — Anthropic (Claude) 和 AWS Bedrock 使用

我们最初的设计让每个 provider 直接实现 `LLMProvider` trait，导致：
- 每个 provider 约 300 行重复的 HTTP/序列化代码
- 流式处理、错误处理和 token 计数不一致
- 添加新 provider 的门槛高（即使它们使用相同的协议）

### 问题分析

| Provider | 使用格式 | 原始代码行数 | 重复度 |
|----------|----------|--------------|--------|
| OpenAI | OpenAI | ~300 | 基准 |
| DeepSeek | OpenAI | ~280 | 93% 相同 |
| Moonshot | OpenAI | ~290 | 97% 相同 |
| Qwen | OpenAI | ~285 | 95% 相同 |
| Grok | OpenAI | ~280 | 93% 相同 |
| Anthropic | Anthropic | ~320 | 基准 |
| Bedrock | Anthropic | ~350 | 85% 相同（AWS 认证不同）|

**关键洞察：** Provider 的差异化主要在**配置**（base URL、认证头），而非**协议逻辑**。

---

## 决策

我们在第 2 层（Agent 内核协议）内为 provider 系统引入**三层内部抽象**：

> **注意**：以下第 1/2/3 层级是指 `claw-provider` 内部的三层架构（属于系统架构的第 2 层），与全局五层架构（第 0-3 层）不同。

```
┌─────────────────────────────────────────────────────────┐
│  第 3 层级：Provider 配置                                │
│  包含 base_url、api_key、model、client 的结构体          │
├─────────────────────────────────────────────────────────┤
│  第 2 层级：HttpTransport（可复用 Trait）                │
│  通用请求/流式逻辑，委托给 Format 处理                   │
├─────────────────────────────────────────────────────────┤
│  第 1 层级：MessageFormat（协议抽象）                    │
│  AnthropicFormat, OpenAIFormat                          │
│  请求/响应序列化、token 计数                             │
└─────────────────────────────────────────────────────────┘
```

### 核心 Trait

```rust
/// 第 1 层级：协议特定序列化（第 2 层内部）
pub trait MessageFormat: Send + Sync {
    type Request: Serialize;
    type Response: DeserializeOwned;
    type StreamChunk: DeserializeOwned;
    type Error: std::error::Error;
    
    fn build_request(messages: &[Message], opts: &Options) -> Self::Request;
    fn parse_response(raw: Self::Response) -> Result<CompletionResponse, Self::Error>;
    fn parse_stream_chunk(chunk: &[u8]) -> Result<Option<Delta>, Self::Error>;
    fn token_count(messages: &[Message]) -> usize;
    fn endpoint() -> &'static str;
}

/// 第 2 层级：可复用 HTTP 逻辑（第 2 层内部）
#[async_trait]
pub trait HttpTransport: Send + Sync {
    fn base_url(&self) -> &str;
    fn auth_headers(&self) -> HeaderMap;
    fn http_client(&self) -> &Client;
    
    async fn request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> Result<CompletionResponse, ProviderError>;
    
    async fn stream_request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
}

/// 第 3 层级：面向用户的 trait（第 2 层内部，接口不变）
#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError>;
    async fn stream_complete(&self, messages: &[Message], opts: &Options) -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
    fn token_count(&self, messages: &[Message]) -> usize;
}

/// 单独的嵌入接口（不是所有 provider 都支持）
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, ProviderError>;
}
```

### 实现模式

一个新的 OpenAI 兼容 provider 现在只需要**配置**：

```rust
pub struct DeepSeekProvider {
    api_key: String,
    model: String,
    client: Client,
}

#[async_trait]
impl LLMProvider for DeepSeekProvider {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError> {
        // 委托给使用 OpenAI 格式的通用传输层
        self.request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn stream_complete(&self, messages: &[Message], opts: &Options) 
        -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError> {
        self.stream_request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, ProviderError> {
        // 使用 OpenAI 的嵌入 API 实现
        todo!("实现嵌入功能")
    }
    
    fn token_count(&self, messages: &[Message]) -> usize {
        OpenAIFormat::token_count(messages)
    }
}

impl HttpTransport for DeepSeekProvider {
    fn base_url(&self) -> &str { "https://api.deepseek.com/v1" }
    fn auth_headers(&self) -> HeaderMap { /* Bearer token */ }
    fn http_client(&self) -> &Client { &self.client }
}
```

---

## 后果

### 积极方面

- **代码大幅减少：** 新的 OpenAI 兼容 provider = ~20 行（原来是 ~300 行）
- **协议一致性：** 所有使用 OpenAIFormat 的 provider 以相同方式处理流式/错误
- **更容易维护：** OpenAI API 变更只需修改一个文件，而非 N 个
- **更好的测试：** Format 层只需测试一次，到处复用
- **可扩展性：** 用户无需上游变更即可添加 provider（仅需配置）

### 中性方面

- **额外的 trait 复杂性：** 三个 trait 而非一个（通过清晰的分层缓解）
- **学习曲线：** 贡献者必须理解三层模型

### 消极方面

- **泛型代码复杂性：** 大量使用 `<F: MessageFormat>` 类型参数
- **文档工作量：** 必须清楚解释何时使用现有格式 vs 创建新格式

---

## 考虑的替代方案

### 替代方案 1：基于宏的代码生成

**已拒绝：** 宏隐藏了复杂性，但没有改善实际抽象。调试宏生成的代码比理解 trait 层更难。

### 替代方案 2：运行时格式选择

**已拒绝：**
```rust
// 废弃的方法
pub struct GenericProvider {
    format: Box<dyn MessageFormat>, // 动态分发
    base_url: String,
}
```
动态分发损失性能和类型安全。优先选择编译时泛型。

### 替代方案 3：HTTP 客户端中间件

**已拒绝：** 中间件方法混淆了传输关注点（认证、URL）与协议关注点（消息序列化）。我们的分离更清晰。

---

## 迁移路径

生态系统中的现有 provider 可以增量迁移：

1. **阶段 1：** 为你的协议实现 `MessageFormat`（如果是唯一的）
2. **阶段 2：** 为你的认证/URL 模式实现 `HttpTransport`
3. **阶段 3：** 通过委托给 `HttpTransport` 实现 `LLMProvider`

---

## 参考

- [claw-provider crate 文档](../crates/claw-provider.md)
- [架构概述](../architecture/overview.md)
