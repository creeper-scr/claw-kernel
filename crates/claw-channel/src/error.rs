use thiserror::Error;

#[derive(Debug, Error)]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_channel_error_display() {
        let e = ChannelError::ConnectionFailed("timeout".to_string());
        assert!(e.to_string().contains("timeout"));
    }
    #[test]
    fn test_rate_limited_display() {
        let e = ChannelError::RateLimited;
        assert_eq!(e.to_string(), "rate limited");
    }
}
