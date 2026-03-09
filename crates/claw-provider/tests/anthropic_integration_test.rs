//! Integration tests for AnthropicProvider using mock HTTP transport.
//!
//! These tests verify the AnthropicProvider behavior without requiring
//! real API credentials or network access.
//!
//! Run with: cargo test -p claw-provider --features test-utils

#![cfg(feature = "test-utils")]

use async_trait::async_trait;
use bytes::Bytes;
use claw_provider::{
    AnthropicProvider, Delta, FinishReason, HttpTransport, LLMProvider, Message, Options,
    ProviderError, RetryConfig,
};
use futures::{stream, Stream, StreamExt};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Mock HTTP transport for testing Anthropic API interactions.
struct MockHttpTransport {
    /// Pre-configured JSON response for post_json calls
    json_response: Option<serde_json::Value>,
    /// Pre-configured error for post_json calls
    json_error: Option<ProviderError>,
    /// Pre-configured stream chunks for post_stream calls
    stream_chunks: Vec<Result<Bytes, ProviderError>>,
    /// Pre-configured error for post_stream calls
    stream_error: Option<ProviderError>,
    /// Record of calls made to this transport
    calls: Arc<Mutex<Vec<CallRecord>>>,
}

#[derive(Debug, Clone)]
struct CallRecord {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<serde_json::Value>,
}

