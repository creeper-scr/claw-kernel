---
title: claw-provider
description: LLM provider trait + Anthropic/OpenAI/Ollama implementations
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](claw-provider.md)


# claw-provider

> **Layer 2: Agent Kernel Protocol** — LLM Provider 抽象  
> 智能体内核协议层 (Layer 2) — LLM Provider 抽象

LLM Provider 抽象层，采用三层内部架构：消息格式 → HTTP 传输 → Provider 配置。

---

## 概述

`claw-provider` 使用**协议格式抽象**设计来最大化代码复用。我们不再为每个 provider 实现 HTTP 逻辑，而是按照**消息格式兼容性**进行抽象：

```
┌─────────────────────────────────────────────────────────┐
│  第 3 层（内部 Tier 3）：Provider 配置                   │
│  AnthropicProvider, DeepSeekProvider, MoonshotProvider  │
│  （仅配置 base_url + 认证）                              │
├─────────────────────────────────────────────────────────┤
│  第 2 层（内部 Tier 2）：HTTP 传输（可复用）             │
│  HttpTransport trait，包含请求/流式逻辑                  │
├─────────────────────────────────────────────────────────┤
│  第 1 层（内部 Tier 1）：消息格式（协议抽象）            │
│  AnthropicFormat, OpenAIFormat                          │
│  （请求/响应序列化 + Token 计数）                        │
└─────────────────────────────────────────────────────────┘
```

> **注意：** 这里的"第 1/2/3 层"指 claw-provider 的内部架构（内核 Layer 2），
> 与内核五层架构（Layer 0-3）不同。

### 为什么使用格式抽象？

| Provider | 使用格式 | 代码行数 |
|----------|----------|----------|
| OpenAI | OpenAIFormat | ~20（仅配置） |
| DeepSeek | OpenAIFormat | ~20（仅配置） |
| Moonshot | OpenAIFormat | ~20（仅配置） |
| Qwen | OpenAIFormat | ~20（仅配置） |
| Anthropic | AnthropicFormat | ~20（仅配置） |
| Bedrock | AnthropicFormat | ~30（AWS 认证） |

> 市面上 90% 的 LLM provider 都兼容 OpenAI 或 Anthropic 格式。这种架构消除了重复的 HTTP 和序列化代码。

---

## 用法

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
        content: "你好！".to_string(),
    }
], &Default::default()).await?;

println!("{}", response.content);
```

---

## 内置 Providers

### Anthropic 兼容 Providers

```rust
use claw_provider::AnthropicProvider;

// Anthropic (Claude)
let provider = AnthropicProvider::from_env()?; // ANTHROPIC_API_KEY

// AWS Bedrock (通过 AWS 使用 Claude)
use claw_provider::BedrockProvider;
let provider = BedrockProvider::new(BedrockConfig {
    region: "us-east-1".to_string(),
    model: "anthropic.claude-3-opus-20240229-v1:0".to_string(),
});
```

### OpenAI 兼容 Providers

所有 OpenAI 兼容的 providers 共享相同的协议实现，仅 base URL 和认证方式不同。

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

// Azure OpenAI（特殊认证处理）
let provider = AzureOpenAIProvider::from_env()?; // AZURE_OPENAI_API_KEY
```

### 本地模型

```rust
use claw_provider::OllamaProvider;

let provider = OllamaProvider::new(OllamaConfig {
    base_url: "http://localhost:11434".to_string(),
    model: "llama2".to_string(),
});
```

---

## 架构设计

### 第 1 层（Tier 1）：MessageFormat（协议抽象）

```rust
#[async_trait]
pub trait MessageFormat: Send + Sync {
    type Request: Serialize;
    type Response: DeserializeOwned;
    type StreamChunk: DeserializeOwned;
    type Error: std::error::Error;
    
    /// 将统一 Message 转换为格式特定请求
    fn build_request(messages: &[Message], opts: &Options) -> Self::Request;
    
    /// 解析响应为统一格式
    fn parse_response(raw: Self::Response) -> Result<CompletionResponse, Self::Error>;
    
    /// 解析流式 chunk
    fn parse_stream_chunk(chunk: &[u8]) -> Result<Option<Delta>, Self::Error>;
    
    /// Token 计数（格式特定算法）
    fn token_count(messages: &[Message]) -> usize;
    
    /// API 端点路径
    fn endpoint() -> &'static str;
}
```

