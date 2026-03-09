// Ollama uses the OpenAI-compatible Chat Completions API format.
// Re-export OpenAIFormat for use within this module.
pub use crate::openai::format::OpenAIFormat as OllamaFormat;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        openai::format::{OpenAIChoice, OpenAIResponse, OpenAIResponseMessage, OpenAIUsage},
        traits::MessageFormat,
        types::{FinishReason, Message, Options},
    };

    /// OllamaFormat 是 OpenAIFormat 的类型别名 — 验证 user message 被正确序列化为请求体
    #[test]
    fn test_user_message_format() {
        let messages = vec![Message::user("tell me a joke")];
        let opts = Options::new("llama3");
        let req = OllamaFormat::build_request(&messages, &opts);

        assert_eq!(req.model, "llama3");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, "user");
        assert_eq!(req.messages[0].content, "tell me a joke");
        assert!(!req.stream);
    }

    /// 验证 assistant message 被正确映射为 role="assistant"
    #[test]
    fn test_assistant_message_format() {
        let messages = vec![
            Message::user("hi"),
            Message::assistant("Hello! How can I help?"),
        ];
        let opts = Options::new("llama3");
        let req = OllamaFormat::build_request(&messages, &opts);

        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0].role, "user");
        assert_eq!(req.messages[1].role, "assistant");
        assert_eq!(req.messages[1].content, "Hello! How can I help?");
    }

    /// 验证 system message 被正确插入到消息列表头部
    #[test]
    fn test_system_message_format() {
        let messages = vec![Message::user("hello")];
        let opts = Options::new("llama3").with_system("You are a helpful assistant.");
        let req = OllamaFormat::build_request(&messages, &opts);

        // system 消息应在第一位
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0].role, "system");
        assert_eq!(req.messages[0].content, "You are a helpful assistant.");
        assert_eq!(req.messages[1].role, "user");
    }

    /// 验证 finish_reason="tool_calls" 被正确映射为 FinishReason::ToolCalls
    #[test]
    fn test_tool_call_response_format() {
        let raw = OpenAIResponse {
            id: "chatcmpl-ollama-1".to_string(),
            model: "llama3".to_string(),
            choices: vec![OpenAIChoice {
                message: OpenAIResponseMessage {
                    role: "assistant".to_string(),
                    content: "".to_string(),
                    tool_calls: None,
                },
                finish_reason: "tool_calls".to_string(),
            }],
            usage: OpenAIUsage {
                prompt_tokens: 10,
                completion_tokens: 0,
                total_tokens: 10,
            },
        };
        let resp = OllamaFormat::parse_response(raw).unwrap();
        assert_eq!(resp.finish_reason, FinishReason::ToolCalls);
    }

    /// 验证 OllamaFormat 使用 OpenAI 兼容的 endpoint（Ollama 服务器通过 /v1 前缀支持此路径）
    #[test]
    fn test_endpoint() {
        // Ollama 通过 /v1/chat/completions 提供 OpenAI 兼容接口
        assert_eq!(OllamaFormat::endpoint(), "/v1/chat/completions");
    }

    /// 验证 token 估算逻辑（4 chars ≈ 1 token）
    #[test]
    fn test_token_count() {
        let messages = vec![
            Message::user("hello world"), // 11 chars → 2 tokens
            Message::assistant("okay"),   // 4 chars → 1 token
        ];
        // (11 + 4) / 4 = 3
        assert_eq!(OllamaFormat::token_count(&messages), 3);
    }

    /// 验证流式请求模式被正确设置
    #[test]
    fn test_stream_flag() {
        let messages = vec![Message::user("stream test")];
        let opts = Options::new("mistral").with_stream();
        let req = OllamaFormat::build_request(&messages, &opts);
        assert!(req.stream);
    }
}