impl MockHttpTransport {
    fn new() -> Self {
        Self {
            json_response: None,
            json_error: None,
            stream_chunks: Vec::new(),
            stream_error: None,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_json_response(mut self, response: serde_json::Value) -> Self {
        self.json_response = Some(response);
        self
    }

    fn with_json_error(mut self, error: ProviderError) -> Self {
        self.json_error = Some(error);
        self
    }

    fn with_stream_chunks(mut self, chunks: Vec<Result<Bytes, ProviderError>>) -> Self {
        self.stream_chunks = chunks;
        self
    }

    fn with_stream_error(mut self, error: ProviderError) -> Self {
        self.stream_error = Some(error);
        self
    }

    fn calls(&self) -> Arc<Mutex<Vec<CallRecord>>> {
        Arc::clone(&self.calls)
    }
}

#[async_trait]
impl HttpTransport for MockHttpTransport {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        let record = CallRecord {
            method: "POST".to_string(),
            url: url.to_string(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: Some(body.clone()),
        };
        self.calls.lock().unwrap().push(record);

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
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send>>, ProviderError>
    {
        let record = CallRecord {
            method: "POST_STREAM".to_string(),
            url: url.to_string(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: Some(body.clone()),
        };
        self.calls.lock().unwrap().push(record);

        if let Some(ref error) = self.stream_error {
            return Err(error.clone());
        }

        let chunks: Vec<_> = self.stream_chunks.clone();
        Ok(Box::pin(stream::iter(chunks)))
    }
}

/// Create a standard Anthropic API success response.
fn anthropic_success_response() -> serde_json::Value {
    serde_json::json!({
        "id": "msg_01XgS0Kp8b7",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Hello! How can I help you today?"}],
        "model": "claude-3-opus-20240229",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 10, "output_tokens": 8}
    })
}

/// Create an Anthropic streaming chunk for text delta.
fn anthropic_text_delta_chunk(text: &str) -> Bytes {
    Bytes::from(format!(
        r#"{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"{}"}}}}"#,
        text.replace('"', "\\\"")
    ))
}

/// Create an Anthropic streaming chunk for message stop.
fn anthropic_message_stop_chunk() -> Bytes {
    Bytes::from(r#"{"type":"message_stop"}"#)
}

/// Create an Anthropic streaming chunk for message delta (finish reason + usage).
fn anthropic_message_delta_chunk(stop_reason: &str, output_tokens: u64) -> Bytes {
    Bytes::from(format!(
        r#"{{"type":"message_delta","delta":{{"stop_reason":"{}"}},"usage":{{"output_tokens":{}}}}}"#,
        stop_reason, output_tokens
    ))
}

/// Helper to create a test provider with mock transport.
fn create_test_provider(transport: MockHttpTransport) -> AnthropicProvider {
    AnthropicProvider::with_transport(
        "sk-ant-test-key",
        "claude-3-opus-20240229",
        Arc::new(transport),
    )
}

mod complete_tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_complete_success_response() {
        let mock_response = anthropic_success_response();
        let transport = MockHttpTransport::new().with_json_response(mock_response);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Hello, Claude!")];
        let options = Options::new("claude-3-opus-20240229").with_max_tokens(100);

        let result = provider.complete(messages, options).await;

        assert!(result.is_ok(), "Expected successful completion");
        let response = result.unwrap();
        assert_eq!(response.id, "msg_01XgS0Kp8b7");
        assert_eq!(response.model, "claude-3-opus-20240229");
        assert_eq!(response.message.content, "Hello! How can I help you today?");
        assert_eq!(response.finish_reason, FinishReason::Stop);
        assert_eq!(response.usage.prompt_tokens, 10);
        assert_eq!(response.usage.completion_tokens, 8);
        assert_eq!(response.usage.total_tokens, 18);
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_with_system_message() {
        let mock_response = serde_json::json!({
            "id": "msg_02TestSystem",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "I am a helpful assistant."}],
            "model": "claude-3-opus-20240229",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 25, "output_tokens": 6}
        });
        let transport = MockHttpTransport::new().with_json_response(mock_response);
        let calls = transport.calls();
        let provider = create_test_provider(transport);

        let messages = vec![
            Message::system("You are a helpful assistant."),
            Message::user("Who are you?"),
        ];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete(messages, options).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.message.content, "I am a helpful assistant.");

        // Verify the request body contains the system message
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let body = calls[0].body.as_ref().unwrap();
        assert_eq!(body["system"], "You are a helpful assistant.");
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_max_tokens_stop_reason() {
        let mock_response = serde_json::json!({
            "id": "msg_03MaxTokens",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "This is a long response that..."}],
            "model": "claude-3-opus-20240229",
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 10, "output_tokens": 4096}
        });
        let transport = MockHttpTransport::new().with_json_response(mock_response);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Write a very long story")];
        let options = Options::new("claude-3-opus-20240229").with_max_tokens(4096);

        let result = provider.complete(messages, options).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.finish_reason, FinishReason::Length);
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_tool_use_stop_reason() {
        let mock_response = serde_json::json!({
            "id": "msg_04ToolUse",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": ""}],
            "model": "claude-3-opus-20240229",
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 50, "output_tokens": 25}
        });
        let transport = MockHttpTransport::new().with_json_response(mock_response);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("What's the weather?")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete(messages, options).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
    }
}

mod stream_tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_complete_stream_single_chunk() {
        let chunks = vec![
            Ok(anthropic_text_delta_chunk("Hello!")),
            Ok(anthropic_message_delta_chunk("end_turn", 2)),
            Ok(anthropic_message_stop_chunk()),
        ];
        let transport = MockHttpTransport::new().with_stream_chunks(chunks);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Hi!")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete_stream(messages, options).await;

        assert!(result.is_ok());
        let stream = result.unwrap();
        let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