**内置格式：**
- `OpenAIFormat` — OpenAI、DeepSeek、Moonshot、Qwen、Grok 及大多数云服务商使用
- `AnthropicFormat` — Anthropic (Claude) 和 AWS Bedrock 使用
- `OllamaFormat` — Ollama 的 OpenAI 兼容变体

### 第 2 层（Tier 2）：HttpTransport（可复用）

```rust
#[async_trait]
pub trait HttpTransport: Send + Sync {
    fn base_url(&self) -> &str;
    fn auth_headers(&self) -> HeaderMap;
    fn http_client(&self) -> &Client;
    
    /// 通用请求 — 被所有 providers 复用
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
    
    /// 通用流式请求
    async fn stream_request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
}
```

### 第 3 层（Tier 3）：Provider（仅配置）

```rust
pub struct DeepSeekProvider {
    api_key: String,
    model: String,
    client: Client,
}

#[async_trait]
impl LLMProvider for DeepSeekProvider {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError> {
        // 简单地委托给 HttpTransport 并使用 OpenAIFormat
        self.request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn stream_complete(&self, messages: &[Message], opts: &Options) 
        -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError> {
        self.stream_request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, ProviderError> {
        todo!("Implement embedding")
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

## 自定义 Provider

### 方案 A：使用现有格式（推荐）

如果你的 provider 使用 OpenAI 或 Anthropic 兼容格式：

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
        todo!("实现嵌入功能")
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

### 方案 B：自定义格式

如果你的 provider 使用独特的协议：

```rust
use claw_provider::MessageFormat;
use serde::{Serialize, Deserialize};

// 定义格式特定类型
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
        // 转换消息为你的格式
    }
    
    fn parse_response(raw: Self::Response) -> CompletionResponse {
        // 转换响应为统一格式
    }
    
    fn parse_stream_chunk(chunk: Self::StreamChunk) -> Option<Delta> {
        // 解析 SSE chunk
    }
    
    fn token_count(messages: &[Message]) -> usize {
        // 你的 token 计数逻辑
    }
    
    fn endpoint() -> &'static str { "/v1/chat/completions" }
}
```

---

## 特性开关

```toml
[features]
default = ["openai", "anthropic"]

# OpenAI 兼容 providers（都使用相同的 OpenAIFormat）
openai = []
deepseek = ["openai"]  # 复用 OpenAIFormat
moonshot = ["openai"]  # 复用 OpenAIFormat
qwen = ["openai"]      # 复用 OpenAIFormat
grok = ["openai"]      # 复用 OpenAIFormat
azure = ["openai"]     # 复用 OpenAIFormat，特殊认证

# Anthropic 兼容 providers
anthropic = []
bedrock = ["anthropic"]  # 复用 AnthropicFormat，AWS 认证

# 本地模型
ollama = []

# 所有 providers
full = ["openai", "deepseek", "moonshot", "qwen", "grok", "azure", 
        "anthropic", "bedrock", "ollama"]
```

---

## 流式处理

```rust
let mut stream = provider.stream_complete(&messages, &opts).await?;

while let Some(delta) = stream.next().await {
    print!("{}", delta.content);
    io::stdout().flush()?;
}
```

---

## 错误处理

```rust
use claw_provider::ProviderError;

match provider.complete(&messages, &opts).await {
    Ok(response) => response,
    Err(ProviderError::RateLimit { retry_after }) => {
        tokio::time::sleep(retry_after).await;
        // 重试
    }
    Err(ProviderError::Auth) => {
        // 检查 API key
    }
    Err(ProviderError::Format(e)) => {
        // MessageFormat 解析错误
    }
    Err(e) => return Err(e.into()),
}
```
