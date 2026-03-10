//! JSON-RPC 2.0 protocol types for KernelServer.
//!
//! Defines request/response/notification types for the IPC communication protocol.

use serde::{Deserialize, Serialize};

/// JSON-RPC 2.0 Request object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Method name to invoke.
    pub method: String,
    /// Method parameters (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    /// Request identifier (can be string, number, or null for notifications).
    pub id: Option<RequestId>,
}

impl Request {
    /// Creates a new JSON-RPC request.
    pub fn new(
        method: impl Into<String>,
        params: Option<serde_json::Value>,
        id: Option<RequestId>,
    ) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
            id,
        }
    }

    /// Returns true if this is a notification (no id).
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// JSON-RPC 2.0 Response object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Result of the method call (present if no error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error object (present if method call failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    /// Request identifier (matches the request).
    pub id: Option<RequestId>,
}

impl Response {
    /// Creates a successful response.
    pub fn success(result: serde_json::Value, id: Option<RequestId>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Creates an error response.
    pub fn error(
        code: i32,
        message: impl Into<String>,
        data: Option<serde_json::Value>,
        id: Option<RequestId>,
    ) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data,
            }),
            id,
        }
    }
}

/// JSON-RPC 2.0 Notification (request without id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Method name to invoke.
    pub method: String,
    /// Method parameters (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl Notification {
    /// Creates a new notification.
    pub fn new(method: impl Into<String>, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 Error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// Error code (integer).
    pub code: i32,
    /// Error message (short description).
    pub message: String,
    /// Additional error data (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Request identifier (string, number, or null).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// String identifier.
    String(String),
    /// Numeric identifier.
    Number(i64),
    /// Null identifier.
    Null,
}

/// Standard JSON-RPC 2.0 error codes.
pub mod error_codes {
    /// Parse error (-32700): Invalid JSON was received by the server.
    pub const PARSE_ERROR: i32 = -32700;
    /// Invalid Request (-32600): The JSON sent is not a valid Request object.
    pub const INVALID_REQUEST: i32 = -32600;
    /// Method not found (-32601): The method does not exist / is not available.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Invalid params (-32602): Invalid method parameter(s).
    pub const INVALID_PARAMS: i32 = -32602;
    /// Internal error (-32603): Internal JSON-RPC error.
    pub const INTERNAL_ERROR: i32 = -32603;
    /// Session not found (-32000): The session does not exist.
    pub const SESSION_NOT_FOUND: i32 = -32000;
    /// Max sessions reached (-32001): Maximum number of sessions exceeded.
    pub const MAX_SESSIONS_REACHED: i32 = -32001;
    /// Provider error (-32002): LLM provider error.
    pub const PROVIDER_ERROR: i32 = -32002;
    /// Agent error (-32003): Agent loop error.
    pub const AGENT_ERROR: i32 = -32003;
    /// Daemon already running (-32004): Another daemon instance is already running.
    pub const DAEMON_ALREADY_RUNNING: i64 = -32004;
    /// Provider not found (-32005): The requested provider is not registered.
    pub const PROVIDER_NOT_FOUND: i32 = -32005;
}

/// Configuration provided at session creation time.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionConfig {
    /// System prompt to use for this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Maximum number of conversation turns (default: 20).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// Override the default provider ("anthropic", "openai", "ollama", etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_override: Option<String>,
    /// Override the default model name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// External tools the client will provide implementations for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ExternalToolDef>>,
    /// Whether to persist conversation history to SQLite.
    #[serde(default)]
    pub persist_history: bool,
}

/// Definition of a client-side external tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalToolDef {
    /// Tool name (snake_case).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: serde_json::Value,
}

/// Parameters for `createSession` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionParams {
    /// Optional session configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<SessionConfig>,
}

