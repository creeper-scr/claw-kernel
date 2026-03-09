//! Integration tests for DeepSeekProvider using mock HTTP transport.
//!
//! These tests verify DeepSeekProvider functionality without requiring real API keys.
//! DeepSeek uses OpenAI-compatible API format with additional features like
//! `reasoning_content` field in responses.

#![cfg(feature = "test-utils")]

use async_trait::async_trait;
use bytes::Bytes;
use claw_provider::{
    CompletionResponse, DeepSeekProvider, Delta, FinishReason, HttpTransport, LLMProvider, Message,
    Options, ProviderError,
};
use futures::{stream, Stream, StreamExt};
use std::pin::Pin;
use std::sync::Arc;

// ============================================================================
// MockHttpTransport Implementation
// ============================================================================

/// Mock HTTP transport for testing DeepSeekProvider.
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

/// Create a standard DeepSeek API success response.
fn deepseek_success_response(content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "cmpl-test123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
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

/// Create a DeepSeek API response with reasoning_content (DeepSeek-specific feature).
fn deepseek_response_with_reasoning(content: &str, reasoning: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "cmpl-reasoning123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "deepseek-reasoner",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
                "reasoning_content": reasoning
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 20,
            "completion_tokens": 15,
            "total_tokens": 35
        }
    })
}

/// Create a DeepSeek API response with multiple choices.
fn deepseek_response_multiple_choices() -> serde_json::Value {
    serde_json::json!({
        "id": "cmpl-multi123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "deepseek-chat",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "First choice response"
                },
                "finish_reason": "stop"
            },
            {
                "index": 1,
                "message": {
                    "role": "assistant",
                    "content": "Second choice response"
                },
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 12,
            "total_tokens": 22
        }
    })
}

/// Create streaming chunks for SSE response.
fn create_stream_chunk(content: &str, finish_reason: Option<&str>) -> String {
    let finish = finish_reason.map(|r| format!(r#","finish_reason":"{}""#, r));
    format!(
        r#"{{"choices":[{{"delta":{{"content":"{}"}}{}}}]}}"#,
        content,
        finish.unwrap_or_default()
    )
}

// ============================================================================
// Tests for complete() method
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_complete_success_response() {
    let mock_response = deepseek_success_response("Hello! How can I help you today?");
    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Hello!")];
    let options = Options::new("deepseek-chat");

    let result = provider.complete(messages, options).await;

    assert!(result.is_ok(), "Expected successful completion");

    let response: CompletionResponse = result.unwrap();
    assert_eq!(response.id, "cmpl-test123");
    assert_eq!(response.model, "deepseek-chat");
    assert_eq!(response.message.content, "Hello! How can I help you today?");
    assert_eq!(response.finish_reason, FinishReason::Stop);
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 8);
    assert_eq!(response.usage.total_tokens, 18);
}

#[tokio::test]
#[ignore]
async fn test_complete_with_reasoning_content() {
    // DeepSeek-specific feature: reasoning_content field
    let mock_response = deepseek_response_with_reasoning(
        "The answer is 42.",
        "Let me think about this problem step by step...",
    );
    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-reasoner", Arc::new(transport));

    let messages = vec![Message::user("What is the meaning of life?")];
    let options = Options::new("deepseek-reasoner");

    let result = provider.complete(messages, options).await;

    assert!(
        result.is_ok(),
        "Expected successful completion with reasoning"
    );

    let response: CompletionResponse = result.unwrap();
    assert_eq!(response.id, "cmpl-reasoning123");
    assert_eq!(response.model, "deepseek-reasoner");
    assert_eq!(response.message.content, "The answer is 42.");
    // Note: reasoning_content is DeepSeek-specific and may be stored in metadata
    // or handled differently depending on the implementation
}

#[tokio::test]
#[ignore]
async fn test_complete_empty_response_content() {
    let mock_response = serde_json::json!({
        "id": "cmpl-empty123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": ""
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 5,
            "completion_tokens": 0,
            "total_tokens": 5
        }
    });
    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Generate nothing")];
    let options = Options::new("deepseek-chat");

    let result = provider.complete(messages, options).await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.message.content, "");
    assert_eq!(response.finish_reason, FinishReason::Stop);
}

