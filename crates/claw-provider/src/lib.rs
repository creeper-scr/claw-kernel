//! LLM provider traits and implementations.

pub mod anthropic;
pub mod deepseek;
pub mod embedding;
pub mod error;
pub mod moonshot;
pub mod ollama;
pub mod openai;
pub mod providers;
pub mod retry;
pub mod streaming;
pub mod traits;
pub mod transport;
pub mod types;

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
