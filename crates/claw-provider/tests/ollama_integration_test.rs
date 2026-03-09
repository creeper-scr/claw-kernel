//! Integration tests for OllamaProvider using mock HTTP transport.
//!
//! These tests verify:
//! - Normal complete() response handling
//! - Streaming complete_stream() response handling
//! - Error handling (connection failures, model not found)
//! - Custom base_url configuration

#![cfg(feature = "test-utils")]

use async_trait::async_trait;
use claw_provider::{HttpTransport, LLMProvider, Message, OllamaProvider, Options, ProviderError};
use futures::{stream, Stream};
use std::pin::Pin;
use std::sync::Arc;

/// Mock HTTP transport for testing OllamaProvider
struct MockHttpTransport {
    /// Response to return for post_json calls
    json_response: Option<serde_json::Value>,
    /// Error to return for post_json calls (if set)
    json_error: Option<ProviderError>,
    /// Chunks to return for post_stream calls
    stream_chunks: Vec<Result<bytes::Bytes, ProviderError>>,
    /// Error to return for post_stream calls (if set)
    stream_error: Option<ProviderError>,
    /// Record the URL that was called for verification
    last_url: std::sync::Mutex<Option<String>>,
}

impl MockHttpTransport {
    /// Create a mock transport that returns a successful JSON response
    fn with_json_response(response: serde_json::Value) -> Self {
        Self {
            json_response: Some(response),
            json_error: None,
            stream_chunks: Vec::new(),
            stream_error: None,
            last_url: std::sync::Mutex::new(None),
        }
    }

    /// Create a mock transport that returns an error for JSON requests
    fn with_json_error(error: ProviderError) -> Self {
        Self {
            json_response: None,
            json_error: Some(error),
            stream_chunks: Vec::new(),
            stream_error: None,
            last_url: std::sync::Mutex::new(None),
        }
    }

    /// Create a mock transport that returns streaming chunks
    fn with_stream_chunks(chunks: Vec<Result<bytes::Bytes, ProviderError>>) -> Self {
        Self {
            json_response: None,
            json_error: None,
            stream_chunks: chunks,
            stream_error: None,
            last_url: std::sync::Mutex::new(None),
        }
    }

    /// Create a mock transport that returns an error for streaming requests
    fn with_stream_error(error: ProviderError) -> Self {
        Self {
            json_response: None,
            json_error: None,
            stream_chunks: Vec::new(),
            stream_error: Some(error),
            last_url: std::sync::Mutex::new(None),
        }
    }

    /// Get the last URL that was called
    fn last_url(&self) -> Option<String> {
        self.last_url.lock().unwrap().clone()
    }
}

#[async_trait]
impl HttpTransport for MockHttpTransport {
    async fn post_json(
        &self,
        url: &str,
        _headers: &[(&str, &str)],
        _body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        *self.last_url.lock().unwrap() = Some(url.to_string());

        if let Some(ref error) = self.json_error {
            return Err(error.clone());
        }

        self.json_response
            .clone()
            .ok_or_else(|| ProviderError::Other("No mock response configured".to_string()))
    }

    async fn post_stream(
        &self,
        url: &str,
        _headers: &[(&str, &str)],
        _body: &serde_json::Value,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<bytes::Bytes, ProviderError>> + Send>>,
        ProviderError,
    > {
        *self.last_url.lock().unwrap() = Some(url.to_string());

        if let Some(ref error) = self.stream_error {
            return Err(error.clone());
        }

        let chunks = self.stream_chunks.clone();
        Ok(Box::pin(stream::iter(chunks)))
    }
}

/// Create a standard Ollama/OpenAI compatible success response
fn create_success_response(id: &str, content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "model": "llama3.2",
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    })
}

/// Create a streaming chunk in OpenAI SSE format
///
/// When finish_reason is Some, the content should typically be empty since
/// OpenAI sends the finish_reason in a separate chunk without content.
fn create_stream_chunk(content: &str, finish_reason: Option<&str>) -> bytes::Bytes {
    let json = if let Some(finish) = finish_reason {
        // Finish chunk: has finish_reason but no content (or empty content)
        if content.is_empty() {
            format!(
                r#"{{"choices":[{{"delta":{{}},"finish_reason":"{}"}}]}}"#,
                finish
            )
        } else {
            format!(
                r#"{{"choices":[{{"delta":{{"content":"{}"}},"finish_reason":"{}"}}]}}"#,
                content.replace('"', "\\\""),
                finish
            )
        }
    } else {
        // Normal content chunk
        format!(
            r#"{{"choices":[{{"delta":{{"content":"{}"}},"finish_reason":null}}]}}"#,
            content.replace('"', "\\\"")
        )
    };
    bytes::Bytes::from(format!("data: {}\n\n", json))
}

