---
title: claw-provider
description: LLM provider trait + Anthropic/OpenAI/Ollama implementations
status: implemented
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
    /// POST JSON and return the full response body.
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError>;

    /// POST and return a raw byte stream (for SSE / NDJSON responses).
    async fn post_stream(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<bytes::Bytes, ProviderError>> + Send>>,
        ProviderError,
    >;
}

/// Extension trait for HttpTransport providing generic request methods.
pub trait HttpTransportExt: HttpTransport {
    fn base_url(&self) -> &str;
    fn auth_headers(&self) -> reqwest::header::HeaderMap;
    fn http_client(&self) -> &reqwest::Client;
    
    /// Generic request using MessageFormat — reused by ALL providers.
    fn request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> impl Future<Output = Result<CompletionResponse, ProviderError>> + Send;
    
    /// Generic streaming request.
    fn stream_request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> impl Future<Output = Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError>> + Send;
}
```

### Tier 3: Provider (Configuration Only)

```rust
pub struct DeepSeekProvider {
    transport: DefaultHttpTransport,
    model: String,
}

#[async_trait]
impl LLMProvider for DeepSeekProvider {
    fn provider_id(&self) -> &str { "deepseek" }
    fn model_id(&self) -> &str { &self.model }
    
    async fn complete(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<CompletionResponse, ProviderError> {
        self.request::<OpenAIFormat>(&messages, &options).await
    }
    
    async fn complete_stream(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError> {
        self.stream_request::<OpenAIFormat>(&messages, &options).await
    }
}

impl HttpTransportExt for DeepSeekProvider {
    fn base_url(&self) -> &str { self.transport.base_url() }
    fn auth_headers(&self) -> HeaderMap { self.transport.auth_headers() }
    fn http_client(&self) -> &Client { self.transport.http_client() }
}

impl HttpTransport for DeepSeekProvider {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        self.transport.post_json(url, headers, body).await
    }

    async fn post_stream(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<bytes::Bytes, ProviderError>> + Send>>, ProviderError> {
        self.transport.post_stream(url, headers, body).await
    }
}
```

---

## Custom Provider

### Option A: Use Existing Format (Recommended)

If your provider uses OpenAI or Anthropic compatible format:

```rust
use claw_provider::{OpenAIFormat, HttpTransport, HttpTransportExt, LLMProvider, Message, CompletionResponse};
use async_trait::async_trait;

pub struct MyProvider {
    transport: DefaultHttpTransport,
    model: String,
}

#[async_trait]
impl LLMProvider for MyProvider {
    fn provider_id(&self) -> &str { "my-provider" }
    fn model_id(&self) -> &str { &self.model }
    
    async fn complete(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<CompletionResponse, ProviderError> {
        self.request::<OpenAIFormat>(&messages, &options).await
    }
    
    async fn complete_stream(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError> {
        self.stream_request::<OpenAIFormat>(&messages, &options).await
    }
}

impl HttpTransportExt for MyProvider {
    fn base_url(&self) -> &str { self.transport.base_url() }
    fn auth_headers(&self) -> HeaderMap { self.transport.auth_headers() }
    fn http_client(&self) -> &Client { self.transport.http_client() }
}

impl HttpTransport for MyProvider {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        self.transport.post_json(url, headers, body).await
    }

    async fn post_stream(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<bytes::Bytes, ProviderError>> + Send>>, ProviderError> {
        self.transport.post_stream(url, headers, body).await
    }
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
