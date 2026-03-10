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

/// Wraps any [`Channel`] to add transparent exponential-backoff retry on both
/// `send()` and `recv()`.
///
/// ## `send()` retry
///
/// The message is cloned on each attempt.  Transient errors are retried up to
/// `retry_config.max_retries` additional times with full-jitter exponential
/// backoff.
///
/// ## `recv()` reconnect-retry
///
/// Network channels (WebSocket, Discord, …) can lose their connection while
/// waiting for the next message.  When `recv()` returns a retryable error the
/// wrapper:
///
/// 1. Sleeps for the computed backoff duration.
/// 2. Calls `inner.connect()` to re-establish the connection (best-effort; a
///    connect failure does not consume an extra retry slot).
/// 3. Re-issues `inner.recv()`.
///
/// The sequence repeats up to `retry_config.max_retries` additional times.
///
/// ## Permanent errors
///
/// [`ChannelError::AuthFailed`] and [`ChannelError::InvalidConfig`] are
/// considered permanent and surface immediately on both `send()` and `recv()`
/// without retrying.
///
/// `connect()` and `disconnect()` are forwarded directly to the inner channel
/// without retry, as their retry semantics are better handled at the call site.
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

    /// Receive the next message, reconnecting on transient failures.
    ///
    /// When `recv()` returns a retryable error the wrapper:
    ///
    /// 1. Sleeps for the computed backoff duration.
    /// 2. Calls `inner.connect()` (best-effort reconnect; a connect failure
    ///    does not consume an extra retry slot).
    /// 3. Re-issues `inner.recv()`.
    ///
    /// [`ChannelError::AuthFailed`] and [`ChannelError::InvalidConfig`] are
    /// permanent and surface immediately without retrying.
    async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
        for attempt in 0..=self.retry_config.max_retries {
            match self.inner.recv().await {
                Ok(msg) => return Ok(msg),
                Err(e) if !is_retryable(&e) => return Err(e),
                Err(e) => {
                    if attempt == self.retry_config.max_retries {
                        return Err(e);
                    }
                    let delay = self.retry_config.calculate_delay(attempt);
                    tracing::warn!(
                        attempt,
                        delay_ms = delay.as_millis(),
                        error = %e,
                        "recv transient error — reconnecting and retrying",
                    );
                    tokio::time::sleep(delay).await;
                    // Best-effort reconnect; ignore connect errors so the
                    // retry budget is spent on recv attempts only.
                    let _ = self.inner.connect().await;
                }
            }
        }
        // The loop always returns via one of the branches above.
        unreachable!("retry loop must always return")
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
        /// How many times `recv()` has been called.
        recv_calls: Arc<AtomicUsize>,
        /// How many times `connect()` has been called.
        connect_calls: Arc<AtomicUsize>,
        /// If `Some`, return this error on send calls ≤ fail_until.
        fail_until: Option<usize>,
        /// Error to return when failing sends.
        fail_with: Option<ChannelError>,
        /// If `Some`, return this error on recv calls ≤ recv_fail_until.
        recv_fail_until: Option<usize>,
        /// Error to return when failing recvs.
        recv_fail_with: Option<ChannelError>,
    }

    impl CountingChannel {
        fn new(id: &str) -> Self {
            Self {
                id: ChannelId::new(id),
                send_calls: Arc::new(AtomicUsize::new(0)),
                recv_calls: Arc::new(AtomicUsize::new(0)),
                connect_calls: Arc::new(AtomicUsize::new(0)),
                fail_until: None,
                fail_with: None,
                recv_fail_until: None,
                recv_fail_with: None,
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

        /// Fail the first `count` `recv()` calls with the given error.
        fn with_recv_transient_failures(mut self, count: usize, err: ChannelError) -> Self {
            self.recv_fail_until = Some(count);
            self.recv_fail_with = Some(err);
            self
        }

        /// Fail all `recv()` calls with the given error.
        fn with_recv_permanent_failure(mut self, err: ChannelError) -> Self {
            self.recv_fail_until = Some(usize::MAX);
            self.recv_fail_with = Some(err);
            self
        }

        fn send_calls(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.send_calls)
        }

        fn recv_calls(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.recv_calls)
        }

        fn connect_calls(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.connect_calls)
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
            let n = self.recv_calls.fetch_add(1, Ordering::SeqCst);
            match (&self.recv_fail_until, &self.recv_fail_with) {
                (Some(limit), Some(err)) if n < *limit => Err(err.clone()),
                _ => Ok(ChannelMessage::inbound(
                    self.id.clone(),
                    Platform::Stdin,
                    "hello",
                )),
            }
        }
        async fn connect(&self) -> Result<(), ChannelError> {
            self.connect_calls.fetch_add(1, Ordering::SeqCst);
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

    // ── recv retry tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_recv_succeeds_without_retry() {
        let ch = RetryableChannel::new(CountingChannel::new("ch"), fast_config());
        let msg = ch.recv().await.unwrap();
        assert_eq!(msg.content, "hello");
    }

    #[tokio::test]
    async fn test_recv_retries_disconnected_error_then_succeeds() {
        // First 2 recv() calls return Disconnected; 3rd succeeds.
        let inner = CountingChannel::new("ch")
            .with_recv_transient_failures(2, ChannelError::Disconnected);
        let recv_calls = inner.recv_calls();
        let connect_calls = inner.connect_calls();
        let ch = RetryableChannel::new(inner, fast_config());

        let msg = ch.recv().await.unwrap();

        assert_eq!(msg.content, "hello");
        // 3 total recv() calls: 2 failures + 1 success.
        assert_eq!(recv_calls.load(Ordering::SeqCst), 3);
        // connect() called once per failure (2 reconnect attempts).
        assert_eq!(connect_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_recv_exhausts_retries_and_returns_error() {
        // recv() always returns a transient error — exhaust max_retries (3).
        let inner = CountingChannel::new("ch")
            .with_recv_permanent_failure(ChannelError::ReceiveFailed("timeout".into()));
        let recv_calls = inner.recv_calls();
        let ch = RetryableChannel::new(inner, fast_config());

        let err = ch.recv().await.unwrap_err();

        assert!(matches!(err, ChannelError::ReceiveFailed(_)));
        // 1 initial + 3 retries = 4 total calls.
        assert_eq!(recv_calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn test_recv_does_not_retry_auth_failed() {
        let inner = CountingChannel::new("ch")
            .with_recv_permanent_failure(ChannelError::AuthFailed);
        let recv_calls = inner.recv_calls();
        let connect_calls = inner.connect_calls();
        let ch = RetryableChannel::new(inner, fast_config());

        let err = ch.recv().await.unwrap_err();

        assert_eq!(err, ChannelError::AuthFailed);
        // Must not retry permanent errors.
        assert_eq!(recv_calls.load(Ordering::SeqCst), 1);
        assert_eq!(connect_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_recv_does_not_retry_invalid_config() {
        let inner = CountingChannel::new("ch")
            .with_recv_permanent_failure(ChannelError::InvalidConfig("bad url".into()));
        let recv_calls = inner.recv_calls();
        let ch = RetryableChannel::new(inner, fast_config());

        let err = ch.recv().await.unwrap_err();

        assert!(matches!(err, ChannelError::InvalidConfig(_)));
        assert_eq!(recv_calls.load(Ordering::SeqCst), 1);
    }
}
