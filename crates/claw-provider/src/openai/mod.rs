pub mod format;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use secrecy::{ExposeSecret, SecretString};

use crate::{
    error::ProviderError,
    retry::RetryConfig,
    traits::{HttpTransport, LLMProvider, MessageFormat},
    transport::DefaultHttpTransport,
    types::{CompletionResponse, Delta, Message, Options},
};

pub use format::OpenAIFormat;
use crate::stream_utils::parse_sse_stream;

pub struct OpenAIProvider {
    pub(crate) api_key: SecretString,
    pub(crate) model: String,
    pub(crate) base_url: String,
    pub(crate) transport: Arc<dyn HttpTransport>,
    pub(crate) retry_config: Option<crate::retry::RetryConfig>,
}

impl OpenAIProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: SecretString::new(api_key.into()),
            model: model.into(),
            base_url: "https://api.openai.com/v1".to_string(),
            transport: Arc::new(DefaultHttpTransport::new("https://api.openai.com")),
            retry_config: None,
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn from_env() -> Result<Self, ProviderError> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| ProviderError::Auth("OPENAI_API_KEY not set".into()))?;
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());
        Ok(Self::new(api_key, model))
    }

    /// Set the retry configuration for this provider.
    pub fn with_retry(mut self, config: RetryConfig) -> Self {
        self.retry_config = Some(config);
        // Recreate transport with retry config
        let base_url = self.base_url.trim_end_matches("/v1").to_string();
        let transport = DefaultHttpTransport::new(base_url).with_retry(config);
        self.transport = Arc::new(transport);
        self
    }

    /// Get the current retry configuration.
    pub fn retry_config(&self) -> Option<&RetryConfig> {
        self.retry_config.as_ref()
    }

    /// Set a custom HTTP transport (for testing purposes).
    #[cfg(feature = "test-utils")]
    pub fn with_transport(mut self, transport: Arc<dyn HttpTransport>) -> Self {
        self.transport = transport;
        self
    }

    /// Set a custom HTTP transport (internal testing helper).
    /// Not part of the public API, may change without notice.
    #[doc(hidden)]
    pub fn __with_transport(mut self, transport: Arc<dyn HttpTransport>) -> Self {
        self.transport = transport;
        self
    }

    fn build_headers(&self) -> Vec<(String, String)> {
        vec![
            (
                "Authorization".to_string(),
                format!("Bearer {}", self.api_key.expose_secret()),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ]
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    fn provider_id(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    async fn complete(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<CompletionResponse, ProviderError> {
        let body = serde_json::to_value(OpenAIFormat::build_request(&messages, &options))
            .map_err(|e| ProviderError::Serialization(e.to_string()))?;
        let url = format!("{}/chat/completions", self.base_url);
        let headers_owned = self.build_headers();
        let headers: Vec<(&str, &str)> = headers_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let raw = self.transport.post_json(&url, &headers, &body).await?;
        OpenAIFormat::parse_response(
            serde_json::from_value(raw).map_err(|e| ProviderError::Serialization(e.to_string()))?,
        )
        .map_err(|e| ProviderError::Serialization(e.to_string()))
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
        let body = serde_json::to_value(OpenAIFormat::build_request(&messages, &stream_opts))
            .map_err(|e| ProviderError::Serialization(e.to_string()))?;
        let url = format!("{}/chat/completions", self.base_url);
        let headers_owned = self.build_headers();
        let headers: Vec<(&str, &str)> = headers_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let byte_stream = self.transport.post_stream(&url, &headers, &body).await?;

        let delta_stream = parse_sse_stream::<OpenAIFormat>(byte_stream);

        Ok(Box::pin(delta_stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry::RetryConfig;

    #[test]
    fn test_openai_provider_new() {
        let p = OpenAIProvider::new("test-key-placeholder", "gpt-4o");
        assert_eq!(p.api_key.expose_secret(), "test-key-placeholder");
        assert_eq!(p.model, "gpt-4o");
        assert!(p.retry_config().is_none());
    }

    #[test]
    fn test_openai_provider_id() {
        let p = OpenAIProvider::new("test-key-placeholder", "gpt-4o");
        assert_eq!(p.provider_id(), "openai");
    }

    #[test]
    fn test_openai_model_id() {
        let p = OpenAIProvider::new("test-key-placeholder", "gpt-4o-mini");
        assert_eq!(p.model_id(), "gpt-4o-mini");
    }

    #[test]
    fn test_openai_provider_with_retry() {
        let config = RetryConfig::new().with_max_retries(5);
        let p = OpenAIProvider::new("test-key-placeholder", "gpt-4o").with_retry(config);
        assert_eq!(p.retry_config().unwrap().max_retries, 5);
    }
}
