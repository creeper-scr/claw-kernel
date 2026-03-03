//! Streaming response handling for LLM providers.
//!
//! Provides unified parsing for SSE (Server-Sent Events) and NDJSON streams.

use std::pin::Pin;

use futures::Stream;

use crate::error::ProviderError;
use crate::types::{Delta, FinishReason, TokenUsage};

/// Type alias for boxed streams of results.
pub type BoxStream<T> = Pin<Box<dyn Stream<Item = Result<T, ProviderError>> + Send>>;

/// A chunk from a streaming response.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    /// Incremental content delta.
    Delta {
        /// Text content increment (if any).
        content: Option<String>,
        /// Partial or complete tool call.
        tool_call: Option<crate::types::ToolCall>,
    },
    /// Stream completed successfully.
    Done {
        /// Final finish reason.
        finish_reason: FinishReason,
        /// Final token usage (if available).
        usage: Option<TokenUsage>,
    },
    /// Error occurred during streaming.
    Error {
        /// Error message.
        message: String,
        /// Whether this is a retryable error.
        retryable: bool,
    },
}

impl StreamChunk {
    /// Create a content delta chunk.
    pub fn content(content: impl Into<String>) -> Self {
        Self::Delta {
            content: Some(content.into()),
            tool_call: None,
        }
    }

    /// Create a tool call delta chunk.
    pub fn tool_call(tool_call: crate::types::ToolCall) -> Self {
        Self::Delta {
            content: None,
            tool_call: Some(tool_call),
        }
    }

    /// Create a completion chunk.
    pub fn done(finish_reason: FinishReason) -> Self {
        Self::Done {
            finish_reason,
            usage: None,
        }
    }

    /// Create a completion chunk with usage.
    pub fn done_with_usage(finish_reason: FinishReason, usage: TokenUsage) -> Self {
        Self::Done {
            finish_reason,
            usage: Some(usage),
        }
    }

    /// Create an error chunk.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            retryable: false,
        }
    }

    /// Create a retryable error chunk.
    pub fn retryable_error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            retryable: true,
        }
    }

    /// Check if this is a delta chunk.
    pub fn is_delta(&self) -> bool {
        matches!(self, Self::Delta { .. })
    }

    /// Check if this is a done chunk.
    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done { .. })
    }

    /// Check if this is an error chunk.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }
}

/// Parse an SSE (Server-Sent Events) event line.
///
/// SSE format:
/// ```text
/// event: message
/// data: {"key": "value"}
///
/// event: message
/// data: {"key": "value"}
/// ```
///
/// # Arguments
///
/// * `line` - A line from the SSE stream
///
/// # Returns
///
/// * `Ok(Some((event_type, data)))` - Parsed event with type and data
/// * `Ok(None)` - Empty line or comment (should be skipped)
/// * `Err(...)` - Parse error
pub fn parse_sse_event(line: &str) -> Result<Option<(String, String)>, ProviderError> {
    let line = line.trim();

    // Empty line indicates end of event
    if line.is_empty() {
        return Ok(None);
    }

    // Skip comments (lines starting with :)
    if line.starts_with(':') {
        return Ok(None);
    }

    // Parse "field: value" format
    if let Some((field, value)) = line.split_once(':') {
        let field = field.trim();
        let value = value.trim_start();

        match field {
            "event" => Ok(Some((value.to_string(), String::new()))),
            "data" => Ok(Some(("message".to_string(), value.to_string()))),
            // Ignore other fields (id, retry, etc.)
            _ => Ok(None),
        }
    } else {
        // Line without colon is ignored per SSE spec
        Ok(None)
    }
}

/// Parse an NDJSON (Newline Delimited JSON) line.
///
/// NDJSON format: One JSON object per line
/// ```text
/// {"key": "value1"}
/// {"key": "value2"}
/// ```
///
/// # Arguments
///
/// * `line` - A line from the NDJSON stream
///
/// # Returns
///
/// * `Ok(Some(json))` - Parsed JSON value
/// * `Ok(None)` - Empty line (should be skipped)
/// * `Err(...)` - Parse error
pub fn parse_ndjson_line(line: &str) -> Result<Option<serde_json::Value>, ProviderError> {
    let line = line.trim();

    if line.is_empty() {
        return Ok(None);
    }

    // Parse as JSON
    let value = serde_json::from_str(line)
        .map_err(|e| ProviderError::Serialization(format!("Invalid NDJSON: {}", e)))?;

    Ok(Some(value))
}

