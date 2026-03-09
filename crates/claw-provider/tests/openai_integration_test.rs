//! Integration tests for OpenAIProvider using mock HTTP transport.
//!
//! These tests verify OpenAIProvider functionality without requiring real API keys.

#![cfg(feature = "test-utils")]

use async_trait::async_trait;
use bytes::Bytes;
use claw_provider::{
    Delta, FinishReason, HttpTransport, LLMProvider, Message, OpenAIProvider, Options,
    ProviderError, ToolDef,
};
use futures::{stream, Stream, StreamExt};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// Mock HTTP transport for testing OpenAIProvider.
struct MockHttpTransport {
    /// Predefined JSON response for post_json calls
    json_response: Option<serde_json::Value>,
    /// Predefined stream chunks for post_stream calls
    stream_chunks: Vec<Result<Bytes, ProviderError>>,
    /// Error to return (optional)
    error: Option<ProviderError>,
    /// Captured request details for verification
    captured_request: Arc<Mutex<Option<CapturedRequest>>>,
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    #[allow(dead_code)]
    url: String,
    #[allow(dead_code)]
    headers: Vec<(String, String)>,
    body: serde_json::Value,
}

impl MockHttpTransport {
    fn new() -> Self {
        Self {
            json_response: None,
            stream_chunks: Vec::new(),
            error: None,
            captured_request: Arc::new(Mutex::new(None)),
        }
    }

    fn with_json_response(mut self, response: serde_json::Value) -> Self {
        self.json_response = Some(response);
        self
    }

    fn with_stream_chunks(mut self, chunks: Vec<Result<Bytes, ProviderError>>) -> Self {
        self.stream_chunks = chunks;
        self
    }

    fn with_error(mut self, error: ProviderError) -> Self {
        self.error = Some(error);
        self
    }

