//! Error types for claw-memory.
//!
//! Provides unified error handling for memory operations including storage,
//! embedding generation, quota management, and access control.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_error_display() {
        let err = MemoryError::NotFound("item1".to_string());
        assert_eq!(err.to_string(), "item not found: item1");

        let err = MemoryError::Storage("disk full".to_string());
        assert_eq!(err.to_string(), "storage error: disk full");

        let err = MemoryError::Embedding("model error".to_string());
        assert_eq!(err.to_string(), "embedding error: model error");

        let err = MemoryError::QuotaExceeded {
            namespace: "user1".to_string(),
            used: 1000000,
            limit: 1000000,
        };
        assert!(err.to_string().contains("quota exceeded"));
        assert!(err.to_string().contains("user1"));

        let err = MemoryError::AccessDenied("readonly".to_string());
        assert_eq!(err.to_string(), "access denied: readonly");

        let err = MemoryError::Serialization("json error".to_string());
        assert_eq!(err.to_string(), "serialization error: json error");
    }

    #[test]
    fn test_memory_error_clone() {
        let err = MemoryError::NotFound("item1".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }
}