/// Parameters for `sendMessage` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageParams {
    /// Session ID.
    pub session_id: String,
    /// Message content to send.
    pub content: String,
    /// Optional message metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Parameters for `toolResult` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultParams {
    /// Session ID.
    pub session_id: String,
    /// Tool call ID.
    pub tool_call_id: String,
    /// Tool result content.
    pub result: serde_json::Value,
    /// Whether the tool execution was successful.
    pub success: bool,
}

/// Parameters for `destroySession` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestroySessionParams {
    /// Session ID to destroy.
    pub session_id: String,
}

/// Parameters for stream chunk notification (`agent/streamChunk`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkParams {
    /// Session ID.
    pub session_id: String,
    /// Chunk content (delta text).
    pub delta: String,
    /// Whether this is the final chunk.
    pub done: bool,
}

/// Parameters for tool call notification (`agent/toolCall`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallParams {
    /// Session ID.
    pub session_id: String,
    /// Tool call ID.
    pub tool_call_id: String,
    /// Tool name.
    pub tool_name: String,
    /// Tool arguments.
    pub arguments: serde_json::Value,
}

/// Parameters for finish notification (`agent/finish`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinishParams {
    /// Session ID.
    pub session_id: String,
    /// Final response content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Stop reason.
    pub reason: String,
}

/// Usage information (token counts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInfo {
    /// Input tokens consumed.
    pub input_tokens: u32,
    /// Output tokens consumed.
    pub output_tokens: u32,
    /// Total tokens consumed.
    pub total_tokens: u32,
}

/// Result of `kernel.info` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelInfoResult {
    /// Kernel version (from CARGO_PKG_VERSION).
    pub version: String,
    /// Protocol version (current: 2).
    pub protocol_version: u32,
    /// List of compiled provider names.
    pub providers: Vec<String>,
    /// Name of the active (default) provider.
    pub active_provider: String,
    /// Current default model name.
    pub active_model: String,
    /// List of enabled features.
    pub features: Vec<String>,
    /// Maximum allowed sessions.
    pub max_sessions: usize,
    /// Current active session count.
    pub current_sessions: usize,
}

/// JSON-RPC 2.0 Notification (server-push, no id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// Parameters for events.subscribe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsSubscribeParams {
    pub session_id: String,
    /// Filter: "all", "agent_lifecycle", "tool_calls", "llm_requests", "a2a", "shutdown"
    #[serde(default = "default_event_filter")]
    pub filter: String,
}

fn default_event_filter() -> String {
    "all".to_string()
}

/// Parameters for events.unsubscribe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsUnsubscribeParams {
    pub session_id: String,
}

/// Parameters for schedule.create.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleCreateParams {
    pub session_id: String,
    /// Cron expression or "once" for one-shot.
    pub cron: String,
    /// The agent message / prompt to run.
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Parameters for schedule.cancel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleCancelParams {
    pub task_id: String,
}

/// Parameters for schedule.list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleListParams {
    pub session_id: String,
}

/// Information about a scheduled task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTaskInfo {
    pub task_id: String,
    pub cron: String,
    pub label: Option<String>,
    pub status: String,
}

/// Parameters for channel.create.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelCreateParams {
    pub session_id: String,
    /// Channel type: "websocket"
    pub channel_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// Parameters for channel.send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSendParams {
    pub channel_id: String,
    pub message: String,
}

/// Parameters for channel.close.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelCloseParams {
    pub channel_id: String,
}

// ─── B1: Channel API (register/unregister/list) ───────────────────────────────

/// Parameters for `channel.register` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelRegisterParams {
    /// Channel type: "webhook" | "stdin" | "discord"
    pub r#type: String,
    /// Unique channel identifier.
    pub channel_id: String,
    /// Type-specific configuration (JSON object).
    pub config: serde_json::Value,
}

/// Parameters for `channel.unregister` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelUnregisterParams {
    /// Channel identifier to unregister.
    pub channel_id: String,
}

// ─── B2: Trigger API ──────────────────────────────────────────────────────────

