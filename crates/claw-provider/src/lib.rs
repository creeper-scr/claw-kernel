//! LLM provider traits and implementations for claw-kernel.
//!
//! This crate provides a unified interface for interacting with various LLM providers
//! including Anthropic, OpenAI, Ollama, DeepSeek, and Moonshot.
//!
//! # Main Types
//!
//! - [`LLMProvider`] - The main trait for LLM providers
//! - [`EmbeddingProvider`] - Trait for embedding providers
//! - [`Message`], [`Role`], [`Options`] - Core types for completions
//! - [`CompletionResponse`], [`Delta`] - Response types
//! - [`AnthropicProvider`], [`OpenAIProvider`], [`OllamaProvider`] - Provider implementations
//!
//! # Example
//!
//! ```rust,no_run
//! use claw_provider::{LLMProvider, OllamaProvider};
//! use claw_provider::types::{Message, Options};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a provider from environment
//!     let provider = OllamaProvider::from_env()?;
//!
//!     // Prepare messages and options
//!     let messages = vec![
//!         Message::system("You are a helpful assistant."),
//!         Message::user("What is Rust?"),
//!     ];
//!     let options = Options::new("llama3.2:latest").with_max_tokens(1024);
//!
//!     // Get completion
//!     let response = provider.complete(messages, options).await?;
//!     println!("{}", response.message.content);
//!     Ok(())
//! }
//! ```

// Provider 实现模块 — 各自受特性标志门控。
// deepseek/moonshot 内部使用 openai::format，因此依赖 "openai" 特性。
#[cfg(feature = "anthropic")]
mod anthropic;
#[cfg(feature = "deepseek")]
mod deepseek;
#[cfg(feature = "moonshot")]
mod moonshot;
#[cfg(feature = "ollama")]
mod ollama;
#[cfg(feature = "openai")]
mod openai;

// 新增 provider 模块 — 基于 openai 特性构建，各自受独立 feature 门控。
#[cfg(feature = "gemini")]
pub mod gemini;
#[cfg(feature = "mistral")]
pub mod mistral;
#[cfg(feature = "azure-openai")]
pub mod azure_openai;

// 其他公共模块（traits, types, transport 等）保持公开
pub mod embedding;
pub mod error;
pub mod providers;
pub mod retry;
pub mod streaming;
pub mod traits;
pub mod transport;
pub mod types;
pub(crate) mod stream_utils;

// 从私有模块重新导出 Provider 类型，保持公共 API 可用
#[cfg(feature = "anthropic")]
pub use anthropic::AnthropicProvider;
#[cfg(feature = "deepseek")]
pub use deepseek::DeepSeekProvider;
pub use embedding::NgramEmbeddingProvider;
pub use error::ProviderError;
#[cfg(feature = "moonshot")]
pub use moonshot::MoonshotProvider;
#[cfg(feature = "ollama")]
pub use ollama::OllamaProvider;
#[cfg(feature = "openai")]
pub use openai::OpenAIProvider;
pub use providers::provider_from_env;
pub use retry::{with_retry, RetryConfig};
pub use streaming::{parse_ndjson_line, parse_sse_event, BoxStream, StreamChunk};
pub use traits::{EmbeddingProvider, HttpTransport, LLMProvider, MessageFormat, ProviderEventPublisher, NoopProviderEventPublisher, ProviderHealth};
pub use transport::DefaultHttpTransport;
pub use types::{
    CompletionResponse, Delta, Embedding, FinishReason, Message, Options, Role, TokenUsage,
    ToolCall, ToolCallResult, ToolDef,
};

// 便捷重导出新 provider 工厂函数
#[cfg(feature = "gemini")]
pub use gemini::gemini_provider;
#[cfg(feature = "gemini")]
pub use gemini::gemini_provider_from_env;
#[cfg(feature = "mistral")]
pub use mistral::mistral_provider;
#[cfg(feature = "mistral")]
pub use mistral::mistral_provider_from_env;
#[cfg(feature = "azure-openai")]
pub use azure_openai::azure_openai_provider;
#[cfg(feature = "azure-openai")]
pub use azure_openai::azure_openai_provider_from_env;