#[tokio::test]
#[ignore]
async fn test_complete_success() {
    // Setup mock transport with a successful response
    let mock_response = create_success_response("chatcmpl-123", "Hello! How can I help you?");
    let mock_transport = Arc::new(MockHttpTransport::with_json_response(mock_response));

    // Create provider with mock transport
    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport.clone());

    // Prepare request
    let messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("Hi there!"),
    ];
    let options = Options::new("llama3.2").with_max_tokens(100);

    // Execute request
    let result = provider.complete(messages, options).await;

    // Verify response
    assert!(result.is_ok(), "complete() should succeed");
    let response = result.unwrap();

    assert_eq!(response.id, "chatcmpl-123");
    assert_eq!(response.model, "llama3.2");
    assert_eq!(response.message.content, "Hello! How can I help you?");
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(response.usage.total_tokens, 15);

    // Verify the URL was called correctly
    let last_url = mock_transport
        .last_url()
        .expect("URL should have been called");
    assert!(
        last_url.contains("/v1/chat/completions"),
        "Should call chat completions endpoint"
    );
    assert!(
        last_url.starts_with("http://localhost:11434"),
        "Should use default base URL"
    );
}

#[tokio::test]
#[ignore]
async fn test_complete_stream_success() {
    use futures::StreamExt;

    // Setup mock transport with streaming chunks
    // Note: The last chunk with finish_reason has no content in OpenAI format
    let chunks = vec![
        Ok(create_stream_chunk("Hello", None)),
        Ok(create_stream_chunk("!", None)),
        Ok(create_stream_chunk(" How", None)),
        Ok(create_stream_chunk(" can", None)),
        Ok(create_stream_chunk(" I", None)),
        Ok(create_stream_chunk(" help", None)),
        Ok(create_stream_chunk("?", None)),
        Ok(create_stream_chunk("", Some("stop"))), // finish chunk, no content
    ];
    let mock_transport = Arc::new(MockHttpTransport::with_stream_chunks(chunks));

    // Create provider with mock transport
    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport.clone());

    // Prepare request
    let messages = vec![Message::user("Hi there!")];
    let options = Options::new("llama3.2");

    // Execute streaming request
    let result = provider.complete_stream(messages, options).await;

    // Verify stream setup succeeded
    assert!(result.is_ok(), "complete_stream() should succeed");

    // Collect all deltas from the stream
    let mut stream = result.unwrap();
    let mut contents: Vec<String> = Vec::new();
    let mut finish_reasons: Vec<Option<claw_provider::FinishReason>> = Vec::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(delta) => {
                if let Some(content) = delta.content {
                    contents.push(content);
                }
                finish_reasons.push(delta.finish_reason);
            }
            Err(e) => panic!("Unexpected error in stream: {:?}", e),
        }
    }

    // Verify collected content
    let full_content: String = contents.join("");
    assert_eq!(full_content, "Hello! How can I help?");

    // Verify finish reason on last chunk
    let last_finish = finish_reasons.last().unwrap();
    assert!(
        last_finish.is_some(),
        "Last chunk should have finish_reason"
    );

    // Verify the URL was called correctly
    let last_url = mock_transport
        .last_url()
        .expect("URL should have been called");
    assert!(last_url.contains("/v1/chat/completions"));
}

#[tokio::test]
#[ignore]
async fn test_complete_network_error() {
    // Setup mock transport with a network error
    let error = ProviderError::Network("Connection refused".to_string());
    let mock_transport = Arc::new(MockHttpTransport::with_json_error(error));

    // Create provider with mock transport
    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    // Prepare request
    let messages = vec![Message::user("Hi")];
    let options = Options::new("llama3.2");

    // Execute request
    let result = provider.complete(messages, options).await;

    // Verify error is propagated
    assert!(result.is_err(), "complete() should fail with network error");
    match result {
        Err(ProviderError::Network(msg)) => {
            assert_eq!(msg, "Connection refused");
        }
        Err(other) => panic!("Expected Network error, got: {:?}", other),
        Ok(_) => panic!("Expected Network error, but got Ok"),
    }
}

