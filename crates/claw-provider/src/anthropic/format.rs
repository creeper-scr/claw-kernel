use serde_json::{json, Value};

use crate::{
    error::ProviderError,
    traits::MessageFormat,
    types::{CompletionResponse, Delta, FinishReason, Message, Options, Role, TokenUsage},
};

/// Serialization / deserialization for the Anthropic Messages API.
pub struct AnthropicFormat;

impl AnthropicFormat {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AnthropicFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageFormat for AnthropicFormat {
    fn format_request(
        &self,
        messages: &[Message],
        options: &Options,
    ) -> Result<Value, ProviderError> {
        // System prompt: top-level field, not inside messages
        let system_prompt: Option<String> = options
            .system
            .clone()
            .or_else(|| {
                messages
                    .iter()
                    .find(|m| m.role == Role::System)
                    .map(|m| m.content.clone())
            });

        // Filter out system-role messages from the messages array
        let msg_array: Vec<Value> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| {
                let role_str = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "user", // tool results sent as user in Anthropic
                    Role::System => unreachable!(),
                };
                json!({ "role": role_str, "content": m.content })
            })
            .collect();

        let mut body = json!({
            "model": options.model,
            "max_tokens": options.max_tokens,
            "messages": msg_array,
        });

        if let Some(sys) = system_prompt {
            body["system"] = json!(sys);
        }

        if options.stream {
            body["stream"] = json!(true);
        }