/// Parse SSE data lines, handling the "data: " prefix.
///
/// This function processes lines that have already been identified as data lines
/// from an SSE stream.
pub fn parse_sse_data_line(line: &str) -> Result<Option<serde_json::Value>, ProviderError> {
    let line = line.trim();

    // Strip "data: " prefix if present
    let data = if let Some(stripped) = line.strip_prefix("data: ") {
        stripped.trim()
    } else {
        line
    };

    // Handle [DONE] marker (OpenAI-style stream end)
    if data == "[DONE]" {
        return Ok(None);
    }

    if data.is_empty() {
        return Ok(None);
    }

    // Parse as JSON
    let value = serde_json::from_str(data)
        .map_err(|e| ProviderError::Serialization(format!("Invalid SSE data: {}", e)))?;

    Ok(Some(value))
}

/// Convert a stream of bytes into a stream of SSE events.
///
/// This function processes a raw byte stream and yields complete SSE events.
pub fn into_sse_stream<S>(byte_stream: S) -> impl Stream<Item = Result<StreamChunk, ProviderError>>
where
    S: Stream<Item = Result<bytes::Bytes, ProviderError>> + Send + 'static,
{
    use async_stream::try_stream;
    use futures::StreamExt;

    let stream = try_stream! {
        let mut buffer = String::new();

        futures::pin_mut!(byte_stream);

        while let Some(chunk_result) = byte_stream.next().await {
            let bytes = chunk_result?;
            let text = String::from_utf8_lossy(&bytes);
            buffer.push_str(&text);

            // Process complete lines
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].to_string();
                buffer = buffer[pos + 1..].to_string();

                if let Some((event_type, data)) = parse_sse_event(&line)? {
                    if event_type == "error" {
                        yield StreamChunk::error(data);
                    } else if !data.is_empty() {
                        // Try to parse as JSON
                        match parse_sse_data_line(&data) {
                            Ok(Some(_json)) => {
                                // This is a placeholder - actual parsing would be provider-specific
                                yield StreamChunk::content(data);
                            }
                            Ok(None) => {}
                            Err(e) => yield StreamChunk::error(e.to_string()),
                        }
                    }
                }
            }
        }

        // Process any remaining content
        if !buffer.is_empty() {
            if let Some((event_type, data)) = parse_sse_event(&buffer)? {
                if event_type == "error" {
                    yield StreamChunk::error(data);
                } else if !data.is_empty() {
                    yield StreamChunk::content(data);
                }
            }
        }
    };

    stream
}

/// Convert a stream of bytes into a stream of NDJSON objects.
///
/// This function processes a raw byte stream and yields complete JSON objects.
pub fn into_ndjson_stream<S>(
    byte_stream: S,
) -> impl Stream<Item = Result<StreamChunk, ProviderError>>
where
    S: Stream<Item = Result<bytes::Bytes, ProviderError>> + Send + 'static,
{
    use async_stream::try_stream;
    use futures::StreamExt;

    let stream = try_stream! {
        let mut buffer = String::new();

        futures::pin_mut!(byte_stream);

        while let Some(chunk_result) = byte_stream.next().await {
            let bytes = chunk_result?;
            let text = String::from_utf8_lossy(&bytes);
            buffer.push_str(&text);

            // Process complete lines
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].to_string();
                buffer = buffer[pos + 1..].to_string();

                if let Some(_json) = parse_ndjson_line(&line)? {
                    // This is a placeholder - actual parsing would be provider-specific
                    yield StreamChunk::content(line);
                }
            }
        }

        // Process any remaining content
        if !buffer.is_empty() {
            if let Some(_json) = parse_ndjson_line(&buffer)? {
                yield StreamChunk::content(buffer);
            }
        }
    };

    stream
}