#[tokio::test]
#[ignore]
async fn test_complete_model_not_found() {
    // Setup mock transport with model not found error
    let error = ProviderError::ModelNotFound("llama3.2".to_string());
    let mock_transport = Arc::new(MockHttpTransport::with_json_error(error));

    // Create provider with mock transport
    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    // Prepare request
    let messages = vec![Message::user("Hi")];
    let options = Options::new("llama3.2");

    // Execute request
    let result = provider.complete(messages, options).await;

    // Verify error is propagated
    assert!(
        result.is_err(),
        "complete() should fail with model not found"
    );
    match result {
        Err(ProviderError::ModelNotFound(model)) => {
            assert_eq!(model, "llama3.2");
        }
        Err(other) => panic!("Expected ModelNotFound error, got: {:?}", other),
        Ok(_) => panic!("Expected ModelNotFound error, but got Ok"),
    }
}

#[tokio::test]
#[ignore]
async fn test_complete_http_error() {
    // Setup mock transport with HTTP error
    let error = ProviderError::Http {
        status: 500,
        message: "Internal Server Error".to_string(),
    };
    let mock_transport = Arc::new(MockHttpTransport::with_json_error(error));

    // Create provider with mock transport
    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    // Prepare request
    let messages = vec![Message::user("Hi")];
    let options = Options::new("llama3.2");

    // Execute request
    let result = provider.complete(messages, options).await;

    // Verify error is propagated
    assert!(result.is_err(), "complete() should fail with HTTP error");
    match result {
        Err(ProviderError::Http { status, message }) => {
            assert_eq!(status, 500);
            assert_eq!(message, "Internal Server Error");
        }
        Err(other) => panic!("Expected Http error, got: {:?}", other),
        Ok(_) => panic!("Expected Http error, but got Ok"),
    }
}

#[tokio::test]
#[ignore]
async fn test_stream_error() {
    // Setup mock transport with streaming error
    let error = ProviderError::Stream("Connection reset".to_string());
    let mock_transport = Arc::new(MockHttpTransport::with_stream_error(error));

    // Create provider with mock transport
    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    // Prepare request
    let messages = vec![Message::user("Hi")];
    let options = Options::new("llama3.2");

    // Execute streaming request
    let result = provider.complete_stream(messages, options).await;

    // Verify error is propagated
    assert!(
        result.is_err(),
        "complete_stream() should fail with stream error"
    );
    match result {
        Err(ProviderError::Stream(msg)) => {
            assert_eq!(msg, "Connection reset");
        }
        Err(other) => panic!("Expected Stream error, got: {:?}", other),
        Ok(_) => panic!("Expected Stream error, but got Ok"),
    }
}

#[tokio::test]
#[ignore]
async fn test_custom_base_url() {
    // Setup mock transport with a successful response
    let mock_response = create_success_response("chatcmpl-456", "Response from custom URL");
    let mock_transport = Arc::new(MockHttpTransport::with_json_response(mock_response));

    // Create provider with custom base URL
    let provider = OllamaProvider::new("llama3.2")
        .with_base_url("http://custom-ollama:8080")
        .with_transport(mock_transport.clone());

    // Prepare request
    let messages = vec![Message::user("Hello")];
    let options = Options::new("llama3.2");

    // Execute request
    let result = provider.complete(messages, options).await;

    // Verify response
    assert!(result.is_ok());
    assert_eq!(result.unwrap().message.content, "Response from custom URL");

    // Verify the custom URL was called
    let last_url = mock_transport
        .last_url()
        .expect("URL should have been called");
    assert!(
        last_url.starts_with("http://custom-ollama:8080"),
        "Should use custom base URL, got: {}",
        last_url
    );
}

