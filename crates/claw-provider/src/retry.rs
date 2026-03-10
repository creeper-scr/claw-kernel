//! Retry mechanism for LLM provider requests.
//!
//! Provides exponential backoff with jitter for transient failures.

use std::time::Duration;

use rand::Rng;

use crate::error::ProviderError;

/// Configuration for retry behavior.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Base delay for exponential backoff.
    pub base_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
    /// HTTP status codes that should trigger a retry.
    pub retryable_statuses: &'static [u16],
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(60),
            retryable_statuses: &[429, 500, 502, 503, 504],
        }
    }
}

impl RetryConfig {
    /// Create a new retry config with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum number of retries.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set the base delay for exponential backoff.
    pub fn with_base_delay(mut self, base_delay: Duration) -> Self {
        self.base_delay = base_delay;
        self
    }

    /// Set the maximum delay between retries.
    pub fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    /// Set the retryable HTTP status codes.
    pub fn with_retryable_statuses(mut self, statuses: &'static [u16]) -> Self {
        self.retryable_statuses = statuses;
        self
    }

    /// Check if a status code is retryable.
    pub fn is_retryable_status(&self, status: u16) -> bool {
        self.retryable_statuses.contains(&status)
    }

    /// Calculate the delay for a specific retry attempt.
    ///
    /// Uses exponential backoff with full jitter:
    /// - Attempt 0: 500ms base + jitter
    /// - Attempt 1: 1s + jitter  
    /// - Attempt 2: 2s + jitter
    /// - etc.
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        // Exponential backoff: base_delay * 2^attempt
        let exponential = self
            .base_delay
            .as_millis()
            .saturating_mul(2u128.pow(attempt));
        let delay_ms = std::cmp::min(exponential as u64, self.max_delay.as_millis() as u64);

        // Add full jitter (0 to delay_ms)
        let jitter = if delay_ms > 0 {
            rand::thread_rng().gen_range(0..delay_ms)
        } else {
            0
        };

        let total = std::cmp::min(delay_ms + jitter, self.max_delay.as_millis() as u64);
        Duration::from_millis(total)
    }
}

/// Check if an error is retryable.
///
/// Network errors and certain HTTP status codes are considered retryable.
pub fn is_retryable_error(error: &ProviderError, config: &RetryConfig) -> bool {
    match error {
        // Network errors are always retryable
        ProviderError::Network(_) => true,
        // Rate limiting is retryable
        ProviderError::RateLimited { .. } => true,
        // HTTP errors depend on status code
        ProviderError::Http { status, .. } => config.is_retryable_status(*status),
        // Other errors are not retryable
        _ => false,
    }
}

/// Execute an async operation with retry logic.
///
/// This function will retry the operation on transient failures using
/// exponential backoff with jitter.
///
/// # Example
///
/// ```rust,no_run
/// use claw_provider::retry::{with_retry, RetryConfig};
/// use claw_provider::error::ProviderError;
///
/// async fn fetch_data() -> Result<String, ProviderError> {
///     // ... some async operation
///     Ok("data".to_string())
/// }
///
/// # async fn example() -> Result<(), ProviderError> {
/// let config = RetryConfig::default();
/// let result = with_retry(|| fetch_data(), &config).await?;
/// # Ok(())
/// # }
/// ```
pub async fn with_retry<F, Fut, T>(operation: F, config: &RetryConfig) -> Result<T, ProviderError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, ProviderError>>,
{
    let mut last_error = None;

    for attempt in 0..=config.max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                // Check if we should retry this error
                if attempt < config.max_retries && is_retryable_error(&error, config) {
                    let delay = config.calculate_delay(attempt);
                    tokio::time::sleep(delay).await;
                    last_error = Some(error);
                } else {
                    // Not retryable or max retries exceeded
                    return Err(error);
                }
            }
        }
    }

    // This should not be reached, but return the last error just in case
    Err(last_error.unwrap_or_else(|| ProviderError::Other("Retry exhausted".to_string())))
}

