use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{Stream, StreamExt};

use crate::{
    error::ProviderError,
    openai::format::OpenAIFormat,
    openai::format::OpenAIResponse,
    retry::RetryConfig,
    traits::{HttpTransport, LLMProvider, MessageFormat},
    transport::DefaultHttpTransport,
    types::{CompletionResponse, Delta, Message, Options},
};

pub struct DeepSeekProvider {
    api_key: String,
    model: String,
    transport: Arc<dyn HttpTransport>,
    retry_config: Option<RetryConfig>,
}

impl DeepSeekProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            transport: Arc::new(DefaultHttpTransport::new("https://api.deepseek.com")),
            retry_config: None,
        }
    }

    /// Create a new DeepSeekProvider with a custom HTTP transport.
    /// This is primarily used for testing with mock transports.
    #[doc(hidden)]
    pub fn with_transport(
        api_key: impl Into<String>,
        model: impl Into<String>,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            transport,
            retry_config: None,
        }
    }

    pub fn from_env() -> Result<Self, ProviderError> {
        let api_key = std::env::var("DEEPSEEK_API_KEY")
            .map_err(|_| ProviderError::Auth("DEEPSEEK_API_KEY not set".into()))?;
        let model = std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-chat".to_string());
        Ok(Self::new(api_key, model))
    }

    /// Set the retry configuration for this provider.
    pub fn with_retry(mut self, config: RetryConfig) -> Self {
        self.retry_config = Some(config);
        // Recreate transport with retry config
        let transport = DefaultHttpTransport::new("https://api.deepseek.com").with_retry(config);
        self.transport = Arc::new(transport);
        self
    }

    /// Get the current retry configuration.
    pub fn retry_config(&self) -> Option<&RetryConfig> {
        self.retry_config.as_ref()
    }

    fn base_url(&self) -> &str {
        "https://api.deepseek.com"
    }

    fn build_headers(&self) -> Vec<(String, String)> {
        vec![
            (
                "Authorization".to_string(),
                format!("Bearer {}", self.api_key),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ]
    }
}

#[async_trait]
impl LLMProvider for DeepSeekProvider {
    fn provider_id(&self) -> &str {
        "deepseek"
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    async fn complete(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<CompletionResponse, ProviderError> {
        let req = OpenAIFormat::build_request(&messages, &options);
        let body = serde_json::to_value(&req).map_err(|e| {
            ProviderError::Serialization(format!("Failed to serialize request: {}", e))
        })?;
        let url = format!("{}/chat/completions", self.base_url());
        let headers_owned = self.build_headers();
        let headers: Vec<(&str, &str)> = headers_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let raw = self.transport.post_json(&url, &headers, &body).await?;
        let parsed_response: OpenAIResponse = serde_json::from_value(raw).map_err(|e| {
            ProviderError::Serialization(format!("Failed to deserialize response: {}", e))
        })?;
        OpenAIFormat::parse_response(parsed_response)
            .map_err(|e| ProviderError::Other(e.to_string()))
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
        let req = OpenAIFormat::build_request(&messages, &stream_opts);
        let body = serde_json::to_value(&req).map_err(|e| {
            ProviderError::Serialization(format!("Failed to serialize request: {}", e))
        })?;
        let url = format!("{}/chat/completions", self.base_url());
        let headers_owned = self.build_headers();
        let headers: Vec<(&str, &str)> = headers_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let byte_stream = self.transport.post_stream(&url, &headers, &body).await?;

        let delta_stream = byte_stream.flat_map(move |chunk_result| {
            let deltas: Vec<Result<Delta, ProviderError>> = match chunk_result {
                Err(e) => vec![Err(e)],
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    text.lines()
                        .filter_map(|line| {
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                return None;
                            }
                            match OpenAIFormat::parse_stream_chunk(trimmed.as_bytes()) {
                                Ok(Some(delta)) => Some(Ok(delta)),
                                Ok(None) => None,
                                Err(e) => Some(Err(ProviderError::Other(e.to_string()))),
                            }
                        })
                        .collect()
                }
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
    fn test_deepseek_provider_new() {
        let p = DeepSeekProvider::new("ds-key", "deepseek-chat");
        assert_eq!(p.api_key, "ds-key");
        assert_eq!(p.model, "deepseek-chat");
        assert_eq!(p.provider_id(), "deepseek");
        assert_eq!(p.model_id(), "deepseek-chat");
        assert!(p.retry_config().is_none());
    }

    #[test]
    fn test_deepseek_provider_with_retry() {
        let config = RetryConfig::new().with_max_retries(3);
        let p = DeepSeekProvider::new("ds-key", "deepseek-chat").with_retry(config);
        assert_eq!(p.retry_config().unwrap().max_retries, 3);
    }
}