#[tokio::test]
#[ignore]
async fn test_custom_base_url_with_v1_suffix() {
    // Setup mock transport with a successful response
    let mock_response = create_success_response("chatcmpl-789", "Response");
    let mock_transport = Arc::new(MockHttpTransport::with_json_response(mock_response));

    // Create provider with custom base URL that has /v1 suffix (should be stripped)
    let provider = OllamaProvider::new("llama3.2")
        .with_base_url("http://custom-ollama:8080/v1")
        .with_transport(mock_transport.clone());

    // Prepare request
    let messages = vec![Message::user("Hello")];
    let options = Options::new("llama3.2");

    // Execute request
    let result = provider.complete(messages, options).await;

    // Verify response
    assert!(result.is_ok());

    // Verify the /v1 suffix was stripped and endpoint is correct
    let last_url = mock_transport
        .last_url()
        .expect("URL should have been called");
    assert_eq!(
        last_url, "http://custom-ollama:8080/v1/chat/completions",
        "Should strip /v1 suffix from base_url and use correct endpoint"
    );
}

#[tokio::test]
#[ignore]
async fn test_provider_metadata() {
    // Create a basic provider
    let provider = OllamaProvider::new("llama3.2:latest");

    // Verify provider metadata
    assert_eq!(provider.provider_id(), "ollama");
    assert_eq!(provider.model_id(), "llama3.2:latest");
}

#[tokio::test]
#[ignore]
async fn test_complete_with_system_message() {
    // Setup mock transport with a successful response
    let mock_response = create_success_response("chatcmpl-sys", "I am helpful!");
    let mock_transport = Arc::new(MockHttpTransport::with_json_response(mock_response));

    // Create provider with mock transport
    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    // Prepare request with system message in options
    let messages = vec![Message::user("Who are you?")];
    let options = Options::new("llama3.2").with_system("You are a helpful assistant.");

    // Execute request
    let result = provider.complete(messages, options).await;

    // Verify response
    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.message.content, "I am helpful!");
}

#[tokio::test]
#[ignore]
async fn test_stream_with_multiple_deltas() {
    use futures::StreamExt;

    // Setup mock transport with multiple streaming chunks simulating a longer response
    // Last chunk has finish_reason but no content
    let chunks = vec![
        Ok(create_stream_chunk("The", None)),
        Ok(create_stream_chunk(" quick", None)),
        Ok(create_stream_chunk(" brown", None)),
        Ok(create_stream_chunk(" fox", None)),
        Ok(create_stream_chunk(" jumps", None)),
        Ok(create_stream_chunk(" over", None)),
        Ok(create_stream_chunk(" the", None)),
        Ok(create_stream_chunk(" lazy", None)),
        Ok(create_stream_chunk(" dog", None)),
        Ok(create_stream_chunk(".", None)),
        Ok(create_stream_chunk("", Some("stop"))), // finish chunk
    ];
    let mock_transport = Arc::new(MockHttpTransport::with_stream_chunks(chunks));

    // Create provider with mock transport
    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    // Prepare request
    let messages = vec![Message::user("Complete the sentence")];
    let options = Options::new("llama3.2");

    // Execute streaming request
    let result = provider.complete_stream(messages, options).await;
    assert!(result.is_ok());

    // Collect all content
    let mut stream = result.unwrap();
    let mut full_content = String::new();
    let mut chunk_count = 0;

    while let Some(result) = stream.next().await {
        if let Ok(delta) = result {
            if let Some(content) = delta.content {
                full_content.push_str(&content);
                chunk_count += 1;
            }
        }
    }

    // Verify all chunks were received
    assert_eq!(full_content, "The quick brown fox jumps over the lazy dog.");
    assert_eq!(chunk_count, 10, "Should receive 10 content chunks");
}

#[tokio::test]
#[ignore]
async fn test_stream_empty_response() {
    use futures::StreamExt;

    // Setup mock transport with just the stop marker
    let chunks = vec![Ok(create_stream_chunk("", Some("stop")))];
    let mock_transport = Arc::new(MockHttpTransport::with_stream_chunks(chunks));

    // Create provider with mock transport
    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    // Prepare request
    let messages = vec![Message::user("Say nothing")];
    let options = Options::new("llama3.2");

    // Execute streaming request
    let result = provider.complete_stream(messages, options).await;
    assert!(result.is_ok());

    // Collect all content (should be minimal)
    let stream = result.unwrap();
    let deltas: Vec<_> = stream.collect().await;

    // Should have one delta with finish_reason but no content
    assert_eq!(deltas.len(), 1);
    let delta = deltas[0].as_ref().unwrap();
    assert!(delta.content.is_none() || delta.content.as_ref().unwrap().is_empty());
    assert!(delta.finish_reason.is_some());
}