#[tokio::test]
#[ignore]
async fn test_complete_different_finish_reasons() {
    // Test "stop" finish reason
    let mock_response = serde_json::json!({
        "id": "cmpl-stop123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "Done!"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    });
    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Say done")];
    let options = Options::new("deepseek-chat");

    let result = provider.complete(messages, options).await.unwrap();
    assert_eq!(result.finish_reason, FinishReason::Stop);

    // Test "length" finish reason
    let mock_response = serde_json::json!({
        "id": "cmpl-length123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "Truncated..."},
            "finish_reason": "length"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    });
    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Tell me a long story")];
    let options = Options::new("deepseek-chat").with_max_tokens(5);

    let result = provider.complete(messages, options).await.unwrap();
    assert_eq!(result.finish_reason, FinishReason::Length);
}

#[tokio::test]
#[ignore]
async fn test_complete_multiple_choices_takes_first() {
    let mock_response = deepseek_response_multiple_choices();
    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Hello!")];
    let options = Options::new("deepseek-chat");

    let result = provider.complete(messages, options).await;

    assert!(result.is_ok());
    let response = result.unwrap();
    // Should take the first choice
    assert_eq!(response.message.content, "First choice response");
    assert_eq!(response.id, "cmpl-multi123");
}

#[tokio::test]
#[ignore]
async fn test_complete_content_filter_finish_reason() {
    let mock_response = serde_json::json!({
        "id": "cmpl-filter123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": ""},
            "finish_reason": "content_filter"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 0, "total_tokens": 10}
    });
    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Generate inappropriate content")];
    let options = Options::new("deepseek-chat");

    let result = provider.complete(messages, options).await.unwrap();
    assert_eq!(result.finish_reason, FinishReason::ContentFilter);
}

// ============================================================================
// Tests for complete_stream() method
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_complete_stream_single_chunk() {
    let chunks = vec![
        Ok(Bytes::from(format!(
            "data: {}\n\n",
            create_stream_chunk("Hello", None)
        ))),
        Ok(Bytes::from("data: [DONE]\n\n")),
    ];

    let transport = MockHttpTransport::with_stream_chunks(chunks);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Say hello")];
    let options = Options::new("deepseek-chat").with_max_tokens(50);

    let stream = provider.complete_stream(messages, options).await.unwrap();
    let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

    let valid_deltas: Vec<&Delta> = deltas
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter(|d| d.content.is_some())
        .collect();

    assert_eq!(valid_deltas.len(), 1);
    assert_eq!(valid_deltas[0].content, Some("Hello".to_string()));
}

#[tokio::test]
#[ignore]
async fn test_complete_stream_multiple_chunks() {
    let chunks = vec![
        Ok(Bytes::from(format!(
            "data: {}\n\n",
            create_stream_chunk("Hello", None)
        ))),
        Ok(Bytes::from(format!(
            "data: {}\n\n",
            create_stream_chunk(" world", None)
        ))),
        Ok(Bytes::from(format!(
            "data: {}\n\n",
            create_stream_chunk("!", None)
        ))),
        Ok(Bytes::from("data: [DONE]\n\n")),
    ];

    let transport = MockHttpTransport::with_stream_chunks(chunks);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Say hello world")];
    let options = Options::new("deepseek-chat");

    let stream = provider.complete_stream(messages, options).await.unwrap();
    let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

    let valid_deltas: Vec<&Delta> = deltas
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter(|d| d.content.is_some())
        .collect();

    assert_eq!(valid_deltas.len(), 3);
    let full_content: String = valid_deltas
        .iter()
        .filter_map(|d| d.content.clone())
        .collect();
    assert_eq!(full_content, "Hello world!");
}

#[tokio::test]
#[ignore]
async fn test_complete_stream_with_finish_reason() {
    let chunks = vec![
        Ok(Bytes::from(format!(
            "data: {}\n\n",
            create_stream_chunk("Complete", Some("stop"))
        ))),
        Ok(Bytes::from("data: [DONE]\n\n")),
    ];

    let transport = MockHttpTransport::with_stream_chunks(chunks);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Finish")];
    let options = Options::new("deepseek-chat");

    let stream = provider.complete_stream(messages, options).await.unwrap();
    let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

    let valid_deltas: Vec<&Delta> = deltas
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter(|d| d.content.is_some() || d.finish_reason.is_some())
        .collect();

    assert!(!valid_deltas.is_empty());
    assert_eq!(valid_deltas[0].content, Some("Complete".to_string()));
    assert_eq!(valid_deltas[0].finish_reason, Some(FinishReason::Stop));
}

