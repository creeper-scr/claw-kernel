use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{Stream, StreamExt};

use crate::{
    error::ProviderError,
    openai::format::OpenAIFormat,
    retry::RetryConfig,
    traits::{HttpTransport, LLMProvider, MessageFormat},
    transport::DefaultHttpTransport,
    types::{CompletionResponse, Delta, Message, Options},
};

pub struct MoonshotProvider {
    api_key: String,
    model: String,
    transport: Arc<dyn HttpTransport>,
    retry_config: Option<RetryConfig>,
}

impl MoonshotProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            transport: Arc::new(DefaultHttpTransport::new("https://api.moonshot.cn")),
            retry_config: None,
        }
    }

    /// Create a provider with a custom HTTP transport (for testing).
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
        let api_key = std::env::var("MOONSHOT_API_KEY")
            .map_err(|_| ProviderError::Auth("MOONSHOT_API_KEY not set".into()))?;
        let model =
            std::env::var("MOONSHOT_MODEL").unwrap_or_else(|_| "moonshot-v1-8k".to_string());
        Ok(Self::new(api_key, model))
    }

    /// Set the retry configuration for this provider.
    pub fn with_retry(mut self, config: RetryConfig) -> Self {
        self.retry_config = Some(config);
        // Recreate transport with retry config
        let transport = DefaultHttpTransport::new("https://api.moonshot.cn").with_retry(config);
        self.transport = Arc::new(transport);
        self
    }

    /// Get the current retry configuration.
    pub fn retry_config(&self) -> Option<&RetryConfig> {
        self.retry_config.as_ref()
    }

    fn base_url(&self) -> &str {
        "https://api.moonshot.cn/v1"
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
impl LLMProvider for MoonshotProvider {
    fn provider_id(&self) -> &str {
        "moonshot"
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
        let body =
            serde_json::to_value(&req).map_err(|e| ProviderError::Serialization(e.to_string()))?;
        let url = format!("{}/chat/completions", self.base_url());
        let headers_owned = self.build_headers();
        let headers: Vec<(&str, &str)> = headers_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let response = self.transport.post_json(&url, &headers, &body).await?;
        let raw: <OpenAIFormat as MessageFormat>::Response = serde_json::from_value(response)
            .map_err(|e| ProviderError::Serialization(e.to_string()))?;
        OpenAIFormat::parse_response(raw).map_err(|e| ProviderError::Other(e.to_string()))
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
        let body =
            serde_json::to_value(&req).map_err(|e| ProviderError::Serialization(e.to_string()))?;
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
    fn test_moonshot_provider_new() {
        let p = MoonshotProvider::new("ms-key", "moonshot-v1-8k");
        assert_eq!(p.api_key, "ms-key");
        assert_eq!(p.model, "moonshot-v1-8k");
        assert_eq!(p.provider_id(), "moonshot");
        assert_eq!(p.model_id(), "moonshot-v1-8k");
        assert!(p.retry_config().is_none());
    }

    #[test]
    fn test_moonshot_provider_with_retry() {
        let config = RetryConfig::new().with_max_retries(3);
        let p = MoonshotProvider::new("ms-key", "moonshot-v1-8k").with_retry(config);
        assert_eq!(p.retry_config().unwrap().max_retries, 3);
    }
}
