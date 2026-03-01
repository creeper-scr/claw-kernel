use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    error::ProviderError,
    types::{CompletionResponse, Delta, Embedding, Message, Options},
};

/// Provider-specific HTTP serialization/deserialization (Level 1: Protocol Abstraction).
///
/// Abstracts the message format differences between LLM providers.
/// OpenAI-compatible providers share the same format, Anthropic uses a different format.
pub trait MessageFormat: Send + Sync {
    /// Request type for this format.
    type Request: Serialize;
    /// Response type for this format.
    type Response: DeserializeOwned;
    /// Stream chunk type for this format.
    type StreamChunk: DeserializeOwned;
    /// Error type for parsing failures.
    type Error: std::error::Error + 'static;

    /// Build request from canonical messages and options.
    fn build_request(messages: &[Message], opts: &Options) -> Self::Request;

    /// Parse response to canonical format.
    fn parse_response(raw: Self::Response) -> Result<CompletionResponse, Self::Error>;

    /// Parse one SSE/NDJSON chunk.
    /// Returns `None` if the chunk signals stream end (e.g. `[DONE]`).
    fn parse_stream_chunk(chunk: &[u8]) -> Result<Option<Delta>, Self::Error>;

    /// Token count estimate for this format.
    fn token_count(messages: &[Message]) -> usize;

    /// API endpoint path for this format.
    fn endpoint() -> &'static str;
}

/// Low-level HTTP transport for provider API calls (Level 2: Reusable Transport).
///
/// Implements generic HTTP logic reused by all providers.
/// This trait is object-safe and can be used with `dyn HttpTransport`.
#[async_trait]
pub trait HttpTransport: Send + Sync {
    /// POST JSON and return the full response body.
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError>;

    /// POST and return a raw byte stream (for SSE / NDJSON responses).
    async fn post_stream(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<bytes::Bytes, ProviderError>> + Send>>,
        ProviderError,
    >;
}

/// Extension trait for HttpTransport providing generic request methods.
///
/// This trait uses generic methods and cannot be made into a trait object.
/// Use `HttpTransport` for trait objects, and this trait for generic operations.
pub trait HttpTransportExt: HttpTransport {
    /// Base URL for the provider API.
    fn base_url(&self) -> &str;

    /// Authentication headers for the provider.
    fn auth_headers(&self) -> reqwest::header::HeaderMap;

    /// HTTP client reference.
    fn http_client(&self) -> &reqwest::Client;

    /// Generic request using MessageFormat — reused by ALL providers.
    fn request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> impl std::future::Future<Output = Result<CompletionResponse, ProviderError>> + Send 
    where
        <F as MessageFormat>::Request: Send,
    {
        async move {
            let req = F::build_request(messages, opts);
            let url = format!("{}{}", self.base_url(), F::endpoint());
            
            let body = serde_json::to_value(&req).map_err(|e| {
                ProviderError::Serialization(format!("Failed to serialize request: {}", e))
            })?;
            
            let response = self.post_json(&url, &[], &body).await?;
            
            let raw: F::Response = serde_json::from_value(response).map_err(|e| {
                ProviderError::Serialization(format!("Failed to parse response: {}", e))
            })?;
            
            F::parse_response(raw).map_err(|e| ProviderError::Other(e.to_string()))
        }
    }

    /// Generic streaming request.
    fn stream_request<F: MessageFormat>(
        &self,
        _messages: &[Message],
        _opts: &Options,
    ) -> impl std::future::Future<Output = Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError>> + Send {
        async move {
            todo!("Streaming request requires SSE parsing implementation")
        }
    }
}

/// High-level LLM provider interface (Level 3: User-facing).
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Short identifier for this provider (e.g., "anthropic", "openai").
    fn provider_id(&self) -> &str;

    /// Default model ID used by this provider.
    fn model_id(&self) -> &str;

    /// Non-streaming completion.
    async fn complete(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<CompletionResponse, ProviderError>;

    /// Streaming completion — returns a stream of deltas.
    async fn complete_stream(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError>;

    /// Rough token count estimate (default: chars / 4).
    fn token_count(&self, text: &str) -> usize {
        text.len() / 4
    }
}

/// Provider that generates embedding vectors.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a single text.
    async fn embed(&self, text: &str) -> Result<Embedding, ProviderError>;

    /// Embed a batch of texts.
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>, ProviderError>;

    /// Embedding vector dimensions.
    fn dimensions(&self) -> usize;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FinishReason, TokenUsage};

    struct MockProvider;

    #[async_trait]
    impl LLMProvider for MockProvider {
        fn provider_id(&self) -> &str {
            "mock"
        }

        fn model_id(&self) -> &str {
            "mock-v1"
        }

        async fn complete(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                id: "mock-id".to_string(),
                model: "mock-v1".to_string(),
                message: Message::assistant("mock response"),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            })
        }

        async fn complete_stream(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError>
        {
            use futures::stream;
            Ok(Box::pin(stream::empty()))
        }
    }

    #[tokio::test]
    async fn test_mock_provider_complete() {
        let provider = MockProvider;
        let messages = vec![Message::user("hello")];
        let opts = Options::new("mock-v1");
        let resp = provider
            .complete(messages, opts)
            .await
            .expect("complete failed");
        assert_eq!(resp.id, "mock-id");
        assert_eq!(resp.model, "mock-v1");
        assert_eq!(resp.message.content, "mock response");
        assert_eq!(resp.finish_reason, FinishReason::Stop);
    }

    #[test]
    fn test_mock_provider_token_count_default() {
        let provider = MockProvider;
        // default impl: text.len() / 4
        let text = "abcdefgh"; // 8 chars → 2
        assert_eq!(provider.token_count(text), 2);

        let empty = "";
        assert_eq!(provider.token_count(empty), 0);
    }

    #[test]
    fn test_mock_provider_provider_id() {
        let provider = MockProvider;
        assert_eq!(provider.provider_id(), "mock");
        assert_eq!(provider.model_id(), "mock-v1");
    }

    #[test]
    fn test_finish_reason_serialize() {
        let reason = FinishReason::Stop;
        let json = serde_json::to_string(&reason).expect("serialize failed");
        assert_eq!(json, "\"stop\"");

        let tool_calls = FinishReason::ToolCalls;
        let json2 = serde_json::to_string(&tool_calls).expect("serialize failed");
        assert_eq!(json2, "\"tool_calls\"");

        let restored: FinishReason =
            serde_json::from_str("\"length\"").expect("deserialize failed");
        assert_eq!(restored, FinishReason::Length);
    }

    #[test]
    fn test_token_usage_default() {
        let usage = TokenUsage::default();
        assert_eq!(usage.prompt_tokens, 0);
        assert_eq!(usage.completion_tokens, 0);
        assert_eq!(usage.total_tokens, 0);
    }
}
