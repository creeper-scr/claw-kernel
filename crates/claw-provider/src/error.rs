use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("HTTP error: {status} — {message}")]
    Http { status: u16, message: String },

    #[error("network error: {0}")]
    Network(String),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("rate limited, retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("stream error: {0}")]
    Stream(String),

    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("context length exceeded")]
    ContextLengthExceeded,

    #[error("provider error: {0}")]
    Other(String),
}
