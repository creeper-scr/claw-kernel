//! LLM provider traits and implementations.

// Provider 实现模块 - 改为私有，隐藏实现细节
mod anthropic;
mod deepseek;
mod moonshot;
mod ollama;
mod openai;

// 其他公共模块（traits, types, transport 等）保持公开
pub mod embedding;
pub mod error;
pub mod providers;
pub mod retry;
pub mod streaming;
pub mod traits;
pub mod transport;
pub mod types;

// 从私有模块重新导出 Provider 类型，保持公共 API 可用
pub use anthropic::AnthropicProvider;
pub use deepseek::DeepSeekProvider;
pub use embedding::NgramEmbeddingProvider;
pub use error::ProviderError;
pub use moonshot::MoonshotProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAIProvider;
pub use providers::provider_from_env;
pub use retry::{with_retry, RetryConfig};
pub use streaming::{parse_ndjson_line, parse_sse_event, BoxStream, StreamChunk};
pub use traits::{EmbeddingProvider, HttpTransport, LLMProvider, MessageFormat};
pub use transport::DefaultHttpTransport;
pub use types::{
    CompletionResponse, Delta, Embedding, FinishReason, Message, Options, Role, TokenUsage,
    ToolCall, ToolCallResult,
};
