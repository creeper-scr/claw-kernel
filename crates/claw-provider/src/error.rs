//! Error types for claw-provider.
//!
//! Provides unified error handling for LLM provider operations including HTTP requests,
//! authentication, rate limiting, and response processing.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
#[non_exhaustive]
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

    #[error("invalid temperature: {0}. Must be between 0.0 and 2.0")]
    InvalidTemperature(f32),

    #[error("provider error: {0}")]
    Other(String),

    #[error("transport build failed: {0}")]
    BuildFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_error_display() {
        let err = ProviderError::Http {
            status: 500,
            message: "server error".to_string(),
        };
        assert_eq!(err.to_string(), "HTTP error: 500 — server error");

        let err = ProviderError::Network("timeout".to_string());
        assert_eq!(err.to_string(), "network error: timeout");

        let err = ProviderError::Auth("invalid key".to_string());
        assert_eq!(err.to_string(), "authentication failed: invalid key");

        let err = ProviderError::RateLimited {
            retry_after_secs: 60,
        };
        assert_eq!(err.to_string(), "rate limited, retry after 60s");

        let err = ProviderError::InvalidRequest("bad format".to_string());
        assert_eq!(err.to_string(), "invalid request: bad format");

        let err = ProviderError::Serialization("json error".to_string());
        assert_eq!(err.to_string(), "serialization error: json error");

        let err = ProviderError::Stream("broken".to_string());
        assert_eq!(err.to_string(), "stream error: broken");

        let err = ProviderError::ModelNotFound("gpt-5".to_string());
        assert_eq!(err.to_string(), "model not found: gpt-5");

        let err = ProviderError::ContextLengthExceeded;
        assert_eq!(err.to_string(), "context length exceeded");

        let err = ProviderError::InvalidTemperature(3.0);
        assert_eq!(
            err.to_string(),
            "invalid temperature: 3. Must be between 0.0 and 2.0"
        );

        let err = ProviderError::Other("unknown".to_string());
        assert_eq!(err.to_string(), "provider error: unknown");
    }

    #[test]
    fn test_provider_error_clone() {
        let err = ProviderError::Network("timeout".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }
}