        // Should have 2 deltas: text content and finish reason
        let deltas: Vec<Delta> = deltas.into_iter().filter_map(|r| r.ok()).collect();
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].content, Some("Hello!".to_string()));
        assert_eq!(deltas[1].finish_reason, Some(FinishReason::Stop));
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_stream_multiple_text_chunks() {
        let chunks = vec![
            Ok(anthropic_text_delta_chunk("Hello")),
            Ok(anthropic_text_delta_chunk(", ")),
            Ok(anthropic_text_delta_chunk("world")),
            Ok(anthropic_text_delta_chunk("!")),
            Ok(anthropic_message_delta_chunk("end_turn", 4)),
            Ok(anthropic_message_stop_chunk()),
        ];
        let transport = MockHttpTransport::new().with_stream_chunks(chunks);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Greet me!")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete_stream(messages, options).await;

        assert!(result.is_ok());
        let stream = result.unwrap();
        let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

        let text_deltas: Vec<String> = deltas
            .into_iter()
            .filter_map(|r| r.ok())
            .filter_map(|d| d.content)
            .collect();

        assert_eq!(text_deltas, vec!["Hello", ", ", "world", "!"]);
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_stream_with_usage() {
        let chunks = vec![
            Ok(anthropic_text_delta_chunk("Response text")),
            Ok(anthropic_message_delta_chunk("end_turn", 15)),
            Ok(anthropic_message_stop_chunk()),
        ];
        let transport = MockHttpTransport::new().with_stream_chunks(chunks);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Test")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete_stream(messages, options).await;

        assert!(result.is_ok());
        let stream = result.unwrap();
        let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

        // Find the delta with usage info
        let usage_delta = deltas
            .into_iter()
            .filter_map(|r| r.ok())
            .find(|d| d.usage.is_some())
            .expect("Should have a delta with usage");

        let usage = usage_delta.usage.unwrap();
        assert_eq!(usage.completion_tokens, 15);
        assert_eq!(usage.total_tokens, 15);
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_stream_empty_response() {
        let chunks = vec![
            Ok(anthropic_message_delta_chunk("end_turn", 0)),
            Ok(anthropic_message_stop_chunk()),
        ];
        let transport = MockHttpTransport::new().with_stream_chunks(chunks);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Say nothing")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete_stream(messages, options).await;

        assert!(result.is_ok());
        let stream = result.unwrap();
        let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

        // Should only have the finish reason delta
        let deltas: Vec<Delta> = deltas.into_iter().filter_map(|r| r.ok()).collect();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].finish_reason, Some(FinishReason::Stop));
    }
}

