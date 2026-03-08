use serde::{Deserialize, Serialize};

use crate::{
    traits::MessageFormat,
    types::{CompletionResponse, Delta, FinishReason, Message, Options, Role, TokenUsage},
};

/// Anthropic Messages API request.
#[derive(Serialize, Debug)]
pub struct AnthropicRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub stream: bool,
}

/// Anthropic message format.
#[derive(Serialize, Deserialize, Debug)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: String,
}

/// Anthropic Messages API response.
#[derive(Deserialize, Debug)]
pub struct AnthropicResponse {
    pub id: String,
    pub model: String,
    pub content: Vec<AnthropicContentBlock>,
    #[serde(rename = "stop_reason")]
    pub stop_reason: String,
    pub usage: AnthropicUsage,
}

#[derive(Deserialize, Debug)]
pub struct AnthropicContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
}

#[derive(Deserialize, Debug)]
pub struct AnthropicUsage {
    #[serde(rename = "input_tokens")]
    pub input_tokens: u64,
    #[serde(rename = "output_tokens")]
    pub output_tokens: u64,
}

/// Anthropic streaming chunk.
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum AnthropicStreamChunk {
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { delta: AnthropicTextDelta },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: AnthropicMessageDelta,
        usage: Option<AnthropicStreamUsage>,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug)]
pub struct AnthropicTextDelta {
    #[serde(rename = "type")]
    pub delta_type: String,
    pub text: String,
}

#[derive(Deserialize, Debug)]
pub struct AnthropicMessageDelta {
    #[serde(rename = "stop_reason")]
    pub stop_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct AnthropicStreamUsage {
    #[serde(rename = "output_tokens")]
    pub output_tokens: u64,
}

/// Anthropic format parsing error.
#[derive(Debug)]
pub enum AnthropicError {
    #[allow(dead_code)]
    MissingField(&'static str),
    InvalidFormat(String),
}

impl std::fmt::Display for AnthropicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnthropicError::MissingField(field) => write!(f, "Missing field: {}", field),
            AnthropicError::InvalidFormat(msg) => write!(f, "Invalid format: {}", msg),
        }
    }
}

impl std::error::Error for AnthropicError {}

/// Serialization / deserialization for the Anthropic Messages API.
pub struct AnthropicFormat;

impl MessageFormat for AnthropicFormat {
    type Request = AnthropicRequest;
    type Response = AnthropicResponse;
    type StreamChunk = AnthropicStreamChunk;
    type Error = AnthropicError;

