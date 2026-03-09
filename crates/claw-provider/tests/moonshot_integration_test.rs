//! Integration tests for MoonshotProvider using mock HTTP transport.
//!
//! These tests verify the MoonshotProvider's behavior without requiring
//! real API keys or network access.

#![cfg(feature = "test-utils")]

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use claw_provider::{
    CompletionResponse, Delta, HttpTransport, LLMProvider, Message, MoonshotProvider, Options,
    ProviderError,
};
use futures::{stream, Stream, StreamExt};

// ============================================================================
// MockHttpTransport Implementation
// ============================================================================

/// Mock HTTP transport for testing MoonshotProvider.
/// Allows predefined responses for both JSON and streaming requests.
struct MockHttpTransport {
    /// Response to return from post_json
    json_response: Option<Result<serde_json::Value, ProviderError>>,
    /// Chunks to return from post_stream
    stream_chunks: Vec<Result<Bytes, ProviderError>>,
}

impl MockHttpTransport {
    /// Create a mock transport that returns a successful JSON response.
    fn with_json_response(response: serde_json::Value) -> Self {
        Self {
            json_response: Some(Ok(response)),
            stream_chunks: Vec::new(),
        }
    }

    /// Create a mock transport that returns a JSON error.
    fn with_json_error(error: ProviderError) -> Self {
        Self {
            json_response: Some(Err(error)),
            stream_chunks: Vec::new(),
        }
    }

    /// Create a mock transport that returns streaming chunks.
    fn with_stream_chunks(chunks: Vec<Result<Bytes, ProviderError>>) -> Self {
        Self {
            json_response: None,
            stream_chunks: chunks,
        }
    }
}

#[async_trait]
impl HttpTransport for MockHttpTransport {
    async fn post_json(
        &self,
        _url: &str,
        _headers: &[(&str, &str)],
        _body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        match &self.json_response {
            Some(result) => result.clone(),
            None => Err(ProviderError::Other("No mock response configured".into())),
        }
    }

    async fn post_stream(
        &self,
        _url: &str,
        _headers: &[(&str, &str)],
        _body: &serde_json::Value,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send>>, ProviderError>
    {
        let chunks: Vec<_> = self.stream_chunks.clone();
        Ok(Box::pin(stream::iter(chunks)))
    }
}

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a typical Moonshot (OpenAI-compatible) API success response.
fn moonshot_success_response() -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-test123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "moonshot-v1-8k",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Hello! How can I help you today?"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 8,
            "total_tokens": 18
        }
    })
}

/// Create a Moonshot API response with tool calls.
fn moonshot_tool_calls_response() -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-tool123",
        "object": "chat.completion",
        "created": 1677652289,
        "model": "moonshot-v1-8k",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_abc123",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"location\": \"Beijing\", \"unit\": \"celsius\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 25,
            "completion_tokens": 20,
            "total_tokens": 45
        }
    })
}

/// Create streaming chunks for SSE response.
fn moonshot_stream_chunks() -> Vec<Result<Bytes, ProviderError>> {
    vec![
        Ok(Bytes::from(
            r#"data: {"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#,
        )),
        Ok(Bytes::from(
            r#"data: {"choices":[{"delta":{"content":"!"},"finish_reason":null}]}"#,
        )),
        Ok(Bytes::from(
            r#"data: {"choices":[{"delta":{"content":" How"},"finish_reason":null}]}"#,
        )),
        Ok(Bytes::from(
            r#"data: {"choices":[{"delta":{"content":" can"},"finish_reason":null}]}"#,
        )),
        Ok(Bytes::from(
            r#"data: {"choices":[{"delta":{"content":" I"},"finish_reason":null}]}"#,
        )),
        Ok(Bytes::from(
            r#"data: {"choices":[{"delta":{"content":" help"},"finish_reason":null}]}"#,
        )),
        Ok(Bytes::from(
            r#"data: {"choices":[{"delta":{"content":" you","finish_reason":"stop"},"usage":{"prompt_tokens":10,"completion_tokens":6,"total_tokens":16}}]}"#,
        )),
        Ok(Bytes::from("data: [DONE]")),
    ]
}

