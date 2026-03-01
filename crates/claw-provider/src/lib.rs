//! LLM provider traits and implementations.

pub mod error;
pub mod traits;
pub mod types;

pub use error::ProviderError;
pub use traits::{EmbeddingProvider, HttpTransport, LLMProvider, MessageFormat};
pub use types::{
    CompletionResponse, Delta, Embedding, FinishReason, Message, Options, Role, ToolCall,
    ToolCallResult, TokenUsage,
};