        Ok(body)
    }

    fn parse_response(&self, raw: Value) -> Result<CompletionResponse, ProviderError> {
        let id = raw["id"]
            .as_str()
            .ok_or_else(|| ProviderError::Serialization("missing 'id' field".into()))?
            .to_string();

        let model = raw["model"]
            .as_str()
            .ok_or_else(|| ProviderError::Serialization("missing 'model' field".into()))?
            .to_string();

        // content is an array of content blocks
        let content = raw["content"]
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|block| block["type"].as_str() == Some("text"))
                    .and_then(|block| block["text"].as_str())
            })
            .unwrap_or("")
            .to_string();

        let finish_reason = match raw["stop_reason"].as_str().unwrap_or("end_turn") {
            "end_turn" => FinishReason::Stop,
            "max_tokens" => FinishReason::MaxTokens,
            "tool_use" => FinishReason::ToolUse,
            _ => FinishReason::Other,
        };

        let usage = {
            let u = &raw["usage"];
            let prompt_tokens = u["input_tokens"].as_u64().unwrap_or(0);
            let completion_tokens = u["output_tokens"].as_u64().unwrap_or(0);
            TokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            }
        };

        Ok(CompletionResponse {
            id,
            model,
            message: Message::assistant(content),
            finish_reason,
            usage,
        })
    }

    fn parse_stream_chunk(&self, raw: &str) -> Result<Option<Delta>, ProviderError> {
        // Anthropic SSE format:
        // event: content_block_delta
        // data: {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "..."}}
        //
        // Also: event: message_stop → end of stream

        // Skip event: lines
        if raw.starts_with("event:") {
            return Ok(None);
        }

        let line = if let Some(stripped) = raw.strip_prefix("data: ") {
            stripped.trim()
        } else {
            raw.trim()
        };

        if line.is_empty() {
            return Ok(None);
        }

        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };

        match v["type"].as_str() {
            Some("content_block_delta") => {
                let delta_type = v["delta"]["type"].as_str().unwrap_or("");
                if delta_type == "text_delta" {
                    let text = v["delta"]["text"].as_str().unwrap_or("").to_string();
                    Ok(Some(Delta {
                        content: Some(text),
                        tool_call: None,
                        finish_reason: None,
                        usage: None,
                    }))
                } else {
                    Ok(None)
                }
            }
            Some("message_delta") => {
                // May contain stop_reason and usage
                let finish_reason = match v["delta"]["stop_reason"].as_str() {
                    Some("end_turn") => Some(FinishReason::Stop),
                    Some("max_tokens") => Some(FinishReason::MaxTokens),
                    Some("tool_use") => Some(FinishReason::ToolUse),
                    _ => None,
                };
                let usage = {
                    let u = &v["usage"];
                    if u.is_object() {
                        let out = u["output_tokens"].as_u64().unwrap_or(0);
                        Some(TokenUsage {
                            prompt_tokens: 0,
                            completion_tokens: out,
                            total_tokens: out,
                        })
                    } else {
                        None
                    }
                };
                Ok(Some(Delta {
                    content: None,
                    tool_call: None,
                    finish_reason,
                    usage,
                }))
            }
            Some("message_stop") => Ok(None),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt() -> AnthropicFormat {
        AnthropicFormat::new()
    }

    #[test]
    fn test_anthropic_format_request() {
        let f = fmt();
        let messages = vec![Message::user("hello")];
        let opts = Options::new("claude-opus-4-6");
        let body = f.format_request(&messages, &opts).unwrap();
        assert_eq!(body["model"], "claude-opus-4-6");
        assert_eq!(body["max_tokens"], 4096);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn test_anthropic_system_at_top_level() {
        let f = fmt();
        let messages = vec![Message::user("hello")];
        let opts = Options::new("claude-opus-4-6").with_system("Be helpful");
        let body = f.format_request(&messages, &opts).unwrap();
        assert_eq!(body["system"], "Be helpful");
    }

    #[test]
    fn test_anthropic_system_not_in_messages() {
        let f = fmt();
        let messages = vec![
            Message::system("Be helpful"),
            Message::user("hello"),
        ];
        let opts = Options::new("claude-opus-4-6");
        let body = f.format_request(&messages, &opts).unwrap();
        // system extracted to top-level
        assert_eq!(body["system"], "Be helpful");
        // messages array should only have the user message
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn test_anthropic_parse_response() {
        let f = fmt();
        let raw = serde_json::json!({
            "id": "msg_abc",
            "model": "claude-opus-4-6",
            "content": [{"type": "text", "text": "Hello!"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let resp = f.parse_response(raw).unwrap();
        assert_eq!(resp.id, "msg_abc");
        assert_eq!(resp.model, "claude-opus-4-6");
        assert_eq!(resp.message.content, "Hello!");
        assert_eq!(resp.finish_reason, FinishReason::Stop);
    }

    #[test]
    fn test_anthropic_parse_response_usage() {
        let f = fmt();
        let raw = serde_json::json!({
            "id": "msg_xyz",
            "model": "claude-opus-4-6",
            "content": [{"type": "text", "text": "Hi"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 20, "output_tokens": 10}
        });
        let resp = f.parse_response(raw).unwrap();
        assert_eq!(resp.usage.prompt_tokens, 20);
        assert_eq!(resp.usage.completion_tokens, 10);
        assert_eq!(resp.usage.total_tokens, 30);
    }

    #[test]
    fn test_anthropic_parse_stream_chunk_text_delta() {
        let f = fmt();
        let line = r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}"#;
        let delta = f.parse_stream_chunk(line).unwrap();
        assert!(delta.is_some());
        let d = delta.unwrap();
        assert_eq!(d.content, Some("hello".to_string()));
    }

    #[test]
    fn test_anthropic_parse_stream_chunk_done() {
        let f = fmt();
        let line = r#"data: {"type":"message_stop"}"#;
        let result = f.parse_stream_chunk(line).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_anthropic_stop_reason_mapping() {
        let f = fmt();
        // max_tokens
        let raw = serde_json::json!({
            "id": "msg_1",
            "model": "claude-opus-4-6",
            "content": [{"type": "text", "text": "..."}],
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 0, "output_tokens": 0}
        });
        let resp = f.parse_response(raw).unwrap();
        assert_eq!(resp.finish_reason, FinishReason::MaxTokens);

        // tool_use
        let raw2 = serde_json::json!({
            "id": "msg_2",
            "model": "claude-opus-4-6",
            "content": [{"type": "text", "text": ""}],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 0, "output_tokens": 0}
        });
        let resp2 = f.parse_response(raw2).unwrap();
        assert_eq!(resp2.finish_reason, FinishReason::ToolUse);
    }
}
