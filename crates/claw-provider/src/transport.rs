use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::{Client, header::HeaderMap};

use crate::{
    error::ProviderError,
    traits::{HttpTransport, HttpTransportExt},
};

/// Default HTTP transport backed by `reqwest`.
pub struct DefaultHttpTransport {
    client: Client,
    base_url: String,
    headers: HeaderMap,
}

impl DefaultHttpTransport {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_config(base_url, std::time::Duration::from_secs(120), HeaderMap::new())
    }

    pub fn with_timeout(base_url: impl Into<String>, timeout: std::time::Duration) -> Self {
        Self::with_config(base_url, timeout, HeaderMap::new())
    }

    pub fn with_config(
        base_url: impl Into<String>,
        timeout: std::time::Duration,
        headers: HeaderMap,
    ) -> Self {
        Self {
            client: Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_default(),
            base_url: base_url.into(),
            headers,
        }
    }

    pub fn with_auth(mut self, token: impl AsRef<str>) -> Self {
        use reqwest::header::{AUTHORIZATION, HeaderValue};
        let value = HeaderValue::from_str(&format!("Bearer {}", token.as_ref()))
            .unwrap_or_else(|_| HeaderValue::from_static(""));
        self.headers.insert(AUTHORIZATION, value);
        self
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

    async fn post_stream(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send>>,
        ProviderError,
    > {
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
        
        let byte_stream = resp.bytes_stream().map(|r| {
            r.map_err(|e| ProviderError::Network(e.to_string()))
        });
        
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_http_transport_new() {
        let transport = DefaultHttpTransport::new("https://api.example.com");
        assert_eq!(transport.base_url(), "https://api.example.com");
    }

    #[test]
    fn test_default_http_transport_with_auth() {
        let transport = DefaultHttpTransport::new("https://api.example.com")
            .with_auth("test-token");
        let headers = transport.auth_headers();
        assert!(headers.contains_key("authorization"));
    }
}
