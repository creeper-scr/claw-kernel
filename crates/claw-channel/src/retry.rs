//! Retry wrapper for channel adapters.
//!
//! Provides [`RetryableChannel`], a transparent wrapper that adds exponential-backoff
//! retry semantics to any [`Channel`] implementation without modifying the
//! underlying adapter.
//!
//! # Retry Policy
//!
//! Only transient errors are retried.  The following [`ChannelError`] variants
//! are considered **permanent** and surface immediately without retrying:
//!
//! - [`ChannelError::AuthFailed`] — credentials won't become valid on retry
//! - [`ChannelError::InvalidConfig`] — a configuration bug cannot heal itself
//!
//! All other variants (network failures, rate limiting, etc.) are retried up
//! to `retry_config.max_retries` additional times with full-jitter exponential
//! backoff.
//!
//! # Example
//!
//! ```rust,ignore
//! use claw_channel::{RetryableChannel, StdinChannel, ChannelId};
//! use claw_pal::retry::RetryConfig;
//! use std::time::Duration;
//!
//! let inner = StdinChannel::new(ChannelId::new("cli"));
//! let channel = RetryableChannel::new(inner, RetryConfig::default());
//! ```

use async_trait::async_trait;
use claw_pal::retry::{with_retry_mapped, RetryConfig};

use crate::{
    error::ChannelError,
    traits::Channel,
    types::{ChannelId, ChannelMessage},
};

/// Wraps any [`Channel`] to add transparent exponential-backoff retry on `send()`.
///
/// `recv()`, `connect()`, and `disconnect()` are forwarded directly to the inner
/// channel without retry, because:
///
/// - `recv()` is a long-poll that naturally blocks until a message arrives.
/// - `connect()` and `disconnect()` are lifecycle methods whose retry semantics
///   are better handled at the application level.
pub struct RetryableChannel<T: Channel> {
    inner: T,
    retry_config: RetryConfig,
}

impl<T: Channel> RetryableChannel<T> {
    /// Create a new retryable wrapper around `inner` using the given [`RetryConfig`].
    pub fn new(inner: T, retry_config: RetryConfig) -> Self {
        Self {
            inner,
            retry_config,
        }
    }

    /// Create a new retryable wrapper using the default [`RetryConfig`]
    /// (3 retries, 500 ms base delay, 30 s cap).
    pub fn with_defaults(inner: T) -> Self {
        Self::new(inner, RetryConfig::default())
    }

    /// Return a reference to the underlying channel.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Consume the wrapper and return the underlying channel.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

/// Classify which [`ChannelError`] variants are transient and worth retrying.
fn is_retryable(err: &ChannelError) -> bool {
    !matches!(
        err,
        ChannelError::AuthFailed | ChannelError::InvalidConfig(_)
    )
}

#[async_trait]
impl<T: Channel> Channel for RetryableChannel<T> {
    fn platform(&self) -> &str {
        self.inner.platform()
    }

    fn channel_id(&self) -> &ChannelId {
        self.inner.channel_id()
    }

    /// Send a message, retrying on transient failures.
    ///
    /// The message is cloned on each attempt.  If all retries are exhausted,
    /// the last error is returned.
    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError> {
        with_retry_mapped(
            || {
                let msg = message.clone();
                self.inner.send(msg)
            },
            &self.retry_config,
            is_retryable,
        )
        .await
    }

    /// Receive a message from the underlying channel (no retry applied).
    async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
        self.inner.recv().await
    }

    /// Connect to the underlying channel (no retry applied).
    async fn connect(&self) -> Result<(), ChannelError> {
        self.inner.connect().await
    }

    /// Disconnect from the underlying channel (no retry applied).
    async fn disconnect(&self) -> Result<(), ChannelError> {
        self.inner.disconnect().await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    use super::*;
    use crate::types::Platform;

    // ---------------------------------------------------------------------------
    // Minimal test double
    // ---------------------------------------------------------------------------

    struct CountingChannel {
        id: ChannelId,
        /// How many times `send()` has been called.
        send_calls: Arc<AtomicUsize>,
        /// If `Some`, return this error on calls ≤ fail_until.
        fail_until: Option<usize>,
        /// Error to return when failing.
        fail_with: Option<ChannelError>,
    }

    impl CountingChannel {
        fn new(id: &str) -> Self {
            Self {
                id: ChannelId::new(id),
                send_calls: Arc::new(AtomicUsize::new(0)),
                fail_until: None,
                fail_with: None,
            }
        }

        fn with_transient_failures(mut self, count: usize) -> Self {
            self.fail_until = Some(count);
            self.fail_with = Some(ChannelError::SendFailed("transient".into()));
            self
        }

        fn with_permanent_failure(mut self, err: ChannelError) -> Self {
            self.fail_until = Some(usize::MAX);
            self.fail_with = Some(err);
            self
        }

        fn send_calls(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.send_calls)
        }
    }