    #[allow(dead_code)]
    fn get_captured_request(&self) -> Option<CapturedRequest> {
        self.captured_request.lock().unwrap().clone()
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
        // Capture the request details
        let captured = CapturedRequest {
            url: url.to_string(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.clone(),
        };
        *self.captured_request.lock().unwrap() = Some(captured);

        // Return error if configured
        if let Some(ref err) = self.error {
            return Err(err.clone());
        }

        // Return predefined response or default success response
        Ok(self.json_response.clone().unwrap_or_else(|| {
            serde_json::json!({
                "id": "chatcmpl-default",
                "object": "chat.completion",
                "created": 1677652288,
                "model": "gpt-4o",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Default response"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            })
        }))
    }

    async fn post_stream(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send>>, ProviderError>
    {
        // Capture the request details
        let captured = CapturedRequest {
            url: url.to_string(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.clone(),
        };
        *self.captured_request.lock().unwrap() = Some(captured);

        // Return error if configured
        if let Some(ref err) = self.error {
            return Err(err.clone());
        }

        let chunks: Vec<_> = self.stream_chunks.clone();
        Ok(Box::pin(stream::iter(chunks)))
    }
}

/// Helper to create OpenAIProvider with mock transport
fn create_provider_with_mock(mock: MockHttpTransport) -> OpenAIProvider {
    OpenAIProvider::new("test-api-key", "gpt-4o").__with_transport(Arc::new(mock))
}

/// Create a standard OpenAI API success response
fn create_success_response(content: &str, model: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 9, "completion_tokens": 5, "total_tokens": 14}
    })
}

/// Create an OpenAI API streaming chunk
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
async fn test_complete_success() {
    let mock = MockHttpTransport::new()
        .with_json_response(create_success_response("Hello! How can I help?", "gpt-4o"));
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Hi there")];
    let options = Options::new("gpt-4o").with_max_tokens(100);

    let response = provider.complete(messages, options).await.unwrap();

    assert_eq!(response.id, "chatcmpl-123");
    assert_eq!(response.model, "gpt-4o");
    assert_eq!(response.message.content, "Hello! How can I help?");
}

#[tokio::test]
#[ignore]
async fn test_complete_with_system_message() {
    let mock = MockHttpTransport::new().with_json_response(create_success_response(
        "I'm a helpful assistant.",
        "gpt-4o",
    ));
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Who are you?")];
    let options = Options::new("gpt-4o")
        .with_max_tokens(100)
        .with_system("You are a helpful assistant.");

    let response = provider.complete(messages, options).await.unwrap();
    assert_eq!(response.message.content, "I'm a helpful assistant.");
}

#[tokio::test]
#[ignore]
async fn test_complete_captures_correct_url() {
    let mock =
        MockHttpTransport::new().with_json_response(create_success_response("Test", "gpt-4o"));
    let captured = mock.captured_request.clone();
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Test")];
    let options = Options::new("gpt-4o");

    provider.complete(messages, options).await.unwrap();

    let request = captured.lock().unwrap().as_ref().unwrap().clone();
    assert!(request.url.contains("/chat/completions"));
    assert!(request.url.starts_with("https://api.openai.com/v1"));
}

#[tokio::test]
#[ignore]
async fn test_complete_sends_authorization_header() {
    let mock =
        MockHttpTransport::new().with_json_response(create_success_response("Test", "gpt-4o"));
    let captured = mock.captured_request.clone();
    let provider =
        OpenAIProvider::new("sk-test-api-key", "gpt-4o").__with_transport(Arc::new(mock));

    let messages = vec![Message::user("Test")];
    let options = Options::new("gpt-4o");

    provider.complete(messages, options).await.unwrap();

    let request = captured.lock().unwrap().as_ref().unwrap().clone();
    let auth_header = request
        .headers
        .iter()
        .find(|(k, _)| k.to_lowercase() == "authorization");
    assert!(auth_header.is_some());
    assert!(auth_header.unwrap().1.contains("sk-test-api-key"));
}

#[tokio::test]
#[ignore]
async fn test_complete_request_body_structure() {
    let mock =
        MockHttpTransport::new().with_json_response(create_success_response("Test", "gpt-4o"));
    let captured = mock.captured_request.clone();
    let provider = create_provider_with_mock(mock);

    let messages = vec![
        Message::system("System prompt"),
        Message::user("User message"),
    ];
    let options = Options::new("gpt-4o")
        .with_max_tokens(100)
        .with_temperature(0.5)
        .unwrap();

    provider.complete(messages, options).await.unwrap();

    let request = captured.lock().unwrap().as_ref().unwrap().clone();
    let body = request.body;

    assert_eq!(body["model"], "gpt-4o");
    assert_eq!(body["max_tokens"], 100);
    // Use approx comparison for float due to serialization precision
    let temp = body["temperature"].as_f64().unwrap();
    assert!(
        (temp - 0.5).abs() < 0.01,
        "temperature should be approximately 0.5"
    );
    assert_eq!(body["stream"], false);
    assert!(body["messages"].is_array());
}

// ============================================================================
// Tests for complete_stream() method
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_complete_stream_success() {
    let chunks = vec![
        Ok(Bytes::from(format!(
            "data: {}\n\n",
            create_stream_chunk("Hello", None)
        ))),
        Ok(Bytes::from(format!(
            "data: {}\n\n",
            create_stream_chunk(" world", None)
        ))),
        Ok(Bytes::from("data: [DONE]\n\n")),
    ];

    let mock = MockHttpTransport::new().with_stream_chunks(chunks);
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Say hello")];
    let options = Options::new("gpt-4o").with_max_tokens(50);

    let stream = provider.complete_stream(messages, options).await.unwrap();
    let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

    assert_eq!(deltas.len(), 2);
    assert_eq!(
        deltas[0].as_ref().unwrap().content,
        Some("Hello".to_string())
    );
    assert_eq!(
        deltas[1].as_ref().unwrap().content,
        Some(" world".to_string())
    );
}

#[tokio::test]
#[ignore]
async fn test_complete_stream_with_finish_reason() {
    let chunks = vec![
        Ok(Bytes::from(format!(
            "data: {}\n\n",
            create_stream_chunk("Done", Some("stop"))
        ))),
        Ok(Bytes::from("data: [DONE]\n\n")),
    ];

    let mock = MockHttpTransport::new().with_stream_chunks(chunks);
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Complete")];
    let options = Options::new("gpt-4o");

    let stream = provider.complete_stream(messages, options).await.unwrap();
    let deltas: Vec<Result<Delta, ProviderError>> = stream.collect().await;

    assert_eq!(deltas.len(), 1);
    assert_eq!(
        deltas[0].as_ref().unwrap().content,
        Some("Done".to_string())
    );
    assert_eq!(
        deltas[0].as_ref().unwrap().finish_reason,
        Some(FinishReason::Stop)
    );
}

#[tokio::test]
#[ignore]
async fn test_complete_stream_sets_stream_flag() {
    let mock =
        MockHttpTransport::new().with_stream_chunks(vec![Ok(Bytes::from("data: [DONE]\n\n"))]);
    let captured = mock.captured_request.clone();
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Test")];
    let options = Options::new("gpt-4o");

    // Just verify the request was made with stream flag set
    let _ = provider.complete_stream(messages, options).await.unwrap();

    let request = captured.lock().unwrap().as_ref().unwrap().clone();
    assert_eq!(request.body["stream"], true);
}

// ============================================================================
// Tests for tool calls (function calling)
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_complete_with_tool_calls() {
    let tool_response = serde_json::json!({
        "id": "chatcmpl-tool-123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "gpt-4o",
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
                        "arguments": r#"{"location": "San Francisco", "unit": "celsius"}"#
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 20, "completion_tokens": 15, "total_tokens": 35}
    });

    let mock = MockHttpTransport::new().with_json_response(tool_response);
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("What's the weather in San Francisco?")];
    let tools = vec![ToolDef {
        name: "get_weather".to_string(),
        description: "Get weather for a location".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "location": {"type": "string"},
                "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]}
            },
            "required": ["location"]
        }),
    }];
    let options = Options::new("gpt-4o").with_tools(tools);

    let response = provider.complete(messages, options).await.unwrap();

    // The response should indicate tool calls finish reason
    assert_eq!(response.finish_reason, FinishReason::ToolCalls);
}

