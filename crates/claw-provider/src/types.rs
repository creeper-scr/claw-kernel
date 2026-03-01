use serde::{Deserialize, Serialize};

/// Role in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

/// A tool call request from the LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this tool call.
    pub id: String,
    /// Tool name to invoke.
    pub name: String,
    /// JSON-serialized arguments.
    pub arguments: String,
}

/// A tool call result to return to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub tool_call_id: String,
    pub content: String,
}

/// A single message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    /// Text content. Empty string if this is a tool call message.
    pub content: String,
    /// Tool calls from the assistant (Some when role = Assistant and LLM made calls).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Tool result (Some when role = Tool).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl TokenUsage {
    pub fn new(prompt: u64, completion: u64) -> Self {
        Self {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
        }
    }
}

/// Reason why the LLM stopped generating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    /// Generation stopped due to token length limit.
    Length,
    /// Generation stopped because the model made tool calls.
    ToolCalls,
    ContentFilter,
    /// Other reasons (provider-specific).
    Other(String),
}

/// A complete (non-streaming) response from an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// Provider-assigned response ID.
    pub id: String,
    /// Model that generated the response.
    pub model: String,
    /// The assistant's reply message.
    pub message: Message,
    /// Why generation stopped.
    pub finish_reason: FinishReason,
    /// Token usage.
    pub usage: TokenUsage,
}

/// A streaming chunk / delta from an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    /// Incremental text content (may be empty for tool call chunks).
    pub content: Option<String>,
    /// Partial or complete tool call (accumulate across chunks).
    pub tool_call: Option<ToolCall>,
    /// Set on the final chunk.
    pub finish_reason: Option<FinishReason>,
    /// Token usage (typically only on final chunk).
    pub usage: Option<TokenUsage>,
}

/// Tool definition for function calling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// JSON Schema for tool parameters.
    pub parameters: serde_json::Value,
}

/// Options for LLM completion requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    /// Model identifier (e.g., "claude-opus-4-6", "gpt-4o").
    pub model: String,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature (0.0–2.0).
    pub temperature: f32,
    /// Enable streaming.
    pub stream: bool,
    /// System prompt (overrides any system message in the message list).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Stop sequences to end generation.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,
    /// Available tools for function calling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
    /// Request timeout in seconds.
    pub timeout_seconds: u64,
    /// Max retry attempts.
    pub max_retries: u32,
}

impl Options {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            max_tokens: 4096,
            temperature: 0.7,
            stream: false,
            system: None,
            stop_sequences: Vec::new(),
            tools: None,
            timeout_seconds: 60,
            max_retries: 3,
        }
    }

    pub fn with_stream(mut self) -> Self {
        self.stream = true;
        self
    }

    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }

    pub fn with_temperature(mut self, t: f32) -> Self {
        self.temperature = t;
        self
    }

    pub fn with_system(mut self, sys: impl Into<String>) -> Self {
        self.system = Some(sys.into());
        self
    }

    pub fn with_stop_sequences(mut self, sequences: Vec<String>) -> Self {
        self.stop_sequences = sequences;
        self
    }

    pub fn with_tools(mut self, tools: Vec<ToolDef>) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_seconds = timeout_secs;
        self
    }

    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }
}

/// Embedding vector.
pub type Embedding = Vec<f32>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_user_constructor() {
        let msg = Message::user("hello world");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "hello world");
        assert!(msg.tool_calls.is_none());
        assert!(msg.tool_call_id.is_none());
    }

    #[test]
    fn test_message_assistant_constructor() {
        let msg = Message::assistant("I can help you");
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "I can help you");
        assert!(msg.tool_calls.is_none());
        assert!(msg.tool_call_id.is_none());
    }

    #[test]
    fn test_message_tool_result_constructor() {
        let msg = Message::tool_result("call-123", "tool output here");
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.content, "tool output here");
        assert!(msg.tool_calls.is_none());
        assert_eq!(msg.tool_call_id, Some("call-123".to_string()));
    }

    #[test]
    fn test_options_builder_pattern() {
        let opts = Options::new("claude-opus-4-6")
            .with_max_tokens(8192)
            .with_temperature(0.3)
            .with_stream()
            .with_system("You are a helpful assistant.");

        assert_eq!(opts.model, "claude-opus-4-6");
        assert_eq!(opts.max_tokens, 8192);
        assert!((opts.temperature - 0.3f32).abs() < 1e-6);
        assert!(opts.stream);
        assert_eq!(
            opts.system,
            Some("You are a helpful assistant.".to_string())
        );
    }

    #[test]
    fn test_message_serialize_deserialize() {
        let original = Message::user("test content");
        let json = serde_json::to_string(&original).expect("serialize failed");
        let restored: Message = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(restored.role, original.role);
        assert_eq!(restored.content, original.content);
        assert_eq!(restored.tool_calls, original.tool_calls);
        assert_eq!(restored.tool_call_id, original.tool_call_id);
    }
}
