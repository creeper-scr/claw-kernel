use std::pin::Pin;
use std::sync::Arc;

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
///
/// # Examples
///
/// This trait is typically implemented for marker types that define the wire format:
///
/// ```rust
/// use claw_provider::{MessageFormat, Message, Options, CompletionResponse, Delta};
/// use serde::{Serialize, Deserialize};
///
/// // Define request/response types for a custom provider
/// #[derive(Serialize)]
/// struct CustomRequest {
///     model: String,
///     messages: Vec<CustomMessage>,
/// }
///
/// #[derive(Serialize)]
/// struct CustomMessage {
///     role: String,
///     content: String,
/// }
///
/// #[derive(Deserialize)]
/// struct CustomResponse {
///     text: String,
/// }
///
/// #[derive(Deserialize)]
/// struct CustomChunk;
///
/// struct CustomFormat;
///
/// #[derive(Debug)]
/// struct CustomError;
///
/// impl std::fmt::Display for CustomError {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         write!(f, "custom format error")
///     }
/// }
///
/// impl std::error::Error for CustomError {}
///
/// impl MessageFormat for CustomFormat {
///     type Request = CustomRequest;
///     type Response = CustomResponse;
///     type StreamChunk = CustomChunk;
///     type Error = CustomError;
///
///     fn build_request(messages: &[Message], opts: &Options) -> Self::Request {
///         CustomRequest {
///             model: opts.model.clone(),
///             messages: messages.iter().map(|m| CustomMessage {
///                 role: format!("{:?}", m.role).to_lowercase(),
///                 content: m.content.clone(),
///             }).collect(),
///         }
///     }
///
///     fn parse_response(_raw: Self::Response) -> Result<CompletionResponse, Self::Error> {
///         // Parse provider-specific response into canonical format
///         unimplemented!()
///     }
///
///     fn parse_stream_chunk(_chunk: &[u8]) -> Result<Option<Delta>, Self::Error> {
///         // Parse SSE/NDJSON chunk
///         Ok(None)
///     }
///
///     fn token_count(_messages: &[Message]) -> usize {
///         // Estimate tokens for the request
///         0
///     }
///
///     fn endpoint() -> &'static str {
///         "/v1/chat/completions"
///     }
/// }
/// ```
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
///
/// # Examples
///
/// Using the default HTTP transport:
///
/// ```rust
/// use claw_provider::DefaultHttpTransport;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create a transport with default settings
/// let transport = DefaultHttpTransport::new("https://api.example.com");
///
/// // Add authentication if needed
/// let transport = transport.with_auth("your-api-key").expect("valid token");
///
/// // The transport can now be used with any provider
/// // let provider = OpenAIProvider::new(transport, "gpt-4o");
/// # Ok(())
/// # }
/// ```
///
/// Implementing a custom transport for testing:
///
/// ```rust
/// use claw_provider::{HttpTransport, ProviderError};
/// use async_trait::async_trait;
/// use std::pin::Pin;
/// use futures::Stream;
///
/// struct MockTransport;
///
/// #[async_trait]
/// impl HttpTransport for MockTransport {
///     async fn post_json(
///         &self,
///         _url: &str,
///         _headers: &[(&str, &str)],
///         _body: &serde_json::Value,
///     ) -> Result<serde_json::Value, ProviderError> {
///         // Return mock response for testing
///         Ok(serde_json::json!({"id": "mock", "content": "test"}))
///     }
///
///     async fn post_stream(
///         &self,
///         _url: &str,
///         _headers: &[(&str, &str)],
///         _body: &serde_json::Value,
///     ) -> Result<Pin<Box<dyn Stream<Item = Result<bytes::Bytes, ProviderError>> + Send>>, ProviderError> {
///         use futures::stream;
///         Ok(Box::pin(stream::empty()))
///     }
/// }
/// ```
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
///
/// # Examples
///
/// The `HttpTransportExt` trait provides generic request methods that work
/// with any `MessageFormat` implementation:
///
/// ```rust,no_run
/// use claw_provider::traits::{HttpTransportExt, MessageFormat};
/// use claw_provider::types::{Message, Options, CompletionResponse};
///
/// // These methods are available on any type implementing HttpTransportExt
/// async fn example<T, F>(transport: T) -> Result<CompletionResponse, claw_provider::ProviderError>
/// where
///     T: HttpTransportExt,
///     F: MessageFormat,
///     <F as MessageFormat>::Request: Send,
///     <F as MessageFormat>::Error: std::error::Error + Send + Sync + 'static,
/// {
///     let messages = vec![Message::user("Hello")];
///     let opts = Options::new("model");
///     
///     // Generic request using MessageFormat
///     let response = transport.request::<F>(&messages, &opts).await?;
///     
///     // Streaming request
///     let _stream = transport.stream_request::<F>(&messages, &opts).await?;
    ///     Ok(response)
/// }
/// ```
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

