use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::{header::HeaderMap, Client};

use crate::{
    error::ProviderError,
    retry::{with_retry, RetryConfig},
    traits::{HttpTransport, HttpTransportExt},
};

/// Default HTTP transport backed by `reqwest`.
pub struct DefaultHttpTransport {
    client: Client,
    base_url: String,
    headers: HeaderMap,
    retry_config: Option<RetryConfig>,
}

impl DefaultHttpTransport {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_config(
            base_url,
            std::time::Duration::from_secs(120),
            HeaderMap::new(),
        )
        .expect(
            "reqwest::Client 构建失败 — 这是 TLS 配置或系统证书存储的 bug。请确认 openssl/native-tls 已正确链接。",
        )
    }

    pub fn with_timeout(base_url: impl Into<String>, timeout: std::time::Duration) -> Self {
        Self::with_config(base_url, timeout, HeaderMap::new())
            .expect("reqwest::Client 构建失败 — 这是 TLS 配置或系统证书存储的 bug。")
    }

    pub fn with_config(
        base_url: impl Into<String>,
        timeout: std::time::Duration,
        headers: HeaderMap,
    ) -> Result<Self, crate::error::ProviderError> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| crate::error::ProviderError::BuildFailed(e.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.into(),
            headers,
            retry_config: None,
        })
    }

    /// 配置 Bearer Token 授权头。
    /// # Errors
    /// 若 token 含非 ASCII 字符或控制字符，返回 `ProviderError::Auth`。
    pub fn with_auth(mut self, token: impl AsRef<str>) -> Result<Self, ProviderError> {
        use reqwest::header::{HeaderValue, AUTHORIZATION};
        let header_value = HeaderValue::from_str(&format!("Bearer {}", token.as_ref()))
            .map_err(|e| ProviderError::Auth(
                format!("授权 token 含非法字符（非 ASCII 或控制字符）: {}", e)
            ))?;
        self.headers.insert(AUTHORIZATION, header_value);
        Ok(self)
    }

    /// Enable retry with the given configuration.
    pub fn with_retry(mut self, config: RetryConfig) -> Self {
        self.retry_config = Some(config);
        self
    }

    /// Get the current retry configuration.
    pub fn retry_config(&self) -> Option<&RetryConfig> {
        self.retry_config.as_ref()
    }

    /// Perform a POST request with optional retry logic.
    async fn post_json_with_retry(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        let do_request = || async { self.do_post_json(url, headers, body).await };

        if let Some(ref config) = self.retry_config {
            with_retry(do_request, config).await
        } else {
            do_request().await
        }
    }

    /// Internal method to perform the actual POST request.
    async fn do_post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        let mut req = self.client.post(url).json(body);

        // Add default headers
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }

        // Add request-specific headers
        for (k, v) in headers {
            req = req.header(*k, *v);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        let status = resp.status();

        if status.is_client_error() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status: status.as_u16(),
                message: msg,
            });
        }
        if status.is_server_error() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Network(format!(
                "server error {}: {}",
                status.as_u16(),
                msg
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Serialization(e.to_string()))?;
        Ok(json)
    }

    /// Internal method to perform the actual GET request.
    async fn do_get_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<serde_json::Value, ProviderError> {
        let mut req = self.client.get(url);

        // Add default headers
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }

        // Add request-specific headers
        for (k, v) in headers {
            req = req.header(*k, *v);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        let status = resp.status();

        if status.is_client_error() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status: status.as_u16(),
                message: msg,
            });
        }
        if status.is_server_error() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Network(format!(
                "server error {}: {}",
                status.as_u16(),
                msg
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Serialization(e.to_string()))?;
        Ok(json)
    }
}

impl Default for DefaultHttpTransport {
    fn default() -> Self {
        Self::new("https://api.openai.com")
    }
}

#[async_trait]
impl HttpTransport for DefaultHttpTransport {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        self.post_json_with_retry(url, headers, body).await
    }

    async fn get_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<serde_json::Value, ProviderError> {
        self.do_get_json(url, headers).await
    }

    async fn post_stream(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send>>, ProviderError>
    {
        let mut req = self.client.post(url).json(body);

        // Add default headers
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }

        for (k, v) in headers {
            req = req.header(*k, *v);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        let byte_stream = resp
            .bytes_stream()
            .map(|r| r.map_err(|e| ProviderError::Network(e.to_string())));

        Ok(Box::pin(byte_stream))
    }
}

impl HttpTransportExt for DefaultHttpTransport {
    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn auth_headers(&self) -> HeaderMap {
        self.headers.clone()
    }

    fn http_client(&self) -> &Client {
        &self.client
    }
}

/// Parse a `Retry-After` header value into a `Duration`.
///
/// Supports integer seconds (e.g., `"30"`).
/// TODO: support HTTP-date format in Retry-After header (e.g., with httpdate crate)
/// Currently only integer seconds are supported; date-format values fall back to 1s.
pub fn parse_retry_after(value: &str) -> Duration {
    // Try seconds first
    if let Ok(secs) = value.trim().parse::<u64>() {
        return Duration::from_secs(secs);
    }
    // Default fallback: 1 second
    // HTTP-date format (e.g., "Tue, 15 Nov 1994 08:12:31 GMT") is not yet supported.
    Duration::from_secs(1)
}

/// Build a redacted list of headers for safe logging.
///
/// Authorization and API key headers are replaced with `[REDACTED]` to prevent
/// credential leakage in logs.
pub fn redact_headers_for_log(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    let sensitive = ["authorization", "x-api-key", "api-key", "x-goog-api-key"];
    headers
        .iter()
        .map(|(k, v)| {
            let key = k.as_str().to_lowercase();
            let val = if sensitive.iter().any(|s| key.contains(s)) {
                "[REDACTED]".to_string()
            } else {
                v.to_str().unwrap_or("<invalid>").to_string()
            };
            (k.as_str().to_string(), val)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry::RetryConfig;

    #[test]
    fn test_default_http_transport_new() {
        let transport = DefaultHttpTransport::new("https://api.example.com");
        assert_eq!(transport.base_url(), "https://api.example.com");
        assert!(transport.retry_config().is_none());
    }

    #[test]
    fn test_default_http_transport_with_auth() {
        let transport =
            DefaultHttpTransport::new("https://api.example.com").with_auth("test-token").expect("valid token");
        let headers = transport.auth_headers();
        assert!(headers.contains_key("authorization"));
    }

    #[test]
    fn test_default_http_transport_with_retry() {
        let retry_config = RetryConfig::default();
        let transport =
            DefaultHttpTransport::new("https://api.example.com").with_retry(retry_config);
        assert!(transport.retry_config().is_some());
    }

    #[test]
    fn test_retry_config_builder_chaining() {
        let transport = DefaultHttpTransport::new("https://api.example.com")
            .with_auth("test-token").expect("valid token")
            .with_retry(RetryConfig::new().with_max_retries(5));

        assert_eq!(transport.base_url(), "https://api.example.com");
        assert!(transport.auth_headers().contains_key("authorization"));
        assert_eq!(transport.retry_config().unwrap().max_retries, 5);
    }
}
