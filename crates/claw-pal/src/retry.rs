//! Generic retry mechanism with exponential backoff and full jitter.
//!
//! Provides a platform-agnostic retry utility for resilient network operations.
//! Used by `claw-channel` (webhook/Discord send paths) and available to any
//! other crate that needs transparent retry without pulling in provider-specific
//! error types.

use std::time::Duration;

use rand::Rng;

/// Configuration for retry behaviour.
///
/// Defaults: **3 retries**, **500 ms base delay**, **30 s cap**.
#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    /// Maximum number of *additional* attempts after the first failure.
    /// A value of `3` means up to 4 total calls (1 initial + 3 retries).
    pub max_retries: u32,
    /// Base delay for exponential backoff.
    pub base_delay: Duration,
    /// Upper bound on the computed delay (including jitter).
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryConfig {
    /// Create a config with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the maximum number of retries.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Override the base delay.
    pub fn with_base_delay(mut self, base_delay: Duration) -> Self {
        self.base_delay = base_delay;
        self
    }

    /// Override the maximum delay cap.
    pub fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    /// Compute the sleep duration for a given attempt index (0-based).
    ///
    /// Formula: `min(base * 2^attempt + uniform_jitter[0, base * 2^attempt), max_delay)`.
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let exp_ms = self
            .base_delay
            .as_millis()
            .saturating_mul(2u128.pow(attempt));
        let cap_ms = std::cmp::min(exp_ms as u64, self.max_delay.as_millis() as u64);

        let jitter = if cap_ms > 0 {
            rand::thread_rng().gen_range(0..cap_ms)
        } else {
            0
        };

        Duration::from_millis(std::cmp::min(
            cap_ms.saturating_add(jitter),
            self.max_delay.as_millis() as u64,
        ))
    }
}

/// Execute an async operation with exponential-backoff retry.
///
/// - `operation` is called up to `config.max_retries + 1` times.
/// - `is_retryable(err)` decides whether a given error warrants a retry.
///   Return `false` to surface the error immediately (e.g. for 4xx HTTP
///   status codes that indicate a caller bug rather than a transient fault).
/// - Delays between attempts follow full-jitter exponential backoff capped at
///   `config.max_delay`.
///
/// # Example
///
/// ```rust,no_run
/// use std::time::Duration;
/// use claw_pal::retry::{RetryConfig, with_retry_mapped};
///
/// async fn example() -> Result<&'static str, String> {
///     let config = RetryConfig::new().with_base_delay(Duration::from_millis(100));
///     with_retry_mapped(
///         || async { Ok::<_, String>("data") },
///         &config,
///         |_| true,
///     ).await
/// }
/// ```
pub async fn with_retry_mapped<F, Fut, T, E, M>(
    operation: F,
    config: &RetryConfig,
    is_retryable: M,
) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    M: Fn(&E) -> bool,
{
    let mut last_error: Option<E> = None;

    for attempt in 0..=config.max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                if attempt < config.max_retries && is_retryable(&error) {
                    let delay = config.calculate_delay(attempt);
                    tracing::debug!(
                        attempt,
                        delay_ms = delay.as_millis(),
                        "transient error — will retry"
                    );
                    tokio::time::sleep(delay).await;
                    last_error = Some(error);
                } else {
                    // Non-retryable error or retries exhausted — return immediately.
                    return Err(error);
                }
            }
        }
    }

    // Reached only when the last retry attempt itself set last_error then the
    // loop exits; the earlier `return Err(error)` handles all other paths.
    Err(last_error.expect("loop executes at least once"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn test_retry_config_default() {
        let c = RetryConfig::default();
        assert_eq!(c.max_retries, 3);
        assert_eq!(c.base_delay, Duration::from_millis(500));
        assert_eq!(c.max_delay, Duration::from_secs(30));
    }

    #[test]
    fn test_retry_config_builder() {
        let c = RetryConfig::new()
            .with_max_retries(5)
            .with_base_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(120));
        assert_eq!(c.max_retries, 5);
        assert_eq!(c.base_delay, Duration::from_secs(1));
        assert_eq!(c.max_delay, Duration::from_secs(120));
    }

    #[test]
    fn test_calculate_delay_within_max() {
        let c = RetryConfig::default();
        for attempt in 0..10 {
            let d = c.calculate_delay(attempt);
            assert!(d <= c.max_delay, "attempt {attempt}: {d:?} > max {:?}", c.max_delay);
        }
    }

    #[test]
    fn test_calculate_delay_capped() {
        let c = RetryConfig::new()
            .with_base_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(5));
        assert!(c.calculate_delay(10) <= Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_with_retry_mapped_success_on_first() {
        let calls = AtomicUsize::new(0);
        let config = RetryConfig::new().with_max_retries(3);

        let result = with_retry_mapped(
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>("ok")
            },
            &config,
            |_| true,
        )
        .await;

        assert_eq!(result.unwrap(), "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_with_retry_mapped_retries_then_succeeds() {
        let calls = AtomicUsize::new(0);
        let config = RetryConfig::new()
            .with_max_retries(3)
            .with_base_delay(Duration::from_millis(1))
            .with_max_delay(Duration::from_millis(5));

        let result = with_retry_mapped(
            || async {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err("transient".to_string())
                } else {
                    Ok("ok")
                }
            },
            &config,
            |_| true,
        )
        .await;

        assert_eq!(result.unwrap(), "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_with_retry_mapped_exhausted_returns_last_error() {
        let calls = AtomicUsize::new(0);
        let config = RetryConfig::new()
            .with_max_retries(2)
            .with_base_delay(Duration::from_millis(1))
            .with_max_delay(Duration::from_millis(5));

        let result = with_retry_mapped(
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>("always fails".to_string())
            },
            &config,
            |_| true,
        )
        .await;

        assert!(result.is_err());
        // 1 initial + 2 retries = 3 total calls
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_with_retry_mapped_permanent_error_no_retry() {
        let calls = AtomicUsize::new(0);
        let config = RetryConfig::new()
            .with_max_retries(3)
            .with_base_delay(Duration::from_millis(1))
            .with_max_delay(Duration::from_millis(5));

        let result = with_retry_mapped(
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>("permanent")
            },
            &config,
            |_| false, // never retry
        )
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1); // zero retries
    }
}
