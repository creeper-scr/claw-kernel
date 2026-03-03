pub mod format;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{Stream, StreamExt};

use crate::{
    error::ProviderError,
    ollama::format::OllamaFormat,
    retry::RetryConfig,
    traits::{HttpTransport, LLMProvider, MessageFormat},
    transport::DefaultHttpTransport,
    types::{CompletionResponse, Delta, Message, Options},
};

pub struct OllamaProvider {
    model: String,
    base_url: String,
    transport: Arc<dyn HttpTransport>,
    retry_config: Option<RetryConfig>,
}

impl OllamaProvider {
    pub fn new(model: impl Into<String>) -> Self {
        let base_url = "http://localhost:11434".to_string();
        Self {
            model: model.into(),
            base_url: base_url.clone(),
            transport: Arc::new(DefaultHttpTransport::new(base_url)),
            retry_config: None,
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        let url_str: String = url.into();
        // Strip /v1 suffix if present, as we'll add it back in the endpoint
        self.base_url = url_str.trim_end_matches("/v1").to_string();
        self
    }

    pub fn from_env() -> Result<Self, ProviderError> {
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3".to_string());
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        Ok(Self::new(model).with_base_url(base_url))
    }

    /// Set the retry configuration for this provider.
    pub fn with_retry(mut self, config: RetryConfig) -> Self {
        self.retry_config = Some(config);
        // Recreate transport with retry config
        let transport = DefaultHttpTransport::new(self.base_url.clone()).with_retry(config);
        self.transport = Arc::new(transport);
        self
    }

    /// Get the current retry configuration.
    pub fn retry_config(&self) -> Option<&RetryConfig> {
        self.retry_config.as_ref()
    }

    fn build_headers(&self) -> Vec<(String, String)> {
        vec![
            // Ollama accepts any non-empty api_key
            ("Authorization".to_string(), "Bearer ollama".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ]
    }
}

#[async_trait]
impl LLMProvider for OllamaProvider {
    fn provider_id(&self) -> &str {
        "ollama"
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    async fn complete(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<CompletionResponse, ProviderError> {
        let req = OllamaFormat::build_request(&messages, &options);
        let body = serde_json::to_value(&req).map_err(|e| {
            ProviderError::Serialization(format!("Failed to serialize request: {}", e))
        })?;
        let url = format!("{}/v1/chat/completions", self.base_url);
        let headers_owned = self.build_headers();
        let headers: Vec<(&str, &str)> = headers_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let raw = self.transport.post_json(&url, &headers, &body).await?;
        let response: <OllamaFormat as MessageFormat>::Response = serde_json::from_value(raw)
            .map_err(|e| {
                ProviderError::Serialization(format!("Failed to parse response: {}", e))
            })?;
        OllamaFormat::parse_response(response).map_err(|e| ProviderError::Other(e.to_string()))
    }

    async fn complete_stream(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError>
    {
        let stream_opts = Options {
            stream: true,
            ..options
        };
        let req = OllamaFormat::build_request(&messages, &stream_opts);
        let body = serde_json::to_value(&req).map_err(|e| {
            ProviderError::Serialization(format!("Failed to serialize request: {}", e))
        })?;
        let url = format!("{}/v1/chat/completions", self.base_url);
        let headers_owned = self.build_headers();
        let headers: Vec<(&str, &str)> = headers_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let byte_stream = self.transport.post_stream(&url, &headers, &body).await?;

        let delta_stream = byte_stream.flat_map(move |chunk_result| {
            let deltas: Vec<Result<Delta, ProviderError>> = match chunk_result {
                Err(e) => vec![Err(e)],
                Ok(bytes) => match OllamaFormat::parse_stream_chunk(&bytes) {
                    Ok(Some(delta)) => vec![Ok(delta)],
                    Ok(None) => vec![],
                    Err(e) => vec![Err(ProviderError::Other(e.to_string()))],
                },
            };
            futures::stream::iter(deltas)
        });

        Ok(Box::pin(delta_stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry::RetryConfig;

    #[test]
    fn test_ollama_provider_new() {
        let p = OllamaProvider::new("llama3");
        assert_eq!(p.model, "llama3");
        assert_eq!(p.base_url, "http://localhost:11434");
        assert_eq!(p.provider_id(), "ollama");
        assert_eq!(p.model_id(), "llama3");
        assert!(p.retry_config().is_none());
    }

    #[test]
    fn test_ollama_provider_with_retry() {
        let config = RetryConfig::new().with_max_retries(3);
        let p = OllamaProvider::new("llama3").with_retry(config);
        assert_eq!(p.retry_config().unwrap().max_retries, 3);
    }
}
