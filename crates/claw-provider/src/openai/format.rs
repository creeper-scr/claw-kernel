use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    traits::MessageFormat,
    types::{CompletionResponse, Delta, FinishReason, Message, Options, Role, TokenUsage, ToolCall},
};

/// OpenAI Chat Completions API request.
#[derive(Serialize, Debug)]
pub struct OpenAIRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
}

/// OpenAI message format.
#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// OpenAI Chat Completions API response.
#[derive(Deserialize, Debug)]
pub struct OpenAIResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<OpenAIChoice>,
    pub usage: OpenAIUsage,
}

#[derive(Deserialize, Debug)]
pub struct OpenAIChoice {
    pub message: OpenAIResponseMessage,
    pub finish_reason: String,
}

#[derive(Deserialize, Debug)]
pub struct OpenAIResponseMessage {
    #[allow(dead_code)]
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Option<Vec<OpenAIToolCall>>,
}

/// OpenAI tool call format.
#[derive(Deserialize, Debug)]
pub struct OpenAIToolCall {
    pub id: String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub call_type: String,
    pub function: OpenAIFunctionCall,
}

/// OpenAI function call format.
#[derive(Deserialize, Debug)]
pub struct OpenAIFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Deserialize, Debug)]
pub struct OpenAIUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// OpenAI streaming chunk.
#[derive(Deserialize, Debug)]
pub struct OpenAIStreamChunk {
    pub choices: Vec<OpenAIStreamChoice>,
}

#[derive(Deserialize, Debug)]
pub struct OpenAIStreamChoice {
    pub delta: OpenAIDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
pub struct OpenAIDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    pub role: Option<String>,
}

/// OpenAI format parsing error.
#[derive(Debug)]
pub enum OpenAIError {
    MissingField(&'static str),
    InvalidFormat(String),
}

impl std::fmt::Display for OpenAIError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenAIError::MissingField(field) => write!(f, "Missing field: {}", field),
            OpenAIError::InvalidFormat(msg) => write!(f, "Invalid format: {}", msg),
        }
    }
}

impl std::error::Error for OpenAIError {}

/// Serialization / deserialization for the OpenAI Chat Completions API.
pub struct OpenAIFormat;

impl MessageFormat for OpenAIFormat {
    type Request = OpenAIRequest;
    type Response = OpenAIResponse;
    type StreamChunk = OpenAIStreamChunk;
    type Error = OpenAIError;

