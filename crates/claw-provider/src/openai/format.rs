use serde_json::{json, Value};

use crate::{
    error::ProviderError,
    traits::MessageFormat,
    types::{CompletionResponse, Delta, FinishReason, Message, Options, Role, TokenUsage},
};

/// Serialization / deserialization for the OpenAI Chat Completions API.
pub struct OpenAIFormat;

impl OpenAIFormat {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OpenAIFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageFormat for OpenAIFormat {
    fn format_request(
        &self,
        messages: &[Message],
        options: &Options,
    ) -> Result<Value, ProviderError> {
        let mut msg_array: Vec<Value> = Vec::new();

        // System prompt: either from options.system or a system-role message
        if let Some(sys) = &options.system {
            msg_array.push(json!({ "role": "system", "content": sys }));
        }

        for m in messages {
            let role_str = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => {
                    // If we already inserted the options.system above, skip message-level system.
                    // Otherwise, emit it.
                    if options.system.is_none() {
                        msg_array.push(json!({ "role": "system", "content": m.content }));
                    }
                    continue;
                }
                Role::Tool => "tool",
            };

            let mut obj = json!({ "role": role_str, "content": m.content });
            if let Some(tc_id) = &m.tool_call_id {
                obj["tool_call_id"] = json!(tc_id);
            }
            msg_array.push(obj);
        }

        let body = json!({
            "model": options.model,
            "messages": msg_array,
            "max_tokens": options.max_tokens,
            "temperature": options.temperature,
            "stream": options.stream,
        });
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

        let choice = raw["choices"]
            .as_array()
            .and_then(|arr| arr.first())
            .ok_or_else(|| ProviderError::Serialization("missing 'choices' array".into()))?;

        let content = choice["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let finish_reason = match choice["finish_reason"].as_str().unwrap_or("stop") {
            "stop" => FinishReason::Stop,
            "length" => FinishReason::MaxTokens,
            "tool_calls" => FinishReason::ToolUse,
            "content_filter" => FinishReason::ContentFilter,
            _ => FinishReason::Other,
        };

        let usage = {
            let u = &raw["usage"];
            TokenUsage {
                prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0),
                completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0),
                total_tokens: u["total_tokens"].as_u64().unwrap_or(0),
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
        // Strip "data: " prefix if present
        let line = if let Some(stripped) = raw.strip_prefix("data: ") {
            stripped.trim()
        } else {
            raw.trim()
        };

        if line.is_empty() {
            return Ok(None);
        }

        if line == "[DONE]" {
            return Ok(None);
        }

        let v: Value =
            serde_json::from_str(line).map_err(|e| ProviderError::Serialization(e.to_string()))?;

        let choice = match v["choices"].as_array().and_then(|a| a.first()) {
            Some(c) => c,
            None => return Ok(None),
        };

        let content = choice["delta"]["content"].as_str().map(|s| s.to_string());
        let finish_reason = match choice["finish_reason"].as_str() {
            Some("stop") => Some(FinishReason::Stop),
            Some("length") => Some(FinishReason::MaxTokens),
            Some("tool_calls") => Some(FinishReason::ToolUse),
            Some("content_filter") => Some(FinishReason::ContentFilter),
            _ => None,
        };

        // If content is None and no finish_reason, it's usually an initial empty chunk
        Ok(Some(Delta {
            content,
            tool_call: None,
            finish_reason,
            usage: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_format() -> OpenAIFormat {
        OpenAIFormat::new()
    }

    #[test]
    fn test_format_request_basic() {
        let fmt = make_format();
        let messages = vec![Message::user("hello")];
        let opts = Options::new("gpt-4o");
        let body = fmt
            .format_request(&messages, &opts)
            .expect("format_request failed");

        assert_eq!(body["model"], "gpt-4o");
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "hello");
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn test_format_request_with_system() {
        let fmt = make_format();
        let messages = vec![Message::user("hello")];
        let opts = Options::new("gpt-4o").with_system("You are helpful");
        let body = fmt.format_request(&messages, &opts).unwrap();

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "You are helpful");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn test_format_request_stream_enabled() {
        let fmt = make_format();
        let messages = vec![Message::user("test")];
        let opts = Options::new("gpt-4o").with_stream();
        let body = fmt.format_request(&messages, &opts).unwrap();
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn test_parse_response_stop() {
        let fmt = make_format();
        let raw = serde_json::json!({
            "id": "chatcmpl-abc123",
            "model": "gpt-4o",
            "choices": [{
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let resp = fmt.parse_response(raw).unwrap();
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
        let fmt = make_format();
        let raw = serde_json::json!({
            "id": "chatcmpl-xyz",
            "model": "gpt-4o",
            "choices": [{
                "message": {"role": "assistant", "content": "..."},
                "finish_reason": "length"
            }],
            "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
        });
        let resp = fmt.parse_response(raw).unwrap();
        assert_eq!(resp.finish_reason, FinishReason::MaxTokens);
    }

    #[test]
    fn test_parse_stream_chunk_text() {
        let fmt = make_format();
        let line = r#"data: {"choices":[{"delta":{"content":"hello"},"finish_reason":null}]}"#;
        let delta = fmt.parse_stream_chunk(line).unwrap();
        assert!(delta.is_some());
        let d = delta.unwrap();
        assert_eq!(d.content, Some("hello".to_string()));
        assert!(d.finish_reason.is_none());
    }

    #[test]
    fn test_parse_stream_chunk_done() {
        let fmt = make_format();
        let result = fmt.parse_stream_chunk("data: [DONE]").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_stream_chunk_empty_content() {
        let fmt = make_format();
        let line = r#"data: {"choices":[{"delta":{"content":""},"finish_reason":null}]}"#;
        let delta = fmt.parse_stream_chunk(line).unwrap();
        assert!(delta.is_some());
        let d = delta.unwrap();
        assert_eq!(d.content, Some("".to_string()));
    }

    #[test]
    fn test_parse_response_missing_fields_error() {
        let fmt = make_format();
        let raw = serde_json::json!({});
        let result = fmt.parse_response(raw);
        assert!(result.is_err());
    }

    #[test]
    fn test_format_request_multiple_messages() {
        let fmt = make_format();
        let messages = vec![
            Message::user("first"),
            Message::assistant("second"),
            Message::user("third"),
        ];
        let opts = Options::new("gpt-4o");
        let body = fmt.format_request(&messages, &opts).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "user");
    }
}
