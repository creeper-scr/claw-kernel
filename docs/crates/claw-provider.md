---
title: claw-provider
description: LLM provider trait + Anthropic/OpenAI/Ollama implementations
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](claw-provider.zh.md)


LLM Provider abstraction with a three-tier internal architecture: Message Format → HTTP Transport → Provider Configuration.

---

## Overview

`claw-provider` uses a **protocol-format abstraction** design to maximize code reuse. Instead of implementing HTTP logic for each provider, we abstract by **message format compatibility**:

```
┌─────────────────────────────────────────────────────────┐
│  Tier 3: Provider Configuration                         │
│  AnthropicProvider, DeepSeekProvider, MoonshotProvider  │
│  (Only base_url + auth config)                          │
├─────────────────────────────────────────────────────────┤
│  Tier 2: HTTP Transport (Reusable)                      │
│  HttpTransport trait with request/stream logic          │
├─────────────────────────────────────────────────────────┤
│  Tier 1: Message Format (Protocol Abstraction)          │
│  AnthropicFormat, OpenAIFormat                          │
│  (Request/response serialization + token counting)      │
└─────────────────────────────────────────────────────────┘
```

> **Note:** Tier 1/2/3 refers to the internal architecture of claw-provider (Layer 2 of the kernel), 
> not to be confused with the kernel's 5-layer architecture (Layer 0-3).

### Why Format Abstraction?

| Provider | Format | Lines of Code |
|----------|--------|---------------|
| OpenAI | OpenAIFormat | ~20 (config only) |
| DeepSeek | OpenAIFormat | ~20 (config only) |
| Moonshot | OpenAIFormat | ~20 (config only) |
| Qwen | OpenAIFormat | ~20 (config only) |
| Anthropic | AnthropicFormat | ~20 (config only) |
| Bedrock | AnthropicFormat | ~30 (AWS auth) |

> 90% of LLM providers in the market are compatible with either OpenAI or Anthropic format. This architecture eliminates duplicate HTTP and serialization code.

---

## Usage

```toml
[dependencies]
claw-provider = { version = "0.1", features = ["anthropic", "openai", "deepseek"] }
```

```rust
use claw_provider::{LLMProvider, AnthropicProvider, Message, Role};

let provider = AnthropicProvider::from_env()?;

let response = provider.complete(&[
    Message {
        role: Role::User,
        content: "Hello!".to_string(),
    }
], &Default::default()).await?;

println!("{}", response.content);
```

---

## Built-in Providers

### Anthropic-Compatible Providers

```rust
use claw_provider::AnthropicProvider;

// Anthropic (Claude)
let provider = AnthropicProvider::from_env()?; // ANTHROPIC_API_KEY

// AWS Bedrock (Claude via AWS)
use claw_provider::BedrockProvider;
let provider = BedrockProvider::new(BedrockConfig {
    region: "us-east-1".to_string(),
    model: "anthropic.claude-3-opus-20240229-v1:0".to_string(),
});
```

### OpenAI-Compatible Providers

All OpenAI-compatible providers share the same protocol implementation, differing only in base URL and authentication.

```rust
use claw_provider::{OpenAIProvider, DeepSeekProvider, MoonshotProvider};

// OpenAI
let provider = OpenAIProvider::from_env()?; // OPENAI_API_KEY

// DeepSeek
let provider = DeepSeekProvider::from_env()?; // DEEPSEEK_API_KEY

// Moonshot (月之暗面)
let provider = MoonshotProvider::from_env()?; // MOONSHOT_API_KEY

// Qwen (通义千问)
let provider = QwenProvider::from_env()?; // QWEN_API_KEY

// Grok (xAI)
let provider = GrokProvider::from_env()?; // GROK_API_KEY

// Azure OpenAI (special auth handling)
let provider = AzureOpenAIProvider::from_env()?; // AZURE_OPENAI_API_KEY
```

### Local Models

```rust
use claw_provider::OllamaProvider;

let provider = OllamaProvider::new(OllamaConfig {
    base_url: "http://localhost:11434".to_string(),
    model: "llama2".to_string(),
});
```

---

## Architecture

### Tier 1: MessageFormat (Protocol Abstraction)

```rust
#[async_trait]
pub trait MessageFormat: Send + Sync {
    type Request: Serialize;
    type Response: DeserializeOwned;
    type StreamChunk: DeserializeOwned;
    type Error: std::error::Error;
    
    /// Convert unified Message to format-specific request
    fn build_request(messages: &[Message], opts: &Options) -> Self::Request;
    
    /// Parse response to unified format
    fn parse_response(raw: Self::Response) -> Result<CompletionResponse, Self::Error>;
    
    /// Parse streaming chunk
    fn parse_stream_chunk(chunk: &[u8]) -> Result<Option<Delta>, Self::Error>;
    
    /// Token counting (format-specific algorithm)
    fn token_count(messages: &[Message]) -> usize;
    
    /// API endpoint path
    fn endpoint() -> &'static str;
}
```

**Built-in formats:**
- `OpenAIFormat` — Used by OpenAI, DeepSeek, Moonshot, Qwen, Grok, and most cloud providers
- `AnthropicFormat` — Used by Anthropic (Claude) and AWS Bedrock
- `OllamaFormat` — Ollama's OpenAI-compatible variant

