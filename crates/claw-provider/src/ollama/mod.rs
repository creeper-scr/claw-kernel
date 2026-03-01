pub mod format;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{Stream, StreamExt};

use crate::{
    error::ProviderError,
    openai::format::OpenAIFormat,
    traits::{HttpTransport, LLMProvider, MessageFormat},
    transport::DefaultHttpTransport,
    types::{CompletionResponse, Delta, Message, Options},
};

pub struct OllamaProvider {
    model: String,
    base_url: String,
    transport: Arc<dyn HttpTransport>,
    format: OpenAIFormat,
}

impl OllamaProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            base_url: "http://localhost:11434/v1".to_string(),
            transport: Arc::new(DefaultHttpTransport::new()),
            format: OpenAIFormat::new(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn from_env() -> Result<Self, ProviderError> {
        let model =
            std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3".to_string());
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
        Ok(Self::new(model).with_base_url(base_url))
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
        let body = self.format.format_request(&messages, &options)?;
        let url = format!("{}/chat/completions", self.base_url);
        let headers_owned = self.build_headers();
        let headers: Vec<(&str, &str)> =
            headers_owned.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let raw = self.transport.post_json(&url, &headers, &body).await?;
        self.format.parse_response(raw)
    }

    async fn complete_stream(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError>
    {
        let stream_opts = Options { stream: true, ..options };
        let body = self.format.format_request(&messages, &stream_opts)?;
        let url = format!("{}/chat/completions", self.base_url);
        let headers_owned = self.build_headers();
        let headers: Vec<(&str, &str)> =
            headers_owned.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let byte_stream = self.transport.post_stream(&url, &headers, &body).await?;

        let format = OpenAIFormat::new();
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
                            match format.parse_stream_chunk(trimmed) {
                                Ok(Some(delta)) => Some(Ok(delta)),
                                Ok(None) => None,
                                Err(e) => Some(Err(e)),
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

    #[test]
    fn test_ollama_provider_new() {
        let p = OllamaProvider::new("llama3");
        assert_eq!(p.model, "llama3");
        assert_eq!(p.base_url, "http://localhost:11434/v1");
        assert_eq!(p.provider_id(), "ollama");
        assert_eq!(p.model_id(), "llama3");
    }
}