#[tokio::test]
#[ignore]
async fn test_from_env() {
    // Set environment variables
    std::env::set_var("OLLAMA_MODEL", "test-model");
    std::env::set_var("OLLAMA_BASE_URL", "http://env-ollama:11434");

    // Create provider from environment
    let provider = OllamaProvider::from_env().expect("from_env should succeed");

    // Verify settings
    assert_eq!(provider.model_id(), "test-model");
    assert_eq!(provider.provider_id(), "ollama");

    // Cleanup
    std::env::remove_var("OLLAMA_MODEL");
    std::env::remove_var("OLLAMA_BASE_URL");
}

#[tokio::test]
#[ignore]
async fn test_from_env_defaults() {
    // Ensure env vars are not set
    std::env::remove_var("OLLAMA_MODEL");
    std::env::remove_var("OLLAMA_BASE_URL");

    // Create provider from environment (should use defaults)
    let provider = OllamaProvider::from_env().expect("from_env should succeed");

    // Verify default settings
    assert_eq!(provider.model_id(), "llama3"); // default model
    assert_eq!(provider.provider_id(), "ollama");
}

#[tokio::test]
#[ignore]
async fn test_stream_chunk_with_special_characters() {
    use futures::StreamExt;

    // Test that special characters in content are properly escaped
    let chunks = vec![
        Ok(create_stream_chunk("Hello \"world\"!", None)),
        Ok(create_stream_chunk(" It's great!", None)),
        Ok(create_stream_chunk("", Some("stop"))),
    ];
    let mock_transport = Arc::new(MockHttpTransport::with_stream_chunks(chunks));

    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    let messages = vec![Message::user("Greet with quotes")];
    let options = Options::new("llama3.2");

    let result = provider.complete_stream(messages, options).await;
    assert!(result.is_ok());

    let stream = result.unwrap();
    let mut full_content = String::new();

    let deltas: Vec<_> = stream.collect().await;
    for result in deltas {
        if let Ok(delta) = result {
            if let Some(content) = delta.content {
                full_content.push_str(&content);
            }
        }
    }

    assert_eq!(full_content, "Hello \"world\"! It's great!");
}

#[tokio::test]
#[ignore]
async fn test_rate_limit_error() {
    // Setup mock transport with rate limit error
    let error = ProviderError::RateLimited {
        retry_after_secs: 60,
    };
    let mock_transport = Arc::new(MockHttpTransport::with_json_error(error));

    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    let messages = vec![Message::user("Hi")];
    let options = Options::new("llama3.2");

    let result = provider.complete(messages, options).await;

    assert!(result.is_err());
    match result {
        Err(ProviderError::RateLimited { retry_after_secs }) => {
            assert_eq!(retry_after_secs, 60);
        }
        Err(other) => panic!("Expected RateLimited error, got: {:?}", other),
        Ok(_) => panic!("Expected RateLimited error, but got Ok"),
    }
}

#[tokio::test]
#[ignore]
async fn test_invalid_request_error() {
    // Setup mock transport with invalid request error
    let error = ProviderError::InvalidRequest("Missing required field: messages".to_string());
    let mock_transport = Arc::new(MockHttpTransport::with_json_error(error));

    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    let messages = vec![Message::user("Hi")];
    let options = Options::new("llama3.2");

    let result = provider.complete(messages, options).await;

    assert!(result.is_err());
    match result {
        Err(ProviderError::InvalidRequest(msg)) => {
            assert_eq!(msg, "Missing required field: messages");
        }
        Err(other) => panic!("Expected InvalidRequest error, got: {:?}", other),
        Ok(_) => panic!("Expected InvalidRequest error, but got Ok"),
    }
}

#[tokio::test]
#[ignore]
async fn test_context_length_exceeded() {
    // Setup mock transport with context length exceeded error
    let error = ProviderError::ContextLengthExceeded;
    let mock_transport = Arc::new(MockHttpTransport::with_json_error(error));

    let provider = OllamaProvider::new("llama3.2").with_transport(mock_transport);

    let messages = vec![Message::user("Very long message...")];
    let options = Options::new("llama3.2");

    let result = provider.complete(messages, options).await;

    assert!(result.is_err());
    match result {
        Err(ProviderError::ContextLengthExceeded) => {
            // Expected
        }
        Err(other) => panic!("Expected ContextLengthExceeded error, got: {:?}", other),
        Ok(_) => panic!("Expected ContextLengthExceeded error, but got Ok"),
    }
}