    fn build_request(messages: &[Message], opts: &Options) -> Self::Request {
        let msg_array: Vec<OpenAIMessage> = messages
            .iter()
            .filter_map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => "system",
                    Role::Tool => "tool",
                };
                // Skip system messages if we have system prompt in options
                if m.role == Role::System && opts.system.is_some() {
                    return None;
                }
                Some(OpenAIMessage {
                    role: role.to_string(),
                    content: m.content.clone(),
                    tool_call_id: m.tool_call_id.clone(),
                })
            })
            .collect();

        let mut messages = msg_array;

        // Prepend system message if provided
        if let Some(sys) = &opts.system {
            messages.insert(
                0,
                OpenAIMessage {
                    role: "system".to_string(),
                    content: sys.clone(),
                    tool_call_id: None,
                },
            );
        }

        OpenAIRequest {
            model: opts.model.clone(),
            messages,
            max_tokens: Some(opts.max_tokens),
            temperature: Some(opts.temperature),
            stream: opts.stream,
            stop: None,
            tools: None,
        }
    }

    fn parse_response(raw: Self::Response) -> Result<CompletionResponse, Self::Error> {
        let choice = raw
            .choices
            .into_iter()
            .next()
            .ok_or(OpenAIError::MissingField("choices"))?;

        let finish_reason = match choice.finish_reason.as_str() {
            "stop" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "tool_calls" => FinishReason::ToolCalls,
            "content_filter" => FinishReason::ContentFilter,
            _ => FinishReason::Other("unknown".to_string()),
        };

        // Convert OpenAI tool calls to canonical format
        let tool_calls = choice.message.tool_calls.map(|calls| {
            calls
                .into_iter()
                .map(|call| ToolCall {
                    id: call.id,
                    name: call.function.name,
                    arguments: call.function.arguments,
                })
                .collect()
        });

        let mut message = Message::assistant(choice.message.content);
        message.tool_calls = tool_calls;

        Ok(CompletionResponse {
            id: raw.id,
            model: raw.model,
            message,
            finish_reason,
            usage: TokenUsage {
                prompt_tokens: raw.usage.prompt_tokens,
                completion_tokens: raw.usage.completion_tokens,
                total_tokens: raw.usage.total_tokens,
            },
        })
    }

    fn parse_stream_chunk(chunk: &[u8]) -> Result<Option<Delta>, Self::Error> {
        let line = String::from_utf8_lossy(chunk);
        let line = line.trim();

        // Strip "data: " prefix if present
        let line = if let Some(stripped) = line.strip_prefix("data: ") {
            stripped.trim()
        } else {
            line
        };

        if line.is_empty() || line == "[DONE]" {
            return Ok(None);
        }

        let chunk: OpenAIStreamChunk =
            serde_json::from_str(line).map_err(|e| OpenAIError::InvalidFormat(e.to_string()))?;

        let choice = match chunk.choices.into_iter().next() {
            Some(c) => c,
            None => return Ok(None),
        };

        let finish_reason = choice.finish_reason.as_deref().map(|r| match r {
            "stop" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "tool_calls" => FinishReason::ToolCalls,
            "content_filter" => FinishReason::ContentFilter,
            _ => FinishReason::Other("unknown".to_string()),
        });

        Ok(Some(Delta {
            content: choice.delta.content,
            tool_call: None,
            finish_reason,
            usage: None,
        }))
    }

    fn token_count(messages: &[Message]) -> usize {
        // Simple heuristic: 4 chars ≈ 1 token
        messages.iter().map(|m| m.content.len() / 4).sum()
    }

    fn endpoint() -> &'static str {
        "/v1/chat/completions"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_request_basic() {
        let messages = vec![Message::user("hello")];
        let opts = Options::new("gpt-4o");
        let req = OpenAIFormat::build_request(&messages, &opts);

        assert_eq!(req.model, "gpt-4o");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, "user");
        assert_eq!(req.messages[0].content, "hello");
        assert!(!req.stream);
    }

    #[test]
    fn test_format_request_with_system() {
        let messages = vec![Message::user("hello")];
        let opts = Options::new("gpt-4o").with_system("You are helpful");
        let req = OpenAIFormat::build_request(&messages, &opts);

        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0].role, "system");
        assert_eq!(req.messages[0].content, "You are helpful");
        assert_eq!(req.messages[1].role, "user");
    }

    #[test]
    fn test_format_request_stream_enabled() {
        let messages = vec![Message::user("test")];
        let opts = Options::new("gpt-4o").with_stream();
        let req = OpenAIFormat::build_request(&messages, &opts);
        assert!(req.stream);
    }

    #[test]
    fn test_parse_response_stop() {
        let raw = OpenAIResponse {
            id: "chatcmpl-abc123".to_string(),
            model: "gpt-4o".to_string(),
            choices: vec![OpenAIChoice {
                message: OpenAIResponseMessage {
                    role: "assistant".to_string(),
                    content: "Hello!".to_string(),
                    tool_calls: None,
                },
                finish_reason: "stop".to_string(),
            }],
            usage: OpenAIUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        };
        let resp = OpenAIFormat::parse_response(raw).unwrap();
        assert_eq!(resp.id, "chatcmpl-abc123");
        assert_eq!(resp.model, "gpt-4o");
        assert_eq!(resp.message.content, "Hello!");
        assert_eq!(resp.finish_reason, FinishReason::Stop);
        assert_eq!(resp.usage.prompt_tokens, 10);
        assert_eq!(resp.usage.completion_tokens, 5);
        assert_eq!(resp.usage.total_tokens, 15);
    }

    #[test]
    fn test_parse_response_max_tokens() {
        let raw = OpenAIResponse {
            id: "chatcmpl-xyz".to_string(),
            model: "gpt-4o".to_string(),
            choices: vec![OpenAIChoice {
                message: OpenAIResponseMessage {
                    role: "assistant".to_string(),
                    content: "...".to_string(),
                    tool_calls: None,
                },
                finish_reason: "length".to_string(),
            }],
            usage: OpenAIUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
            },
        };
        let resp = OpenAIFormat::parse_response(raw).unwrap();
        assert_eq!(resp.finish_reason, FinishReason::Length);
    }

    #[test]
    fn test_parse_stream_chunk_text() {
        let line = r#"{"choices":[{"delta":{"content":"hello"},"finish_reason":null}]}"#;
        let delta = OpenAIFormat::parse_stream_chunk(line.as_bytes()).unwrap();
        assert!(delta.is_some());
        let d = delta.unwrap();
        assert_eq!(d.content, Some("hello".to_string()));
        assert!(d.finish_reason.is_none());
    }

    #[test]
    fn test_parse_stream_chunk_done() {
        let result = OpenAIFormat::parse_stream_chunk("data: [DONE]".as_bytes()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_stream_chunk_empty_content() {
        let line = r#"{"choices":[{"delta":{"content":""},"finish_reason":null}]}"#;
        let delta = OpenAIFormat::parse_stream_chunk(line.as_bytes()).unwrap();
        assert!(delta.is_some());
        let d = delta.unwrap();
        assert_eq!(d.content, Some("".to_string()));
    }

    #[test]
    fn test_token_count() {
        let messages = vec![
            Message::user("hello world"), // 11 chars
            Message::assistant("test"),   // 4 chars
        ];
        // (11 + 4) / 4 = 3 tokens
        assert_eq!(OpenAIFormat::token_count(&messages), 3);
    }

    #[test]
    fn test_endpoint() {
        assert_eq!(OpenAIFormat::endpoint(), "/v1/chat/completions");
    }

    #[test]
    fn test_format_request_multiple_messages() {
        let messages = vec![
            Message::user("first"),
            Message::assistant("second"),
            Message::user("third"),
        ];
        let opts = Options::new("gpt-4o");
        let req = OpenAIFormat::build_request(&messages, &opts);
        assert_eq!(req.messages.len(), 3);
        assert_eq!(req.messages[0].role, "user");
        assert_eq!(req.messages[1].role, "assistant");
        assert_eq!(req.messages[2].role, "user");
    }
}