/// Result of a provider health check.
///
/// Returned by [`LLMProvider::health_check`] to indicate whether the
/// provider is currently reachable and performing normally.
///
/// # Examples
///
/// ```rust
/// use claw_provider::traits::ProviderHealth;
///
/// let healthy = ProviderHealth::Healthy { latency_ms: 42 };
/// let degraded = ProviderHealth::Degraded {
///     latency_ms: 1500,
///     reason: "rate limited".to_string(),
/// };
/// let unavailable = ProviderHealth::Unavailable {
///     reason: "connection refused".to_string(),
/// };
/// ```
#[derive(Debug, Clone)]
pub enum ProviderHealth {
    /// Provider is healthy and responsive.
    Healthy {
        /// Round-trip latency in milliseconds.
        latency_ms: u64,
    },
    /// Provider is responding but with degraded performance.
    Degraded {
        /// Round-trip latency in milliseconds.
        latency_ms: u64,
        /// Human-readable reason for degraded state.
        reason: String,
    },
    /// Provider is not available.
    Unavailable {
        /// Human-readable reason for unavailability.
        reason: String,
    },
}

/// High-level LLM provider interface (Level 3: User-facing).
///
/// This is the main trait for interacting with LLM providers. Implementations
/// are provided for OpenAI, Anthropic, Ollama, DeepSeek, and Moonshot.
///
/// # Examples
///
/// Using a built-in provider:
///
/// ```rust,no_run
/// use claw_provider::{LLMProvider, OllamaProvider, Message, Options};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create a provider instance
/// let provider = OllamaProvider::new("llama3.2:latest")
///     .with_base_url("http://localhost:11434");
///
/// // Prepare messages and options
/// let messages = vec![
///     Message::system("You are a helpful assistant."),
///     Message::user("What is Rust?"),
/// ];
/// let options = Options::new("llama3.2:latest")
///     .with_max_tokens(1024)
///     .with_temperature(0.7)?;
///
/// // Get completion
/// let response = provider.complete(messages, options).await?;
/// println!("Response: {}", response.message.content);
/// # Ok(())
/// # }
/// ```
///
/// Using Anthropic provider with environment-based setup:
///
/// ```rust,no_run
/// use claw_provider::{AnthropicProvider, traits::LLMProvider};
/// use claw_provider::types::{Message, Options};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let provider = AnthropicProvider::from_env()?;
///     let messages = vec![Message::user("Hello")];
///     let opts = Options::new("claude-3-opus-20240229");
///     let response = provider.complete(messages, opts).await?;
///     println!("{}", response.message.content);
///     Ok(())
/// }
/// ```
///
/// Implementing a custom provider:
///
/// ```rust
/// use claw_provider::{LLMProvider, ProviderError, Message, Options, CompletionResponse, Delta};
/// use claw_provider::{TokenUsage, FinishReason};
/// use async_trait::async_trait;
/// use std::pin::Pin;
/// use futures::Stream;
///
/// struct MyProvider {
///     model: String,
/// }
///
/// #[async_trait]
/// impl LLMProvider for MyProvider {
///     fn provider_id(&self) -> &str {
///         "my_provider"
///     }
///
///     fn model_id(&self) -> &str {
///         &self.model
///     }
///
///     async fn complete(
///         &self,
///         _messages: Vec<Message>,
///         _options: Options,
///     ) -> Result<CompletionResponse, ProviderError> {
///         // Implement API call logic here
///         Ok(CompletionResponse {
///             id: "resp-123".to_string(),
///             model: self.model.clone(),
///             message: Message::assistant("Hello from my provider!"),
///             finish_reason: FinishReason::Stop,
///             usage: TokenUsage::new(10, 5),
///         })
///     }
///
///     async fn complete_stream(
///         &self,
///         _messages: Vec<Message>,
///         _options: Options,
///     ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError> {
///         // Implement streaming logic here
///         use futures::stream;
///         Ok(Box::pin(stream::empty()))
///     }
/// }
///
/// // The default token_count implementation uses chars/4 estimation
/// let provider = MyProvider { model: "custom-model".to_string() };
/// assert_eq!(provider.token_count("Hello world"), 2); // 11 chars / 4 = 2
/// ```
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

    /// Rough token count estimate, CJK-aware.
    /// ASCII text: ~4 chars per token; CJK/emoji: ~1 char per token.
    fn token_count(&self, text: &str) -> usize {
        let ascii_chars = text.chars().filter(|c| c.is_ascii()).count();
        let non_ascii = text.chars().count() - ascii_chars;
        // ASCII text: ~4 chars per token; CJK/emoji: ~1 char per token
        (ascii_chars / 4) + non_ascii
    }

    /// Check the health of this provider.
    ///
    /// The default implementation always returns [`ProviderHealth::Healthy`] with
    /// zero latency, preserving backward compatibility for existing implementations.
    /// Override this method to perform an actual liveness probe (e.g., a lightweight
    /// models-list or ping request).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use claw_provider::traits::{LLMProvider, ProviderHealth};
    ///
    /// # async fn example(provider: impl LLMProvider) {
    /// match provider.health_check().await {
    ///     ProviderHealth::Healthy { latency_ms } => println!("OK ({latency_ms}ms)"),
    ///     ProviderHealth::Degraded { latency_ms, reason } => {
    ///         println!("Degraded ({latency_ms}ms): {reason}")
    ///     }
    ///     ProviderHealth::Unavailable { reason } => println!("Unavailable: {reason}"),
    /// }
    /// # }
    /// ```
    async fn health_check(&self) -> ProviderHealth {
        ProviderHealth::Healthy { latency_ms: 0 }
    }
}