    fn build_request(messages: &[Message], opts: &Options) -> Self::Request {
        // System prompt: top-level field, not inside messages
        let system_prompt: Option<String> = opts.system.clone().or_else(|| {
            messages
                .iter()
                .find(|m| m.role == Role::System)
                .map(|m| m.content.clone())
        });

        // Filter out system-role messages from the messages array
        let msg_array: Vec<AnthropicMessage> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "user", // tool results sent as user in Anthropic
                    Role::System => unreachable!(),
                };
                AnthropicMessage {
                    role: role.to_string(),
                    content: m.content.clone(),
                }
            })
            .collect();

        AnthropicRequest {
            model: opts.model.clone(),
            max_tokens: opts.max_tokens,
            messages: msg_array,
            system: system_prompt,
            stream: opts.stream,
        }
    }

    fn parse_response(raw: Self::Response) -> Result<CompletionResponse, Self::Error> {
        let content = raw
            .content
            .into_iter()
            .find(|block| block.block_type == "text")
            .map(|block| block.text)
            .unwrap_or_default();

        let finish_reason = match raw.stop_reason.as_str() {
            "end_turn" => FinishReason::Stop,
            "max_tokens" => FinishReason::Length,
            "tool_use" => FinishReason::ToolCalls,
            _ => FinishReason::Other("unknown".to_string()),
        };

        Ok(CompletionResponse {
            id: raw.id,
            model: raw.model,
            message: Message::assistant(content),
            finish_reason,
            usage: TokenUsage {
                prompt_tokens: raw.usage.input_tokens,
                completion_tokens: raw.usage.output_tokens,
                total_tokens: raw.usage.input_tokens + raw.usage.output_tokens,
            },
        })
    }

    fn parse_stream_chunk(chunk: &[u8]) -> Result<Option<Delta>, Self::Error> {
        let line = String::from_utf8_lossy(chunk).trim().to_string();

        // Skip event: lines
        if line.starts_with("event:") {
            return Ok(None);
        }

        // Strip "data: " prefix if present
        let line = if let Some(stripped) = line.strip_prefix("data: ") {
            stripped.trim()
        } else {
            &line
        };

        if line.is_empty() {
            return Ok(None);
        }

        let chunk: AnthropicStreamChunk =
            serde_json::from_str(line).map_err(|e| AnthropicError::InvalidFormat(e.to_string()))?;

        match chunk {
            AnthropicStreamChunk::ContentBlockDelta { delta }
                if delta.delta_type == "text_delta" =>
            {
                Ok(Some(Delta {
                    content: Some(delta.text),
                    tool_call: None,
                    finish_reason: None,
                    usage: None,
                }))
            }
            AnthropicStreamChunk::MessageDelta { delta, usage } => {
                let finish_reason = delta.stop_reason.as_deref().map(|r| match r {
                    "end_turn" => FinishReason::Stop,
                    "max_tokens" => FinishReason::Length,
                    "tool_use" => FinishReason::ToolCalls,
                    _ => FinishReason::Other("unknown".to_string()),
                });
                let usage = usage.map(|u| TokenUsage {
                    prompt_tokens: 0,
                    completion_tokens: u.output_tokens,
                    total_tokens: u.output_tokens,
                });
                Ok(Some(Delta {
                    content: None,
                    tool_call: None,
                    finish_reason,
                    usage,
                }))
            }
            AnthropicStreamChunk::MessageStop => Ok(None),
            AnthropicStreamChunk::Other => Ok(None),
            AnthropicStreamChunk::ContentBlockDelta { .. } => Ok(None),
        }
    }

    fn token_count(messages: &[Message]) -> usize {
        // Anthropic uses ~4 chars per token on average
        messages.iter().map(|m| m.content.len() / 4).sum()
    }

    fn endpoint() -> &'static str {
        "/v1/messages"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_format_request() {
        let messages = vec![Message::user("hello")];
        let opts = Options::new("claude-opus-4-6");
        let req = AnthropicFormat::build_request(&messages, &opts);
        assert_eq!(req.model, "claude-opus-4-6");
        assert_eq!(req.max_tokens, 4096);
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, "user");
    }

    #[test]
    fn test_anthropic_system_at_top_level() {
        let messages = vec![Message::user("hello")];
        let opts = Options::new("claude-opus-4-6").with_system("Be helpful");
        let req = AnthropicFormat::build_request(&messages, &opts);
        assert_eq!(req.system, Some("Be helpful".to_string()));
    }

    #[test]
    fn test_anthropic_system_not_in_messages() {
        let messages = vec![Message::system("Be helpful"), Message::user("hello")];
        let opts = Options::new("claude-opus-4-6");
        let req = AnthropicFormat::build_request(&messages, &opts);
        // system extracted to top-level
        assert_eq!(req.system, Some("Be helpful".to_string()));
        // messages array should only have the user message
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, "user");
    }

    #[test]
    fn test_anthropic_parse_response() {
        let raw = AnthropicResponse {
            id: "msg_abc".to_string(),
            model: "claude-opus-4-6".to_string(),
            content: vec![AnthropicContentBlock {
                block_type: "text".to_string(),
                text: "Hello!".to_string(),
            }],
            stop_reason: "end_turn".to_string(),
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        };
        let resp = AnthropicFormat::parse_response(raw).unwrap();
        assert_eq!(resp.id, "msg_abc");
        assert_eq!(resp.model, "claude-opus-4-6");
        assert_eq!(resp.message.content, "Hello!");
        assert_eq!(resp.finish_reason, FinishReason::Stop);
    }

    #[test]
    fn test_anthropic_parse_response_usage() {
        let raw = AnthropicResponse {
            id: "msg_xyz".to_string(),
            model: "claude-opus-4-6".to_string(),
            content: vec![AnthropicContentBlock {
                block_type: "text".to_string(),
                text: "Hi".to_string(),
            }],
            stop_reason: "end_turn".to_string(),
            usage: AnthropicUsage {
                input_tokens: 20,
                output_tokens: 10,
            },
        };
        let resp = AnthropicFormat::parse_response(raw).unwrap();
        assert_eq!(resp.usage.prompt_tokens, 20);
        assert_eq!(resp.usage.completion_tokens, 10);
        assert_eq!(resp.usage.total_tokens, 30);
    }

    #[test]
    fn test_anthropic_parse_stream_chunk_text_delta() {
        let line = r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}"#;
        let delta = AnthropicFormat::parse_stream_chunk(line.as_bytes()).unwrap();
        assert!(delta.is_some());
        let d = delta.unwrap();
        assert_eq!(d.content, Some("hello".to_string()));
    }

    #[test]
    fn test_anthropic_parse_stream_chunk_done() {
        let line = r#"{"type":"message_stop"}"#;
        let result = AnthropicFormat::parse_stream_chunk(line.as_bytes()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_anthropic_stop_reason_mapping() {
        // max_tokens
        let raw = AnthropicResponse {
            id: "msg_1".to_string(),
            model: "claude-opus-4-6".to_string(),
            content: vec![AnthropicContentBlock {
                block_type: "text".to_string(),
                text: "...".to_string(),
            }],
            stop_reason: "max_tokens".to_string(),
            usage: AnthropicUsage {
                input_tokens: 0,
                output_tokens: 0,
            },
        };
        let resp = AnthropicFormat::parse_response(raw).unwrap();
        assert_eq!(resp.finish_reason, FinishReason::Length);

        // tool_use
        let raw2 = AnthropicResponse {
            id: "msg_2".to_string(),
            model: "claude-opus-4-6".to_string(),
            content: vec![AnthropicContentBlock {
                block_type: "text".to_string(),
                text: "".to_string(),
            }],
            stop_reason: "tool_use".to_string(),
            usage: AnthropicUsage {
                input_tokens: 0,
                output_tokens: 0,
            },
        };
        let resp2 = AnthropicFormat::parse_response(raw2).unwrap();
        assert_eq!(resp2.finish_reason, FinishReason::ToolCalls);
    }

    #[test]
    fn test_token_count() {
        let messages = vec![Message::user("hello world")]; // 11 chars
        assert_eq!(AnthropicFormat::token_count(&messages), 2); // 11 / 4 = 2
    }

    #[test]
    fn test_endpoint() {
        assert_eq!(AnthropicFormat::endpoint(), "/v1/messages");
    }
}