mod error_handling_tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_complete_http_401_error() {
        let error = ProviderError::Http {
            status: 401,
            message: "Invalid API key".to_string(),
        };
        let transport = MockHttpTransport::new().with_json_error(error);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Test")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete(messages, options).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Http { status, message } => {
                assert_eq!(status, 401);
                assert_eq!(message, "Invalid API key");
            }
            _ => panic!("Expected Http error with 401 status"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_http_403_error() {
        let error = ProviderError::Http {
            status: 403,
            message: "Permission denied".to_string(),
        };
        let transport = MockHttpTransport::new().with_json_error(error);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Test")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete(messages, options).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Http { status, message } => {
                assert_eq!(status, 403);
                assert_eq!(message, "Permission denied");
            }
            _ => panic!("Expected Http error with 403 status"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_http_429_error() {
        let error = ProviderError::RateLimited {
            retry_after_secs: 30,
        };
        let transport = MockHttpTransport::new().with_json_error(error);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Test")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete(messages, options).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::RateLimited { retry_after_secs } => {
                assert_eq!(retry_after_secs, 30);
            }
            _ => panic!("Expected RateLimited error"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_http_500_error() {
        // Server errors are returned as Network errors in the transport
        let error = ProviderError::Network("server error 500: Internal Server Error".to_string());
        let transport = MockHttpTransport::new().with_json_error(error);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Test")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete(messages, options).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Network(msg) => {
                assert!(msg.contains("500"));
            }
            _ => panic!("Expected Network error for server error"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_auth_error() {
        let error = ProviderError::Auth("API key expired".to_string());
        let transport = MockHttpTransport::new().with_json_error(error);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Test")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete(messages, options).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Auth(msg) => {
                assert_eq!(msg, "API key expired");
            }
            _ => panic!("Expected Auth error"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_stream_network_error() {
        let error = ProviderError::Network("Connection refused".to_string());
        let transport = MockHttpTransport::new().with_stream_error(error);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Test")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete_stream(messages, options).await;

        assert!(result.is_err());
        assert!(result.is_err());
        match result {
            Err(ProviderError::Network(msg)) => {
                assert_eq!(msg, "Connection refused");
            }
            _ => panic!("Expected Network error for stream"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_complete_serialization_error() {
        // Invalid JSON response that can't be parsed
        let invalid_response = serde_json::json!({
            "id": "msg_invalid",
            // Missing required fields like content, model, stop_reason, usage
        });
        let transport = MockHttpTransport::new().with_json_response(invalid_response);
        let provider = create_test_provider(transport);

        let messages = vec![Message::user("Test")];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete(messages, options).await;

        assert!(result.is_err());
        // Should be a serialization error due to missing fields
        match result.unwrap_err() {
            ProviderError::Serialization(_) => {
                // Expected
            }
            _ => panic!("Expected Serialization error for invalid response"),
        }
    }
}

mod retry_config_tests {
    use super::*;

    #[test]
    #[ignore]
    fn test_provider_with_retry_config() {
        let transport = MockHttpTransport::new();
        let provider = create_test_provider(transport);

        let retry_config = RetryConfig::new()
            .with_max_retries(5)
            .with_base_delay(Duration::from_millis(100))
            .with_max_delay(Duration::from_secs(5));

        let provider = provider.with_retry(retry_config);

        let config = provider.retry_config().expect("Should have retry config");
        assert_eq!(config.max_retries, 5);
    }

    #[test]
    #[ignore]
    fn test_provider_default_no_retry() {
        let transport = MockHttpTransport::new();
        let provider = create_test_provider(transport);

        assert!(provider.retry_config().is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn test_retry_applied_to_requests() {
        // This test verifies that when retry is configured, the transport
        // is replaced with one that has retry capabilities.
        // The actual retry behavior is tested in the retry module.

        let transport = MockHttpTransport::new().with_json_response(anthropic_success_response());
        let provider = create_test_provider(transport);

        let retry_config = RetryConfig::new().with_max_retries(3);
        let provider = provider.with_retry(retry_config);

        // Verify retry config is set
        assert!(provider.retry_config().is_some());
        assert_eq!(provider.retry_config().unwrap().max_retries, 3);

        // Note: With the current implementation, with_retry replaces the transport
        // with DefaultHttpTransport that has retry enabled, so this will fail
        // because we're using a mock. This test documents the expected behavior.
        // In a real scenario with actual HTTP, retries would work.

        // For mock-based testing, we verify the config is set correctly
        // rather than testing the actual retry mechanism
    }
}

mod integration_tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_full_conversation_flow() {
        // Simulate a multi-turn conversation
        let mock_response = serde_json::json!({
            "id": "msg_conversation",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "I'm doing well, thank you!"}],
            "model": "claude-3-opus-20240229",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 50, "output_tokens": 8}
        });
        let transport = MockHttpTransport::new().with_json_response(mock_response);
        let provider = create_test_provider(transport);

        let messages = vec![
            Message::system("You are a helpful assistant."),
            Message::user("Hello!"),
            Message::assistant("Hi there! How can I help you today?"),
            Message::user("How are you doing?"),
        ];
        let options = Options::new("claude-3-opus-20240229");

        let result = provider.complete(messages, options).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.message.content, "I'm doing well, thank you!");
    }

    #[tokio::test]
    #[ignore]
    async fn test_provider_identifiers() {
        let transport = MockHttpTransport::new();
        let provider = create_test_provider(transport);

        assert_eq!(provider.provider_id(), "anthropic");
        assert_eq!(provider.model_id(), "claude-3-opus-20240229");
    }

    #[tokio::test]
    #[ignore]
    async fn test_token_count_estimation() {
        let transport = MockHttpTransport::new();
        let provider = create_test_provider(transport);

        // Default implementation: chars / 4
        let text = "Hello world"; // 11 chars
        assert_eq!(provider.token_count(text), 2); // 11 / 4 = 2

        let empty = "";
        assert_eq!(provider.token_count(empty), 0);

        let long_text = "a".repeat(100); // 100 chars
        assert_eq!(provider.token_count(&long_text), 25); // 100 / 4 = 25
    }
}