### Tier 2: HttpTransport (Reusable)

```rust
#[async_trait]
pub trait HttpTransport: Send + Sync {
    fn base_url(&self) -> &str;
    fn auth_headers(&self) -> HeaderMap;
    fn http_client(&self) -> &Client;
    
    /// Generic request — reused by all providers
    async fn request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> Result<CompletionResponse, ProviderError> {
        let req = F::build_request(messages, opts);
        let response = self.http_client()
            .post(format!("{}{}", self.base_url(), F::endpoint()))
            .headers(self.auth_headers())
            .json(&req)
            .send()
            .await?
            .json::<F::Response>()
            .await?;
        
        Ok(F::parse_response(response))
    }
    
    /// Generic streaming request
    async fn stream_request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
}
```

### Tier 3: Provider (Configuration Only)

```rust
pub struct DeepSeekProvider {
    api_key: String,
    model: String,
    client: Client,
}

#[async_trait]
impl LLMProvider for DeepSeekProvider {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError> {
        // Simply delegate to HttpTransport with OpenAIFormat
        self.request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn stream_complete(&self, messages: &[Message], opts: &Options) 
        -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError> {
        self.stream_request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, ProviderError> {
        // Implementation using OpenAI's embedding API
        self.request_embedding::<OpenAIFormat>(texts).await
    }
    
    fn token_count(&self, messages: &[Message]) -> usize {
        OpenAIFormat::token_count(messages)
    }
}

impl HttpTransport for DeepSeekProvider {
    fn base_url(&self) -> &str { "https://api.deepseek.com/v1" }
    
    fn auth_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key)).unwrap(),
        );
        headers
    }
    
    fn http_client(&self) -> &Client { &self.client }
}
```

---

## Custom Provider

### Option A: Use Existing Format (Recommended)

If your provider uses OpenAI or Anthropic compatible format:

```rust
use claw_provider::{OpenAIFormat, HttpTransport, LLMProvider, Message, CompletionResponse};
use async_trait::async_trait;

pub struct MyProvider {
    api_key: String,
    base_url: String,
    client: Client,
}

#[async_trait]
impl LLMProvider for MyProvider {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError> {
        self.request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn stream_complete(&self, messages: &[Message], opts: &Options) 
        -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError> {
        self.stream_request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, ProviderError> {
        self.request_embedding::<OpenAIFormat>(texts).await
    }
    
    fn token_count(&self, messages: &[Message]) -> usize {
        OpenAIFormat::token_count(messages)
    }
}

impl HttpTransport for MyProvider {
    fn base_url(&self) -> &str { &self.base_url }
    
    fn auth_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key)).unwrap(),
        );
        headers
    }
    
    fn http_client(&self) -> &Client { &self.client }
}
```

### Option B: Custom Format

If your provider uses a unique protocol:

```rust
use claw_provider::MessageFormat;
use serde::{Serialize, Deserialize};

// Define format-specific types
#[derive(Serialize)]
pub struct CustomRequest { /* ... */ }

#[derive(Deserialize)]
pub struct CustomResponse { /* ... */ }

pub struct CustomFormat;

impl MessageFormat for CustomFormat {
    type Request = CustomRequest;
    type Response = CustomResponse;
    type StreamChunk = CustomStreamChunk;
    
    fn build_request(messages: &[Message], opts: &Options) -> Self::Request {
        // Convert messages to your format
    }
    
    fn parse_response(raw: Self::Response) -> CompletionResponse {
        // Convert response to unified format
    }
    
    fn parse_stream_chunk(chunk: Self::StreamChunk) -> Option<Delta> {
        // Parse SSE chunk
    }
    
    fn token_count(messages: &[Message]) -> usize {
        // Your token counting logic
    }
    
    fn endpoint() -> &'static str { "/v1/chat/completions" }
}
```

---

## Features

```toml
[features]
default = ["openai", "anthropic"]

# OpenAI-compatible providers (all use same OpenAIFormat)
openai = []
deepseek = ["openai"]  # Reuses OpenAIFormat
moonshot = ["openai"]  # Reuses OpenAIFormat
qwen = ["openai"]      # Reuses OpenAIFormat
grok = ["openai"]      # Reuses OpenAIFormat
azure = ["openai"]     # Reuses OpenAIFormat with special auth

# Anthropic-compatible providers
anthropic = []
bedrock = ["anthropic"]  # Reuses AnthropicFormat with AWS auth

# Local models
ollama = []

# All providers
full = ["openai", "deepseek", "moonshot", "qwen", "grok", "azure", 
        "anthropic", "bedrock", "ollama"]
```

---

## Streaming

```rust
let mut stream = provider.stream_complete(&messages, &opts).await?;

while let Some(delta) = stream.next().await {
    print!("{}", delta.content);
    io::stdout().flush()?;
}
```

---

## Error Handling

```rust
use claw_provider::ProviderError;

match provider.complete(&messages, &opts).await {
    Ok(response) => response,
    Err(ProviderError::RateLimit { retry_after }) => {
        tokio::time::sleep(retry_after).await;
        // Retry
    }
    Err(ProviderError::Auth) => {
        // Check API key
    }
    Err(ProviderError::Format(e)) => {
        // MessageFormat parsing error
    }
    Err(e) => return Err(e.into()),
}
```

---