/// Parameters for `trigger.add_cron` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerAddCronParams {
    /// Unique trigger identifier.
    pub trigger_id: String,
    /// Cron expression (e.g. "0 * * * *").
    pub cron_expr: String,
    /// Target agent ID to fire the trigger against.
    pub target_agent: String,
    /// Optional message/prompt injected when the trigger fires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Parameters for `trigger.add_webhook` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerAddWebhookParams {
    /// Unique trigger identifier.
    pub trigger_id: String,
    /// Target agent ID.
    pub target_agent: String,
    /// Optional HMAC secret for webhook verification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hmac_secret: Option<String>,
}

/// Parameters for `trigger.remove` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerRemoveParams {
    /// Trigger identifier to remove.
    pub trigger_id: String,
}

// ─── B3: Agent API ────────────────────────────────────────────────────────────

/// Parameters for `agent.spawn` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnParams {
    /// Optional pre-assigned agent ID (UUID generated if omitted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Agent configuration.
    pub config: AgentSpawnConfig,
}

/// Agent configuration for `agent.spawn`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnConfig {
    /// System prompt override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Provider override (e.g. "anthropic", "openai").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Maximum turns override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

/// Parameters for `agent.kill` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentKillParams {
    /// Agent ID to stop.
    pub agent_id: String,
}

/// Parameters for `agent.steer` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSteerParams {
    /// Agent ID to inject a message into.
    pub agent_id: String,
    /// Message to inject.
    pub message: String,
}

// REMOVED in v1.3.0: memory.search / memory.store are application-layer concerns.
// See docs/kernel-gap-analysis.md § D1 for rationale.

// ─── B5: Tool API ─────────────────────────────────────────────────────────────

/// Parameters for `tool.register` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRegisterParams {
    /// Tool name (snake_case).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub schema: serde_json::Value,
    /// Executor type: "external" | "inline".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executor: Option<String>,
}

/// Parameters for `tool.unregister` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUnregisterParams {
    /// Tool name to unregister.
    pub name: String,
}

// ─── B6: Skill API ────────────────────────────────────────────────────────────

/// Parameters for `skill.load_dir` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillLoadDirParams {
    /// Filesystem path to the skills directory.
    pub path: String,
}

/// Parameters for `skill.get_full` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillGetFullParams {
    /// Skill name.
    pub name: String,
}

/// Parameters for `provider.register` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRegisterParams {
    /// Provider name to register under.
    pub name: String,
    /// Provider type (e.g. "openai", "anthropic", "ollama").
    pub provider_type: String,
    /// API key (if required).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Base URL override (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Model name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

// ─── Phase 3: Channel routing API ─────────────────────────────────────────────

/// Parameters for `channel.route_add` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelRouteAddParams {
    /// Rule type: "channel" | "sender" | "pattern" | "default"
    pub rule_type: String,
    /// Channel ID (for "channel" rules).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Sender ID (for "sender" rules).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_id: Option<String>,
    /// Regex pattern (for "pattern" rules).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Target agent ID.
    pub agent_id: String,
}