// ============================================================================
// Integration Tests
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_moonshot_complete_success() {
    let mock_response = moonshot_success_response();
    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        MoonshotProvider::with_transport("test-key", "moonshot-v1-8k", Arc::new(transport));

    let messages = vec![Message::user("Hello!")];
    let options = Options::new("moonshot-v1-8k");

    let result = provider.complete(messages, options).await;

    assert!(result.is_ok(), "Expected successful completion");

    let response: CompletionResponse = result.unwrap();
    assert_eq!(response.id, "chatcmpl-test123");
    assert_eq!(response.model, "moonshot-v1-8k");
    assert_eq!(response.message.content, "Hello! How can I help you today?");
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 8);
    assert_eq!(response.usage.total_tokens, 18);
}

#[tokio::test]
#[ignore]
async fn test_moonshot_complete_stream_success() {
    let chunks = moonshot_stream_chunks();
    let transport = MockHttpTransport::with_stream_chunks(chunks);
    let provider =
        MoonshotProvider::with_transport("test-key", "moonshot-v1-8k", Arc::new(transport));

    let messages = vec![Message::user("Hello!")];
    let options = Options::new("moonshot-v1-8k");

    let stream_result = provider.complete_stream(messages, options).await;
    assert!(stream_result.is_ok(), "Expected successful stream creation");

    let stream = stream_result.unwrap();
    let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

    // Filter out any empty results and errors
    let valid_deltas: Vec<&Delta> = deltas
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter(|d| d.content.is_some())
        .collect();

    // Should have 6 content deltas (Hello, !, How, can, I, help you)
    assert!(
        !valid_deltas.is_empty(),
        "Should receive at least one delta"
    );

    // Concatenate all content
    let full_content: String = valid_deltas
        .iter()
        .filter_map(|d| d.content.clone())
        .collect();

    assert_eq!(full_content, "Hello! How can I help you");
}

#[tokio::test]
#[ignore]
async fn test_moonshot_complete_with_tool_calls() {
    let mock_response = moonshot_tool_calls_response();
    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        MoonshotProvider::with_transport("test-key", "moonshot-v1-8k", Arc::new(transport));

    let messages = vec![Message::user("What's the weather in Beijing?")];
    let options = Options::new("moonshot-v1-8k");

    let result = provider.complete(messages, options).await;

    assert!(
        result.is_ok(),
        "Expected successful completion with tool calls"
    );

    let response: CompletionResponse = result.unwrap();
    assert_eq!(response.id, "chatcmpl-tool123");

    // Check tool calls
    assert!(
        response.message.tool_calls.is_some(),
        "Expected tool_calls to be present"
    );

    let tool_calls = response.message.tool_calls.unwrap();
    assert_eq!(tool_calls.len(), 1);

    let tool_call = &tool_calls[0];
    assert_eq!(tool_call.id, "call_abc123");
    assert_eq!(tool_call.name, "get_weather");
    assert!(tool_call.arguments.contains("Beijing"));
    assert!(tool_call.arguments.contains("celsius"));
}

#[tokio::test]
#[ignore]
async fn test_moonshot_complete_http_error_401() {
    let error = ProviderError::Http {
        status: 401,
        message: "Invalid API key".to_string(),
    };
    let transport = MockHttpTransport::with_json_error(error);
    let provider =
        MoonshotProvider::with_transport("test-key", "moonshot-v1-8k", Arc::new(transport));

    let messages = vec![Message::user("Hello!")];
    let options = Options::new("moonshot-v1-8k");

    let result = provider.complete(messages, options).await;

    assert!(result.is_err(), "Expected error for 401");

    match result.unwrap_err() {
        ProviderError::Http { status, message } => {
            assert_eq!(status, 401);
            assert_eq!(message, "Invalid API key");
        }
        other => panic!("Expected Http error, got: {:?}", other),
    }
}