// ============================================================================
// Tests for error handling
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_complete_401_unauthorized() {
    let mock = MockHttpTransport::new().with_error(ProviderError::Http {
        status: 401,
        message: "Invalid API key".to_string(),
    });
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Test")];
    let options = Options::new("gpt-4o");

    let result = provider.complete(messages, options).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ProviderError::Http { status, message } => {
            assert_eq!(status, 401);
            assert!(message.contains("Invalid API key"));
        }
        _ => panic!("Expected Http error with 401 status"),
    }
}

#[tokio::test]
#[ignore]
async fn test_complete_429_rate_limited() {
    let mock = MockHttpTransport::new().with_error(ProviderError::Http {
        status: 429,
        message: "Rate limit exceeded".to_string(),
    });
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Test")];
    let options = Options::new("gpt-4o");

    let result = provider.complete(messages, options).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ProviderError::Http { status, .. } => {
            assert_eq!(status, 429);
        }
        _ => panic!("Expected Http error with 429 status"),
    }
}

#[tokio::test]
#[ignore]
async fn test_complete_500_server_error() {
    let mock = MockHttpTransport::new().with_error(ProviderError::Http {
        status: 500,
        message: "Internal server error".to_string(),
    });
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Test")];
    let options = Options::new("gpt-4o");

    let result = provider.complete(messages, options).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ProviderError::Http { status, .. } => {
            assert_eq!(status, 500);
        }
        _ => panic!("Expected Http error with 500 status"),
    }
}

#[tokio::test]
#[ignore]
async fn test_complete_network_error() {
    let mock = MockHttpTransport::new()
        .with_error(ProviderError::Network("Connection refused".to_string()));
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Test")];
    let options = Options::new("gpt-4o");

    let result = provider.complete(messages, options).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ProviderError::Network(msg) => {
            assert!(msg.contains("Connection refused"));
        }
        _ => panic!("Expected Network error"),
    }
}

// ============================================================================
// Tests for custom base_url (OpenAI-compatible services)
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_custom_base_url_local_llm() {
    let mock = MockHttpTransport::new()
        .with_json_response(create_success_response("Hello", "local-model"));
    let captured = mock.captured_request.clone();

    let provider = OpenAIProvider::new("test-key", "local-model")
        .with_base_url("http://localhost:8080/v1")
        .__with_transport(Arc::new(mock));

    let messages = vec![Message::user("Test")];
    let options = Options::new("local-model");

    provider.complete(messages, options).await.unwrap();

    let request = captured.lock().unwrap().as_ref().unwrap().clone();
    assert!(request.url.starts_with("http://localhost:8080/v1"));
    assert!(request.url.contains("/chat/completions"));
}