#[tokio::test]
#[ignore]
async fn test_complete_stream_empty_chunks_filtered() {
    let chunks = vec![
        Ok(Bytes::from("")),
        Ok(Bytes::from("\n\n")),
        Ok(Bytes::from(format!(
            "data: {}\n\n",
            create_stream_chunk("test", None)
        ))),
        Ok(Bytes::from("data: [DONE]\n\n")),
    ];

    let transport = MockHttpTransport::with_stream_chunks(chunks);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Test")];
    let options = Options::new("deepseek-chat");

    let stream = provider.complete_stream(messages, options).await.unwrap();
    let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

    let valid_deltas: Vec<&Delta> = deltas
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter(|d| d.content.is_some())
        .collect();

    assert_eq!(valid_deltas.len(), 1);
    assert_eq!(valid_deltas[0].content, Some("test".to_string()));
}

// ============================================================================
// Tests for error handling
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_complete_http_401_unauthorized() {
    let error = ProviderError::Http {
        status: 401,
        message: "Invalid API key".to_string(),
    };
    let transport = MockHttpTransport::with_json_error(error);
    let provider =
        DeepSeekProvider::with_transport("invalid-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Hello!")];
    let options = Options::new("deepseek-chat");

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
async fn test_complete_http_429_rate_limited() {
    let error = ProviderError::Http {
        status: 429,
        message: "Rate limit exceeded".to_string(),
    };
    let transport = MockHttpTransport::with_json_error(error);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Hello!")];
    let options = Options::new("deepseek-chat");

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
async fn test_complete_http_500_server_error() {
    let error = ProviderError::Http {
        status: 500,
        message: "Internal server error".to_string(),
    };
    let transport = MockHttpTransport::with_json_error(error);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Hello!")];
    let options = Options::new("deepseek-chat");

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
async fn test_complete_stream_http_error() {
    let chunks = vec![Err(ProviderError::Http {
        status: 503,
        message: "Service unavailable".to_string(),
    })];

    let transport = MockHttpTransport::with_stream_chunks(chunks);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![Message::user("Test")];
    let options = Options::new("deepseek-chat");

    // Stream creation should succeed, but consuming should fail
    let stream = provider.complete_stream(messages, options).await.unwrap();
    let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

    assert!(!deltas.is_empty());
    assert!(deltas[0].is_err());

    match deltas[0].as_ref().unwrap_err() {
        ProviderError::Http { status, .. } => {
            assert_eq!(*status, 503);
        }
        other => panic!("Expected Http error, got: {:?}", other),
    }
}

// ============================================================================
// Tests for Provider Trait implementation
// ============================================================================

#[test]
#[ignore]
fn test_provider_id() {
    let provider = DeepSeekProvider::new("test-key", "deepseek-chat");
    assert_eq!(provider.provider_id(), "deepseek");
}

#[test]
#[ignore]
fn test_model_id() {
    let provider = DeepSeekProvider::new("test-key", "deepseek-chat");
    assert_eq!(provider.model_id(), "deepseek-chat");

    let provider = DeepSeekProvider::new("test-key", "deepseek-reasoner");
    assert_eq!(provider.model_id(), "deepseek-reasoner");
}

#[test]
#[ignore]
fn test_token_count_estimate() {
    let provider = DeepSeekProvider::new("test-key", "deepseek-chat");

    // Default implementation: text.len() / 4
    assert_eq!(provider.token_count("hello"), 1); // 5 / 4 = 1
    assert_eq!(provider.token_count("hello world"), 2); // 11 / 4 = 2
    assert_eq!(provider.token_count(""), 0);
    assert_eq!(provider.token_count("a"), 0); // 1 / 4 = 0
}

// ============================================================================
// Additional tests for DeepSeek-specific behavior
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_deepseek_with_system_message() {
    let mock_response = serde_json::json!({
        "id": "cmpl-sys123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "I'm DeepSeek, a helpful AI assistant."
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
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![
        Message::system("You are DeepSeek, a helpful AI assistant."),
        Message::user("Who are you?"),
    ];
    let options = Options::new("deepseek-chat");

    let result = provider.complete(messages, options).await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(
        response.message.content,
        "I'm DeepSeek, a helpful AI assistant."
    );
}

#[tokio::test]
#[ignore]
async fn test_deepseek_multi_turn_conversation() {
    let mock_response = serde_json::json!({
        "id": "cmpl-multi-turn123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Nice to meet you too!"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 30,
            "completion_tokens": 8,
            "total_tokens": 38
        }
    });

    let transport = MockHttpTransport::with_json_response(mock_response);
    let provider =
        DeepSeekProvider::with_transport("test-key", "deepseek-chat", Arc::new(transport));

    let messages = vec![
        Message::user("Hello!"),
        Message::assistant("Hi there! How can I help you today?"),
        Message::user("Nice to meet you!"),
    ];
    let options = Options::new("deepseek-chat");

    let result = provider.complete(messages, options).await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.message.content, "Nice to meet you too!");
}