/// Parameters for `channel.route_remove` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelRouteRemoveParams {
    /// Remove all rules targeting this agent.
    pub agent_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let request = Request::new(
            "createSession",
            Some(serde_json::json!({ "config": {} })),
            Some(RequestId::String("1".to_string())),
        );
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"createSession\""));
        assert!(json.contains("\"id\":\"1\""));
    }

    #[test]
    fn test_response_success() {
        let response = Response::success(
            serde_json::json!({ "session_id": "abc-123" }),
            Some(RequestId::String("1".to_string())),
        );
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn test_response_error() {
        let response = Response::error(
            error_codes::SESSION_NOT_FOUND,
            "Session not found",
            None,
            Some(RequestId::String("1".to_string())),
        );
        assert!(response.result.is_none());
        assert!(response.error.is_some());
        let err = response.error.unwrap();
        assert_eq!(err.code, -32000);
    }

    #[test]
    fn test_notification() {
        let notification = Notification::new(
            "agent/streamChunk",
            Some(serde_json::json!({ "session_id": "abc", "delta": "Hello", "done": false })),
        );
        assert_eq!(notification.jsonrpc, "2.0");
        assert_eq!(notification.method, "agent/streamChunk");
    }

    #[test]
    fn test_request_id_variants() {
        let id_str: RequestId = serde_json::from_str("\"test-id\"").unwrap();
        assert!(matches!(id_str, RequestId::String(s) if s == "test-id"));

        let id_num: RequestId = serde_json::from_str("42").unwrap();
        assert!(matches!(id_num, RequestId::Number(42)));

        let id_null: RequestId = serde_json::from_str("null").unwrap();
        assert!(matches!(id_null, RequestId::Null));
    }

    #[test]
    fn test_is_notification() {
        let notification = Request::new("test", None, None);
        assert!(notification.is_notification());

        let request = Request::new("test", None, Some(RequestId::Number(1)));
        assert!(!request.is_notification());
    }

    #[test]
    fn test_create_session_params() {
        let params = CreateSessionParams {
            config: Some(SessionConfig {
                system_prompt: Some("You are helpful".to_string()),
                ..Default::default()
            }),
        };
        let json = serde_json::to_string(&params).unwrap();
        assert!(json.contains("system_prompt"));
    }

    #[test]
    fn test_create_session_params_deserialization() {
        let json = r#"{"config": {"system_prompt": "You are helpful"}}"#;
        let params: CreateSessionParams = serde_json::from_str(json).unwrap();
        assert!(params.config.is_some());
        assert_eq!(params.config.unwrap().system_prompt.as_deref(), Some("You are helpful"));
    }

    #[test]
    fn test_external_tool_def_deserialization() {
        let json = r#"{
            "name": "get_weather",
            "description": "Get weather for a city",
            "input_schema": {"type": "object", "properties": {"city": {"type": "string"}}}
        }"#;
        let tool: ExternalToolDef = serde_json::from_str(json).unwrap();
        assert_eq!(tool.name, "get_weather");
    }

    #[test]
    fn test_send_message_params() {
        let params = SendMessageParams {
            session_id: "session-1".to_string(),
            content: "Hello, world!".to_string(),
            metadata: None,
        };
        let json = serde_json::to_string(&params).unwrap();
        assert!(json.contains("session_id"));
        assert!(json.contains("content"));
    }

    #[test]
    fn test_tool_result_params() {
        let params = ToolResultParams {
            session_id: "session-1".to_string(),
            tool_call_id: "call-1".to_string(),
            result: serde_json::json!({ "output": "result" }),
            success: true,
        };
        assert_eq!(params.session_id, "session-1");
        assert!(params.success);
    }

    #[test]
    fn test_destroy_session_params() {
        let params = DestroySessionParams {
            session_id: "session-1".to_string(),
        };
        assert_eq!(params.session_id, "session-1");
    }

    #[test]
    fn test_chunk_params() {
        let params = ChunkParams {
            session_id: "session-1".to_string(),
            delta: "Hello".to_string(),
            done: false,
        };
        assert_eq!(params.delta, "Hello");
        assert!(!params.done);
    }

    #[test]
    fn test_tool_call_params() {
        let params = ToolCallParams {
            session_id: "session-1".to_string(),
            tool_call_id: "call-1".to_string(),
            tool_name: "read_file".to_string(),
            arguments: serde_json::json!({ "path": "/tmp/file" }),
        };
        assert_eq!(params.tool_name, "read_file");
    }

    #[test]
    fn test_finish_params() {
        let params = FinishParams {
            session_id: "session-1".to_string(),
            content: Some("Done!".to_string()),
            reason: "completed".to_string(),
        };
        assert_eq!(params.reason, "completed");
    }

    #[test]
    fn test_usage_info() {
        let usage = UsageInfo {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
        };
        assert_eq!(usage.total_tokens, 150);
    }
}