#[tokio::test]
#[ignore]
async fn test_custom_base_url_openai_compatible_service() {
    let mock = MockHttpTransport::new()
        .with_json_response(create_success_response("Hello", "custom-model"));
    let captured = mock.captured_request.clone();

    let provider = OpenAIProvider::new("test-key", "custom-model")
        .with_base_url("https://api.custom-ai.com/v1")
        .__with_transport(Arc::new(mock));

    let messages = vec![Message::user("Test")];
    let options = Options::new("custom-model");

    provider.complete(messages, options).await.unwrap();

    let request = captured.lock().unwrap().as_ref().unwrap().clone();
    assert!(request.url.starts_with("https://api.custom-ai.com/v1"));
}

// ============================================================================
// Tests for different finish reasons
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_complete_finish_reason_length() {
    let response = serde_json::json!({
        "id": "chatcmpl-123",
        "model": "gpt-4o",
        "choices": [{
            "message": {"role": "assistant", "content": "Truncated..."},
            "finish_reason": "length"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    });

    let mock = MockHttpTransport::new().with_json_response(response);
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Tell me a long story")];
    let options = Options::new("gpt-4o").with_max_tokens(5);

    let result = provider.complete(messages, options).await.unwrap();
    assert_eq!(result.finish_reason, FinishReason::Length);
}

#[tokio::test]
#[ignore]
async fn test_complete_finish_reason_content_filter() {
    let response = serde_json::json!({
        "id": "chatcmpl-123",
        "model": "gpt-4o",
        "choices": [{
            "message": {"role": "assistant", "content": ""},
            "finish_reason": "content_filter"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 0, "total_tokens": 10}
    });

    let mock = MockHttpTransport::new().with_json_response(response);
    let provider = create_provider_with_mock(mock);

    let messages = vec![Message::user("Generate inappropriate content")];
    let options = Options::new("gpt-4o");

    let result = provider.complete(messages, options).await.unwrap();
    assert_eq!(result.finish_reason, FinishReason::ContentFilter);
}

// ============================================================================
// Tests for complex conversation flows
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_complete_multi_turn_conversation() {
    let mock = MockHttpTransport::new().with_json_response(create_success_response(
        "I'm doing well, thanks for asking!",
        "gpt-4o",
    ));
    let captured = mock.captured_request.clone();
    let provider = create_provider_with_mock(mock);

    let messages = vec![
        Message::user("Hello!"),
        Message::assistant("Hi there! How can I help you today?"),
        Message::user("How are you?"),
    ];
    let options = Options::new("gpt-4o");

    provider.complete(messages, options).await.unwrap();

    let request = captured.lock().unwrap().as_ref().unwrap().clone();
    let messages_array = request.body["messages"].as_array().unwrap();

    assert_eq!(messages_array.len(), 3);
    assert_eq!(messages_array[0]["role"], "user");
    assert_eq!(messages_array[1]["role"], "assistant");
    assert_eq!(messages_array[2]["role"], "user");
}

// ============================================================================
// Tests for provider trait implementation
// ============================================================================

#[test]
#[ignore]
fn test_provider_id() {
    let provider = OpenAIProvider::new("test-key", "gpt-4o");
    assert_eq!(provider.provider_id(), "openai");
}

#[test]
#[ignore]
fn test_model_id() {
    let provider = OpenAIProvider::new("test-key", "gpt-4o-mini");
    assert_eq!(provider.model_id(), "gpt-4o-mini");
}

#[tokio::test]
#[ignore]
async fn test_provider_methods_with_mock() {
    let mock =
        MockHttpTransport::new().with_json_response(create_success_response("Test", "gpt-4"));
    let provider = OpenAIProvider::new("test-key", "gpt-4o").__with_transport(Arc::new(mock));

    assert_eq!(provider.provider_id(), "openai");
    assert_eq!(provider.model_id(), "gpt-4o"); // default model from new()
}