/// Create a stream chunk from a raw Delta.
///
/// Helper function to convert internal Delta type to StreamChunk.
pub fn delta_to_chunk(delta: Delta) -> Option<StreamChunk> {
    if delta.content.is_none() && delta.tool_call.is_none() && delta.finish_reason.is_none() {
        return None;
    }

    if let Some(finish_reason) = delta.finish_reason {
        Some(StreamChunk::Done {
            finish_reason,
            usage: delta.usage,
        })
    } else {
        Some(StreamChunk::Delta {
            content: delta.content,
            tool_call: delta.tool_call,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_chunk_content() {
        let chunk = StreamChunk::content("hello");
        assert!(chunk.is_delta());
        assert!(!chunk.is_done());
        assert!(!chunk.is_error());

        if let StreamChunk::Delta { content, tool_call } = chunk {
            assert_eq!(content, Some("hello".to_string()));
            assert!(tool_call.is_none());
        } else {
            panic!("Expected Delta variant");
        }
    }

    #[test]
    fn test_stream_chunk_done() {
        let chunk = StreamChunk::done(FinishReason::Stop);
        assert!(!chunk.is_delta());
        assert!(chunk.is_done());
        assert!(!chunk.is_error());

        if let StreamChunk::Done {
            finish_reason,
            usage,
        } = chunk
        {
            assert_eq!(finish_reason, FinishReason::Stop);
            assert!(usage.is_none());
        } else {
            panic!("Expected Done variant");
        }
    }

    #[test]
    fn test_stream_chunk_error() {
        let chunk = StreamChunk::error("something went wrong");
        assert!(!chunk.is_delta());
        assert!(!chunk.is_done());
        assert!(chunk.is_error());

        if let StreamChunk::Error { message, retryable } = chunk {
            assert_eq!(message, "something went wrong");
            assert!(!retryable);
        } else {
            panic!("Expected Error variant");
        }
    }

    #[test]
    fn test_stream_chunk_retryable_error() {
        let chunk = StreamChunk::retryable_error("timeout");

        if let StreamChunk::Error { message, retryable } = chunk {
            assert_eq!(message, "timeout");
            assert!(retryable);
        } else {
            panic!("Expected Error variant");
        }
    }

    #[test]
    fn test_parse_sse_event_data() {
        let line = "data: {\"key\": \"value\"}";
        let result = parse_sse_event(line).unwrap();
        assert_eq!(
            result,
            Some(("message".to_string(), "{\"key\": \"value\"}".to_string()))
        );
    }

    #[test]
    fn test_parse_sse_event_event() {
        let line = "event: content_block_delta";
        let result = parse_sse_event(line).unwrap();
        assert_eq!(
            result,
            Some(("content_block_delta".to_string(), String::new()))
        );
    }

    #[test]
    fn test_parse_sse_event_empty() {
        let line = "";
        let result = parse_sse_event(line).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_sse_event_comment() {
        let line = ": this is a comment";
        let result = parse_sse_event(line).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_sse_event_with_space() {
        let line = "data:   value with leading spaces";
        let result = parse_sse_event(line).unwrap();
        assert_eq!(
            result,
            Some((
                "message".to_string(),
                "value with leading spaces".to_string()
            ))
        );
    }

    #[test]
    fn test_parse_ndjson_line_valid() {
        let line = r#"{"choices": [{"delta": {"content": "hello"}}]}"#;
        let result = parse_ndjson_line(line).unwrap();
        assert!(result.is_some());
        let json = result.unwrap();
        assert!(json.get("choices").is_some());
    }

    #[test]
    fn test_parse_ndjson_line_empty() {
        let line = "";
        let result = parse_ndjson_line(line).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_ndjson_line_invalid() {
        let line = "not valid json";
        let result = parse_ndjson_line(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_sse_data_line_with_prefix() {
        let line = "data: {\"key\": \"value\"}";
        let result = parse_sse_data_line(line).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_sse_data_line_done() {
        let line = "data: [DONE]";
        let result = parse_sse_data_line(line).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_delta_to_chunk_content() {
        let delta = Delta {
            content: Some("hello".to_string()),
            tool_call: None,
            finish_reason: None,
            usage: None,
        };
        let chunk = delta_to_chunk(delta).unwrap();
        assert!(chunk.is_delta());
    }

    #[test]
    fn test_delta_to_chunk_done() {
        let delta = Delta {
            content: None,
            tool_call: None,
            finish_reason: Some(FinishReason::Stop),
            usage: Some(TokenUsage::new(10, 5)),
        };
        let chunk = delta_to_chunk(delta).unwrap();
        assert!(chunk.is_done());
    }

    #[test]
    fn test_delta_to_chunk_empty() {
        let delta = Delta {
            content: None,
            tool_call: None,
            finish_reason: None,
            usage: None,
        };
        let chunk = delta_to_chunk(delta);
        assert!(chunk.is_none());
    }
}
