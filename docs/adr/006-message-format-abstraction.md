---
title: ADR-006: Message Format Abstraction
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-01"
language: en
---

[中文版 →](006-message-format-abstraction.zh.md)

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
