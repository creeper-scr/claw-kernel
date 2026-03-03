use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum MemoryError {
    #[error("item not found: {0}")]
    NotFound(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("quota exceeded: namespace '{namespace}' used {used} bytes of {limit} byte limit")]
    QuotaExceeded {
        namespace: String,
        used: u64,
        limit: u64,
    },

    #[error("access denied: {0}")]
    AccessDenied(String),

    #[error("serialization error: {0}")]
    Serialization(String),
}
