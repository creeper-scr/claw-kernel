//! Error types for claw-channel.
//!
//! Provides unified error handling for channel operations including connection
//! management, message sending/receiving, and authentication.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ChannelError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("send failed: {0}")]
    SendFailed(String),
    #[error("receive failed: {0}")]
    ReceiveFailed(String),
    #[error("authentication failed")]
    AuthFailed,
    #[error("channel not found: {0}")]
    NotFound(String),
    #[error("rate limited")]
    RateLimited,
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    /// The underlying transport dropped the connection mid-receive.
    ///
    /// This is distinct from [`ConnectionFailed`](Self::ConnectionFailed) (a
    /// failed *connect attempt*): `Disconnected` signals that a previously
    /// healthy connection was lost while waiting for the next message.
    /// [`RetryableChannel`](crate::RetryableChannel) treats this as transient
    /// and will attempt to reconnect before retrying `recv()`.
    #[error("channel disconnected")]
    Disconnected,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_error_display() {
        let e = ChannelError::ConnectionFailed("timeout".to_string());
        assert!(e.to_string().contains("timeout"));

        let e = ChannelError::SendFailed("broken pipe".to_string());
        assert_eq!(e.to_string(), "send failed: broken pipe");

        let e = ChannelError::ReceiveFailed("eof".to_string());
        assert_eq!(e.to_string(), "receive failed: eof");

        let e = ChannelError::AuthFailed;
        assert_eq!(e.to_string(), "authentication failed");

        let e = ChannelError::NotFound("channel1".to_string());
        assert_eq!(e.to_string(), "channel not found: channel1");

        let e = ChannelError::RateLimited;
        assert_eq!(e.to_string(), "rate limited");
    }

    #[test]
    fn test_channel_error_clone() {
        let err = ChannelError::ConnectionFailed("timeout".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }
}
