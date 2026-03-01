use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
use reqwest::Client;

use crate::{error::ProviderError, traits::HttpTransport};

/// Default HTTP transport backed by `reqwest`.
pub struct DefaultHttpTransport {
    client: Client,
}

impl DefaultHttpTransport {
    pub fn new() -> Self {
        Self::with_timeout(std::time::Duration::from_secs(120))
    }

    pub fn with_timeout(timeout: std::time::Duration) -> Self {
        Self {
            client: Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_default(),
        }
    }
}

impl Default for DefaultHttpTransport {
    fn default() -> Self {
        Self::new()
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
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send>>, ProviderError>
    {
        let mut req = self.client.post(url).json(body);
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

        use futures::StreamExt;
        let byte_stream = resp
            .bytes_stream()
            .map(|r| r.map_err(|e| ProviderError::Stream(e.to_string())));
        Ok(Box::pin(byte_stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_transport_new() {
        let _t = DefaultHttpTransport::new();
    }

    #[test]
    fn test_default_transport_default() {
        let _t = DefaultHttpTransport::default();
    }

    #[test]
    fn test_transport_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DefaultHttpTransport>();
    }

    #[test]
    fn test_default_transport_with_timeout() {
        let _t = DefaultHttpTransport::with_timeout(std::time::Duration::from_secs(60));
    }
}