/// Execute an async operation with retry logic and custom error mapping.
///
/// Similar to `with_retry` but allows custom error types.
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
    let mut last_error = None;

    for attempt in 0..=config.max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                if attempt < config.max_retries && is_retryable(&error) {
                    let delay = config.calculate_delay(attempt);
                    tokio::time::sleep(delay).await;
                    last_error = Some(error);
                } else {
                    return Err(error);
                }
            }
        }
    }

    Err(last_error.expect("max_retries >= 0 guarantees at least one attempt"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.base_delay, Duration::from_millis(500));
        assert_eq!(config.max_delay, Duration::from_secs(60));
        assert_eq!(config.retryable_statuses, &[429, 500, 502, 503, 504]);
    }

    #[test]
    fn test_retry_config_builder() {
        let config = RetryConfig::new()
            .with_max_retries(5)
            .with_base_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(300))
            .with_retryable_statuses(&[500, 503]);

        assert_eq!(config.max_retries, 5);
        assert_eq!(config.base_delay, Duration::from_secs(1));
        assert_eq!(config.max_delay, Duration::from_secs(300));
        assert_eq!(config.retryable_statuses, &[500, 503]);
    }

    #[test]
    fn test_is_retryable_status() {
        let config = RetryConfig::default();
        assert!(config.is_retryable_status(429));
        assert!(config.is_retryable_status(500));
        assert!(config.is_retryable_status(502));
        assert!(config.is_retryable_status(503));
        assert!(config.is_retryable_status(504));
        assert!(!config.is_retryable_status(400));
        assert!(!config.is_retryable_status(404));
        assert!(!config.is_retryable_status(200));
    }

    #[test]
    fn test_calculate_delay() {
        let config = RetryConfig::default();

        // Attempt 0: base 500ms
        let delay0 = config.calculate_delay(0);
        assert!(delay0 >= Duration::from_millis(500));
        assert!(delay0 <= Duration::from_millis(1000));

        // Attempt 1: base 1000ms
        let delay1 = config.calculate_delay(1);
        assert!(delay1 >= Duration::from_millis(1000));
        assert!(delay1 <= Duration::from_millis(2000));

        // Attempt 2: base 2000ms
        let delay2 = config.calculate_delay(2);
        assert!(delay2 >= Duration::from_millis(2000));
        assert!(delay2 <= Duration::from_millis(4000));
    }

    #[test]
    fn test_calculate_delay_respects_max() {
        let config = RetryConfig::new()
            .with_base_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(5));

        // High attempt numbers should be capped at max_delay
        let delay = config.calculate_delay(10);
        assert!(delay <= Duration::from_secs(5));
    }

    #[test]
    fn test_is_retryable_error() {
        let config = RetryConfig::default();

        // Network errors are retryable
        let network_err = ProviderError::Network("timeout".to_string());
        assert!(is_retryable_error(&network_err, &config));

        // Rate limited is retryable
        let rate_limited = ProviderError::RateLimited {
            retry_after_secs: 5,
        };
        assert!(is_retryable_error(&rate_limited, &config));

        // Retryable HTTP status
        let http_retryable = ProviderError::Http {
            status: 503,
            message: "Service Unavailable".to_string(),
        };
        assert!(is_retryable_error(&http_retryable, &config));

        // Non-retryable HTTP status
        let http_not_retryable = ProviderError::Http {
            status: 400,
            message: "Bad Request".to_string(),
        };
        assert!(!is_retryable_error(&http_not_retryable, &config));

        // Auth errors are not retryable
        let auth_err = ProviderError::Auth("invalid key".to_string());
        assert!(!is_retryable_error(&auth_err, &config));
    }

    #[tokio::test]
    async fn test_with_retry_success() {
        let config = RetryConfig::new().with_max_retries(3);
        let call_count = AtomicUsize::new(0);

        let result = with_retry(
            || async {
                call_count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ProviderError>("success")
            },
            &config,
        )
        .await;

        assert_eq!(result.unwrap(), "success");
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_with_retry_eventual_success() {
        let config = RetryConfig::new().with_max_retries(3);
        let call_count = AtomicUsize::new(0);

        let result = with_retry(
            || async {
                let count = call_count.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    Err(ProviderError::Network("timeout".to_string()))
                } else {
                    Ok("success")
                }
            },
            &config,
        )
        .await;

        assert_eq!(result.unwrap(), "success");
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_with_retry_exhausted() {
        let config = RetryConfig::new().with_max_retries(2);
        let call_count = AtomicUsize::new(0);

        let result = with_retry(
            || async {
                call_count.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(ProviderError::Network("timeout".to_string()))
            },
            &config,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 3); // initial + 2 retries
    }

    #[tokio::test]
    async fn test_with_retry_non_retryable_error() {
        let config = RetryConfig::new().with_max_retries(3);
        let call_count = AtomicUsize::new(0);

        let result = with_retry(
            || async {
                call_count.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(ProviderError::Auth("invalid".to_string()))
            },
            &config,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // no retries
    }
}
