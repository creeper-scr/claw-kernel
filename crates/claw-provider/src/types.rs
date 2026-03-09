use crate::error::ProviderError;
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
    /// Create a user message.
    ///
    /// User messages represent input from the end user or application.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_provider::types::Message;
    ///
    /// let msg = Message::user("Hello, how are you?");
    /// assert_eq!(msg.content, "Hello, how are you?");
    /// ```
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Create an assistant message.
    ///
    /// Assistant messages represent responses from the LLM.
    /// Use this to add previous assistant responses to the conversation history.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_provider::types::Message;
    ///
    /// let msg = Message::assistant("I'm doing well, thank you!");
    /// assert_eq!(msg.content, "I'm doing well, thank you!");
    /// ```
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Create a system message.
    ///
    /// System messages set the behavior and context for the LLM.
    /// They are typically placed at the beginning of the conversation.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_provider::types::Message;
    ///
    /// let msg = Message::system("You are a helpful coding assistant.");
    /// assert_eq!(msg.content, "You are a helpful coding assistant.");
    /// ```
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Create a tool result message.
    ///
    /// Tool result messages return the output of a tool call back to the LLM.
    /// The `tool_call_id` must match the ID from the original tool call request.
    ///
    /// # Arguments
    ///
    /// * `tool_call_id` - The unique ID of the tool call being responded to
    /// * `content` - The result/output of the tool execution
    ///
    /// # Example
    ///
    /// ```
    /// use claw_provider::types::Message;
    ///
    /// let msg = Message::tool_result("call-123", "Current weather: 72°F, sunny");
    /// assert_eq!(msg.content, "Current weather: 72°F, sunny");
    /// assert_eq!(msg.tool_call_id, Some("call-123".to_string()));
    /// ```
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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
#[non_exhaustive]
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
    /// Model identifier (e.g., "claude-opus-4-6", "gpt-4o"). Required, not optional.
    pub model: String,
    /// Maximum tokens to generate. Required, not optional.
    pub max_tokens: u32,
    /// Sampling temperature (0.0–2.0). Required, not optional.
    ///
    /// **Core Specification:** The kernel strictly validates temperature within 0.0–2.0 range.
    /// For LLMs supporting wider ranges (e.g., 5.0), individual Provider implementations
    /// should perform their own value mapping in the request building layer.
    pub temperature: f32,
    /// Enable streaming. Required, not optional.
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
    /// Request timeout in seconds. Required, not optional.
    pub timeout_seconds: u64,
    /// Max retry attempts. Required, not optional.
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

    /// Set temperature with validation (0.0–2.0).
    ///
    /// # Temperature Specification
    /// - Core kernel enforces 0.0–2.0 range for cross-provider consistency
    /// - For LLMs supporting extended ranges (e.g., Gemini's 0.0–5.0),
    ///   Provider implementations should map values in their MessageFormat::build_request
    pub fn with_temperature(mut self, t: f32) -> Result<Self, ProviderError> {
        if !(0.0..=2.0).contains(&t) {
            return Err(ProviderError::InvalidTemperature(t));
        }
        self.temperature = t;
        Ok(self)
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
    use crate::error::ProviderError;

    #[test]
    fn test_options_temperature_validation() {
        // Test valid temperature values (0.0-2.0 range)
        let opts = Options::new("test-model")
            .with_temperature(0.5)
            .expect("0.5 should be valid");
        assert!((opts.temperature - 0.5f32).abs() < f32::EPSILON);

        // Test boundary values
        let opts = Options::new("test-model")
            .with_temperature(0.0)
            .expect("0.0 should be valid");
        assert!((opts.temperature - 0.0f32).abs() < f32::EPSILON);

        let opts = Options::new("test-model")
            .with_temperature(2.0)
            .expect("2.0 should be valid");
        assert!((opts.temperature - 2.0f32).abs() < f32::EPSILON);

        // Test invalid temperature values
        let result = Options::new("test-model").with_temperature(-0.1);
        assert!(result.is_err());

        let result = Options::new("test-model").with_temperature(2.1);
        assert!(result.is_err());
    }

    #[test]
    fn test_options_all_fields() {
        let opts = Options::new("test-model")
            .with_max_tokens(2048)
            .with_temperature(0.8)
            .expect("0.8 should be valid")
            .with_stream()
            .with_system("You are helpful.")
            .with_stop_sequences(vec!["STOP".to_string()])
            .with_timeout(30)
            .with_max_retries(5);

        assert_eq!(opts.model, "test-model");
        assert_eq!(opts.max_tokens, 2048);
        assert!((opts.temperature - 0.8f32).abs() < f32::EPSILON);
        assert!(opts.stream);
        assert_eq!(opts.system, Some("You are helpful.".to_string()));
        assert_eq!(opts.stop_sequences, vec!["STOP".to_string()]);
        assert_eq!(opts.timeout_seconds, 30);
        assert_eq!(opts.max_retries, 5);
    }

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
            .expect("valid temperature")
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
    fn test_options_invalid_temperature() {
        // Test temperature below valid range
        let result = Options::new("claude-opus-4-6").with_temperature(-0.1);
        assert!(result.is_err());
        match result {
            Err(ProviderError::InvalidTemperature(t)) => assert!((t + 0.1f32).abs() < 1e-6),
            _ => panic!("expected InvalidTemperature error for negative temperature"),
        }

        // Test temperature above valid range
        let result = Options::new("claude-opus-4-6").with_temperature(2.1);
        assert!(result.is_err());
        match result {
            Err(ProviderError::InvalidTemperature(t)) => assert!((t - 2.1f32).abs() < 1e-6),
            _ => panic!("expected InvalidTemperature error for temperature > 2.0"),
        }

        // Test boundary values (valid)
        let opts = Options::new("claude-opus-4-6")
            .with_temperature(0.0)
            .expect("0.0 should be valid");
        assert!((opts.temperature).abs() < 1e-6);

        let opts = Options::new("claude-opus-4-6")
            .with_temperature(2.0)
            .expect("2.0 should be valid");
        assert!((opts.temperature - 2.0f32).abs() < 1e-6);
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