    #[async_trait::async_trait]
    impl Channel for CountingChannel {
        fn platform(&self) -> &str {
            "test"
        }
        fn channel_id(&self) -> &ChannelId {
            &self.id
        }
        async fn send(&self, _msg: ChannelMessage) -> Result<(), ChannelError> {
            let n = self.send_calls.fetch_add(1, Ordering::SeqCst);
            match (&self.fail_until, &self.fail_with) {
                (Some(limit), Some(err)) if n < *limit => Err(err.clone()),
                _ => Ok(()),
            }
        }
        async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
            Ok(ChannelMessage::inbound(
                self.id.clone(),
                Platform::Stdin,
                "hello",
            ))
        }
        async fn connect(&self) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn disconnect(&self) -> Result<(), ChannelError> {
            Ok(())
        }
    }

    fn fast_config() -> RetryConfig {
        RetryConfig::default()
            .with_max_retries(3)
            .with_base_delay(Duration::from_millis(1))
            .with_max_delay(Duration::from_millis(5))
    }

    // ---------------------------------------------------------------------------
    // Tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_send_succeeds_on_first_attempt() {
        let inner = CountingChannel::new("ch");
        let calls = inner.send_calls();
        let ch = RetryableChannel::new(inner, fast_config());
        let msg = ChannelMessage::outbound(ChannelId::new("ch"), Platform::Stdin, "hi");

        ch.send(msg).await.unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_send_retries_transient_failures() {
        // Fail the first 2 attempts, succeed on the 3rd.
        let inner = CountingChannel::new("ch").with_transient_failures(2);
        let calls = inner.send_calls();
        let ch = RetryableChannel::new(inner, fast_config());
        let msg = ChannelMessage::outbound(ChannelId::new("ch"), Platform::Stdin, "hi");

        ch.send(msg).await.unwrap();

        // 3 total calls: 2 failures + 1 success
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_send_exhausts_retries_and_returns_error() {
        // Always fail — exceed max_retries (3).
        let inner =
            CountingChannel::new("ch").with_permanent_failure(ChannelError::RateLimited);
        let calls = inner.send_calls();
        let ch = RetryableChannel::new(inner, fast_config());
        let msg = ChannelMessage::outbound(ChannelId::new("ch"), Platform::Stdin, "hi");

        let err = ch.send(msg).await.unwrap_err();

        assert_eq!(err, ChannelError::RateLimited);
        // 1 initial + 3 retries = 4 total calls
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn test_send_does_not_retry_auth_failed() {
        let inner =
            CountingChannel::new("ch").with_permanent_failure(ChannelError::AuthFailed);
        let calls = inner.send_calls();
        let ch = RetryableChannel::new(inner, fast_config());
        let msg = ChannelMessage::outbound(ChannelId::new("ch"), Platform::Stdin, "hi");

        let err = ch.send(msg).await.unwrap_err();

        assert_eq!(err, ChannelError::AuthFailed);
        // Must not retry on permanent errors
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_send_does_not_retry_invalid_config() {
        let inner = CountingChannel::new("ch")
            .with_permanent_failure(ChannelError::InvalidConfig("bad url".into()));
        let calls = inner.send_calls();
        let ch = RetryableChannel::new(inner, fast_config());
        let msg = ChannelMessage::outbound(ChannelId::new("ch"), Platform::Stdin, "hi");

        let err = ch.send(msg).await.unwrap_err();

        assert!(matches!(err, ChannelError::InvalidConfig(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_platform_and_channel_id_forwarded() {
        let ch = RetryableChannel::with_defaults(CountingChannel::new("my-channel"));
        assert_eq!(ch.platform(), "test");
        assert_eq!(ch.channel_id().as_str(), "my-channel");
    }

    #[tokio::test]
    async fn test_recv_forwarded() {
        let ch = RetryableChannel::with_defaults(CountingChannel::new("ch"));
        let msg = ch.recv().await.unwrap();
        assert_eq!(msg.content, "hello");
    }

    #[tokio::test]
    async fn test_connect_disconnect_forwarded() {
        let ch = RetryableChannel::with_defaults(CountingChannel::new("ch"));
        ch.connect().await.unwrap();
        ch.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn test_into_inner() {
        let inner = CountingChannel::new("ch");
        let wrapper = RetryableChannel::with_defaults(inner);
        let recovered = wrapper.into_inner();
        assert_eq!(recovered.channel_id().as_str(), "ch");
    }
}