#[tokio::test]
#[ignore]
async fn test_moonshot_complete_http_error_429() {
    let error = ProviderError::Http {
        status: 429,
        message: "Rate limited".to_string(),
    };
    let transport = MockHttpTransport::with_json_error(error);
    let provider =
        MoonshotProvider::with_transport("test-key", "moonshot-v1-8k", Arc::new(transport));

    let messages = vec![Message::user("Hello!")];
    let options = Options::new("moonshot-v1-8k");

    let result = provider.complete(messages, options).await;

    assert!(result.is_err(), "Expected error for 429");

    match result.unwrap_err() {
        ProviderError::Http { status, .. } => {
            assert_eq!(status, 429);
        }
        other => panic!("Expected Http error, got: {:?}", other),
    }
}

#[tokio::test]
#[ignore]
async fn test_moonshot_complete_http_error_500() {
    let error = ProviderError::Http {
        status: 500,
        message: "Internal server error".to_string(),
    };
    let transport = MockHttpTransport::with_json_error(error);
    let provider =
        MoonshotProvider::with_transport("test-key", "moonshot-v1-8k", Arc::new(transport));

    let messages = vec![Message::user("Hello!")];
    let options = Options::new("moonshot-v1-8k");

    let result = provider.complete(messages, options).await;

    assert!(result.is_err(), "Expected error for 500");

    match result.unwrap_err() {
        ProviderError::Http { status, .. } => {
            assert_eq!(status, 500);
        }
        other => panic!("Expected Http error, got: {:?}", other),
    }
}

#[tokio::test]
#[ignore]
async fn test_moonshot_provider_id_and_model() {
    let transport = MockHttpTransport::with_json_response(moonshot_success_response());
    let provider =
        MoonshotProvider::with_transport("test-key", "moonshot-v1-8k", Arc::new(transport));

    assert_eq!(provider.provider_id(), "moonshot");
    assert_eq!(provider.model_id(), "moonshot-v1-8k");
}

#[tokio::test]
#[ignore]
async fn test_moonshot_complete_with_system_message() {
    let mock_response = serde_json::json!({
        "id": "chatcmpl-sys123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "moonshot-v1-8k",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "I'm Claude, created by Anthropic."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 20,
            "completion_tokens": 10,
            "total_tokens": 30
        }
    });

    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        MoonshotProvider::with_transport("test-key", "moonshot-v1-8k", Arc::new(transport));

    let messages = vec![
        Message::system("You are Claude, a helpful AI assistant."),
        Message::user("Who are you?"),
    ];
    let options = Options::new("moonshot-v1-8k");

    let result = provider.complete(messages, options).await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(
        response.message.content,
        "I'm Claude, created by Anthropic."
    );
}

#[tokio::test]
#[ignore]
async fn test_moonshot_stream_with_empty_chunks() {
    // Test that empty lines and invalid chunks are handled gracefully
    let chunks = vec![
        Ok(Bytes::from("")),
        Ok(Bytes::from("\n\n")),
        Ok(Bytes::from(
            r#"data: {"choices":[{"delta":{"content":"test"},"finish_reason":null}]}"#,
        )),
        Ok(Bytes::from("data: [DONE]")),
    ];

    let transport = MockHttpTransport::with_stream_chunks(chunks);
    let provider =
        MoonshotProvider::with_transport("test-key", "moonshot-v1-8k", Arc::new(transport));

    let messages = vec![Message::user("Test")];
    let options = Options::new("moonshot-v1-8k");

    let stream_result = provider.complete_stream(messages, options).await;
    assert!(stream_result.is_ok());

    let stream = stream_result.unwrap();
    let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

    // Should only get the valid delta
    let valid_deltas: Vec<&Delta> = deltas
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter(|d| d.content.is_some())
        .collect();

    assert_eq!(valid_deltas.len(), 1);
    assert_eq!(valid_deltas[0].content, Some("test".to_string()));
}

#[tokio::test]
#[ignore]
async fn test_moonshot_token_count() {
    let transport = MockHttpTransport::with_json_response(moonshot_success_response());
    let provider =
        MoonshotProvider::with_transport("test-key", "moonshot-v1-8k", Arc::new(transport));

    // Default implementation: text.len() / 4
    assert_eq!(provider.token_count("hello"), 1); // 5 / 4 = 1
    assert_eq!(provider.token_count("hello world"), 2); // 11 / 4 = 2
    assert_eq!(provider.token_count(""), 0);
}
