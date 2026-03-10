//! Error types for the webhook module.

use thiserror::Error;

/// Errors that can occur in webhook operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WebhookError {
    /// Server bind failed.
    #[error("failed to bind webhook server: {0}")]
    BindFailed(String),

    /// Server is already running.
    #[error("webhook server is already running")]
    AlreadyRunning,

    /// Server is not running.
    #[error("webhook server is not running")]
    NotRunning,

    /// Invalid configuration.
    #[error("invalid webhook configuration: {0}")]
    InvalidConfig(String),

    /// Handler not found.
    #[error("handler not found: {0}")]
    HandlerNotFound(String),

    /// Handler already exists.
    #[error("handler already exists: {0}")]
    HandlerAlreadyExists(String),

    /// HMAC verification failed.
    #[error("HMAC verification failed")]
    HmacVerificationFailed,

    /// Invalid signature format.
    #[error("invalid signature format")]
    InvalidSignature,

    /// Request body read failed.
    #[error("failed to read request body: {0}")]
    BodyReadFailed(String),

    /// Shutdown failed.
    #[error("webhook shutdown failed: {0}")]
    ShutdownFailed(String),

    /// Request body is empty (HMAC verification requires a non-empty body).
    #[error("request body is empty")]
    EmptyBody,

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_error_display() {
        let err = WebhookError::BindFailed("addr in use".to_string());
        assert_eq!(err.to_string(), "failed to bind webhook server: addr in use");

        let err = WebhookError::AlreadyRunning;
        assert_eq!(err.to_string(), "webhook server is already running");

        let err = WebhookError::NotRunning;
        assert_eq!(err.to_string(), "webhook server is not running");

        let err = WebhookError::InvalidConfig("missing port".to_string());
        assert_eq!(err.to_string(), "invalid webhook configuration: missing port");

        let err = WebhookError::HandlerNotFound("github".to_string());
        assert_eq!(err.to_string(), "handler not found: github");

        let err = WebhookError::HmacVerificationFailed;
        assert_eq!(err.to_string(), "HMAC verification failed");

        let err = WebhookError::InvalidSignature;
        assert_eq!(err.to_string(), "invalid signature format");
    }

    #[test]
    fn test_webhook_error_clone() {
        let err = WebhookError::HandlerNotFound("test".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }
}
