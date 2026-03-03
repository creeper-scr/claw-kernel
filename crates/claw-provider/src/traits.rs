use std::pin::Pin;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
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
    ///
    /// Sends a streaming request to the provider and returns a stream of `Delta` objects.
    /// The SSE (Server-Sent Events) response is parsed incrementally using the provided
    /// `MessageFormat::parse_stream_chunk` method.
    #[allow(clippy::type_complexity)]
    fn stream_request<F: MessageFormat>(
        &self,
        messages: &[Message],
        opts: &Options,
    ) -> impl std::future::Future<
        Output = Result<
            Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>,
            ProviderError,
        >,
    > + Send
    where
        <F as MessageFormat>::Request: Send,
        <F as MessageFormat>::Error: std::error::Error + Send + Sync + 'static,
    {
        async move {
            // Build the streaming request with stream enabled
            let mut stream_opts = opts.clone();
            stream_opts.stream = true;

            let req = F::build_request(messages, &stream_opts);
            let url = format!("{}{}", self.base_url(), F::endpoint());

            let body = serde_json::to_value(&req).map_err(|e| {
                ProviderError::Serialization(format!("Failed to serialize request: {}", e))
            })?;

            // Get the raw byte stream from the transport
            let byte_stream = self.post_stream(&url, &[], &body).await?;

            // Convert byte stream to Delta stream by parsing SSE events
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

                                // Parse the line using the format-specific parser
                                match F::parse_stream_chunk(trimmed.as_bytes()) {
                                    Ok(Some(delta)) => Some(Ok(delta)),
                                    Ok(None) => None, // End of stream marker like [DONE]
                                    Err(e) => {
                                        Some(Err(ProviderError::Serialization(e.to_string())))
                                    }
                                }
                            })
                            .collect()
                    }
                };
                futures::stream::iter(deltas)
            });

            Ok(Box::pin(delta_stream)
                as Pin<
                    Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>,
                >)
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
    use futures::stream;
    use serde::Deserialize;

    /// Mock MessageFormat for testing stream_request
    struct MockFormat;

    #[derive(Debug)]
    struct MockFormatError;

    impl std::fmt::Display for MockFormatError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "mock format error")
        }
    }

    impl std::error::Error for MockFormatError {}

    #[derive(Serialize, Deserialize)]
    struct MockRequest {
        model: String,
        messages: Vec<String>,
    }

    #[derive(Deserialize)]
    struct MockResponse {
        #[allow(dead_code)]
        id: String,
    }

    #[derive(Deserialize)]
    struct MockStreamChunk;

    impl MessageFormat for MockFormat {
        type Request = MockRequest;
        type Response = MockResponse;
        type StreamChunk = MockStreamChunk;
        type Error = MockFormatError;

        fn build_request(messages: &[Message], opts: &Options) -> Self::Request {
            MockRequest {
                model: opts.model.clone(),
                messages: messages.iter().map(|m| m.content.clone()).collect(),
            }
        }

        fn parse_response(_raw: Self::Response) -> Result<CompletionResponse, Self::Error> {
            unimplemented!()
        }

        fn parse_stream_chunk(chunk: &[u8]) -> Result<Option<Delta>, Self::Error> {
            let text = String::from_utf8_lossy(chunk);
            if text.contains("[DONE]") {
                return Ok(None);
            }
            if text.starts_with("data: ") {
                let content = text.strip_prefix("data: ").unwrap_or("").trim();
                if content == "[DONE]" {
                    return Ok(None);
                }
                return Ok(Some(Delta {
                    content: Some(content.to_string()),
                    tool_call: None,
                    finish_reason: None,
                    usage: None,
                }));
            }
            Ok(None)
        }

        fn token_count(_messages: &[Message]) -> usize {
            0
        }

        fn endpoint() -> &'static str {
            "/v1/chat"
        }
    }

    /// Mock HttpTransport for testing - successful responses only
    struct MockHttpTransport {
        chunks: Vec<bytes::Bytes>,
    }

    #[async_trait]
    impl HttpTransport for MockHttpTransport {
        async fn post_json(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            _body: &serde_json::Value,
        ) -> Result<serde_json::Value, ProviderError> {
            unimplemented!()
        }

        async fn post_stream(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            _body: &serde_json::Value,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<bytes::Bytes, ProviderError>> + Send>>,
            ProviderError,
        > {
            let chunks: Vec<_> = self.chunks.clone().into_iter().map(Ok).collect();
            Ok(Box::pin(stream::iter(chunks)))
        }
    }

    impl HttpTransportExt for MockHttpTransport {
        fn base_url(&self) -> &str {
            "https://api.test.com"
        }

        fn auth_headers(&self) -> reqwest::header::HeaderMap {
            reqwest::header::HeaderMap::new()
        }

        fn http_client(&self) -> &reqwest::Client {
            unimplemented!()
        }
    }

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

    #[tokio::test]
    async fn test_stream_request_single_chunk() {
        // Test parsing a single SSE chunk
        let chunks = vec![bytes::Bytes::from("data: hello world\n\n")];
        let transport = MockHttpTransport { chunks };
        let messages = vec![Message::user("test")];
        let opts = Options::new("test-model");

        let stream = transport
            .stream_request::<MockFormat>(&messages, &opts)
            .await
            .unwrap();
        let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

        assert_eq!(deltas.len(), 1);
        assert_eq!(
            deltas[0].as_ref().unwrap().content,
            Some("hello world".to_string())
        );
    }

    #[tokio::test]
    async fn test_stream_request_multiple_chunks() {
        // Test parsing multiple SSE chunks
        let chunks = vec![
            bytes::Bytes::from("data: hello\n\n"),
            bytes::Bytes::from("data: world\n\n"),
            bytes::Bytes::from("data: !\n\n"),
        ];
        let transport = MockHttpTransport { chunks };
        let messages = vec![Message::user("test")];
        let opts = Options::new("test-model");

        let stream = transport
            .stream_request::<MockFormat>(&messages, &opts)
            .await
            .unwrap();
        let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

        assert_eq!(deltas.len(), 3);
        assert_eq!(
            deltas[0].as_ref().unwrap().content,
            Some("hello".to_string())
        );
        assert_eq!(
            deltas[1].as_ref().unwrap().content,
            Some("world".to_string())
        );
        assert_eq!(deltas[2].as_ref().unwrap().content, Some("!".to_string()));
    }

    #[tokio::test]
    async fn test_stream_request_with_done_marker() {
        // Test that [DONE] marker is handled correctly
        let chunks = vec![
            bytes::Bytes::from("data: hello\n\n"),
            bytes::Bytes::from("data: [DONE]\n\n"),
        ];
        let transport = MockHttpTransport { chunks };
        let messages = vec![Message::user("test")];
        let opts = Options::new("test-model");

        let stream = transport
            .stream_request::<MockFormat>(&messages, &opts)
            .await
            .unwrap();
        let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

        // [DONE] should produce no delta
        assert_eq!(deltas.len(), 1);
        assert_eq!(
            deltas[0].as_ref().unwrap().content,
            Some("hello".to_string())
        );
    }

    #[tokio::test]
    async fn test_stream_request_empty_lines_filtered() {
        // Test that empty lines are filtered out
        let chunks = vec![bytes::Bytes::from("data: hello\n\n\n\ndata: world\n\n")];
        let transport = MockHttpTransport { chunks };
        let messages = vec![Message::user("test")];
        let opts = Options::new("test-model");

        let stream = transport
            .stream_request::<MockFormat>(&messages, &opts)
            .await
            .unwrap();
        let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

        assert_eq!(deltas.len(), 2);
        assert_eq!(
            deltas[0].as_ref().unwrap().content,
            Some("hello".to_string())
        );
        assert_eq!(
            deltas[1].as_ref().unwrap().content,
            Some("world".to_string())
        );
    }
}