/// Provider that generates embedding vectors.
///
/// Embedding providers convert text into dense vector representations
/// suitable for semantic search and similarity comparisons.
///
/// # Examples
///
/// Using the built-in n-gram embedding provider:
///
/// ```rust
/// use claw_provider::{EmbeddingProvider, NgramEmbeddingProvider};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create a provider with default 64-dimensional embeddings
/// let provider = NgramEmbeddingProvider::new();
///
/// // Embed a single text
/// let embedding = provider.embed("Hello world").await?;
/// assert_eq!(embedding.len(), 64);
///
/// // Embed multiple texts in batch
/// let texts = vec![
///     "First document".to_string(),
///     "Second document".to_string(),
/// ];
/// let embeddings = provider.embed_batch(texts).await?;
/// assert_eq!(embeddings.len(), 2);
/// assert_eq!(embeddings[0].len(), 64);
/// # Ok(())
/// # }
/// ```
///
/// Implementing a custom embedding provider:
///
/// ```rust
/// use claw_provider::{EmbeddingProvider, ProviderError, Embedding};
/// use async_trait::async_trait;
///
/// struct SimpleEmbeddingProvider;
///
/// #[async_trait]
/// impl EmbeddingProvider for SimpleEmbeddingProvider {
///     async fn embed(&self, text: &str) -> Result<Embedding, ProviderError> {
///         // Simple character-based embedding for demonstration
///         let mut vec = vec![0.0f32; self.dimensions()];
///         for (i, byte) in text.bytes().enumerate().take(self.dimensions()) {
///             vec[i] = byte as f32 / 255.0;
///         }
///         Ok(vec)
///     }
///
///     async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>, ProviderError> {
///         let mut results = Vec::new();
///         for text in texts {
///             results.push(self.embed(&text).await?);
///         }
///         Ok(results)
///     }
///
///     fn dimensions(&self) -> usize {
///         128
///     }
/// }
/// ```
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a single text.
    async fn embed(&self, text: &str) -> Result<Embedding, ProviderError>;

    /// Embed a batch of texts.
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>, ProviderError>;

    /// Embedding vector dimensions.
    fn dimensions(&self) -> usize;
}

/// Event publisher for LLM provider events.
///
/// This trait allows Layer 1 (claw-runtime) to inject EventBus capabilities
/// into Layer 2 (claw-provider) without creating a circular dependency.
///
/// Provider implementations use this trait to publish events when:
/// - An LLM request is sent
/// - An LLM response is received
/// - A streaming chunk is received
/// - An error occurs
///
/// # Example
///
/// ```rust,ignore
/// use claw_provider::ProviderEventPublisher;
/// use claw_runtime::{EventBus, events::Event, agent_types::AgentId};
/// use std::sync::Arc;
///
/// struct RuntimeProviderEventPublisher {
///     event_bus: Arc<EventBus>,
/// }
///
/// impl ProviderEventPublisher for RuntimeProviderEventPublisher {
///     fn publish_request_started(&self, agent_id: &str, provider: &str, model: &str) {
///         let _ = self.event_bus.publish(Event::LlmRequestStarted {
///             agent_id: AgentId::new(agent_id),
///             provider: provider.to_string(),
///         });
///     }
///     // ... other methods
/// }
/// ```
pub trait ProviderEventPublisher: Send + Sync {
    /// Publish event when an LLM request is started.
    fn publish_request_started(&self, agent_id: &str, provider: &str, model: &str);

    /// Publish event when an LLM response is received.
    fn publish_response_completed(
        &self,
        agent_id: &str,
        provider: &str,
        model: &str,
        tokens_used: u64,
    );

    /// Publish event when a streaming chunk is received.
    fn publish_stream_chunk(&self, agent_id: &str, provider: &str, chunk_index: usize);

    /// Publish event when an error occurs.
    fn publish_error(&self, agent_id: &str, provider: &str, error_code: &str);
}

/// No-op event publisher for testing or when event publishing is not needed.
pub struct NoopProviderEventPublisher;

impl ProviderEventPublisher for NoopProviderEventPublisher {
    fn publish_request_started(&self, _agent_id: &str, _provider: &str, _model: &str) {}

    fn publish_response_completed(
        &self,
        _agent_id: &str,
        _provider: &str,
        _model: &str,
        _tokens_used: u64,
    ) {
    }

    fn publish_stream_chunk(&self, _agent_id: &str, _provider: &str, _chunk_index: usize) {}

    fn publish_error(&self, _agent_id: &str, _provider: &str, _error_code: &str) {}
}

impl NoopProviderEventPublisher {
    /// Create a new no-op event publisher wrapped in Arc.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Arc<dyn ProviderEventPublisher> {
        Arc::new(NoopProviderEventPublisher)
    }
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
