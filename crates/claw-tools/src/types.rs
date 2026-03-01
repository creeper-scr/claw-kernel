use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

// ─── ToolResult / ToolError ─────────────────────────────────────────────────

/// Standard error codes for tool execution failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCode {
    /// Invalid parameters passed to the tool.
    InvalidParameter,
    /// Tool execution failed (generic execution error).
    ExecutionFailed,
    /// Execution timed out.
    Timeout,
    /// Permission denied for the requested operation.
    PermissionDenied,
    /// Resource not found.
    ResourceNotFound,
    /// Rate limited by external service.
    RateLimited,
    /// Internal error.
    InternalError,
}

/// Tool execution error with code and message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
}

impl ToolError {
    pub fn invalid_args(msg: impl Into<String>) -> Self {
        Self {
            code: ToolErrorCode::InvalidParameter,
            message: msg.into(),
        }
    }
    pub fn permission_denied(msg: impl Into<String>) -> Self {
        Self {
            code: ToolErrorCode::PermissionDenied,
            message: msg.into(),
        }
    }
    pub fn timeout() -> Self {
        Self {
            code: ToolErrorCode::Timeout,
            message: "execution timed out".to_string(),
        }
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: ToolErrorCode::InternalError,
            message: msg.into(),
        }
    }
}

/// Result of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Whether the tool executed successfully.
    pub success: bool,
    /// The output (present on success).
    pub output: Option<serde_json::Value>,
    /// The error (present on failure).
    pub error: Option<ToolError>,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
}

impl ToolResult {
    pub fn ok(output: serde_json::Value, duration_ms: u64) -> Self {
        Self {
            success: true,
            output: Some(output),
            error: None,
            duration_ms,
        }
    }
    pub fn err(error: ToolError, duration_ms: u64) -> Self {
        Self {
            success: false,
            output: None,
            error: Some(error),
            duration_ms,
        }
    }
}

// ─── Schema / Permissions ───────────────────────────────────────────────────

/// JSON Schema for a tool parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// Tool name (snake_case).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for input parameters.
    pub parameters: serde_json::Value,
}

impl ToolSchema {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// Filesystem permission for a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsPermissions {
    /// Allowed read paths (glob patterns or absolute paths).
    pub read_paths: HashSet<String>,
    /// Allowed write paths.
    pub write_paths: HashSet<String>,
}

impl FsPermissions {
    pub fn none() -> Self {
        Self {
            read_paths: HashSet::new(),
            write_paths: HashSet::new(),
        }
    }
    pub fn read_only(paths: impl IntoIterator<Item = String>) -> Self {
        Self {
            read_paths: paths.into_iter().collect(),
            write_paths: HashSet::new(),
        }
    }
}

/// Network permissions for a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPermissions {
    /// Allowed domains (e.g., "api.example.com"). Empty = no network.
    pub allowed_domains: HashSet<String>,
}

impl NetworkPermissions {
    pub fn none() -> Self {
        Self {
            allowed_domains: HashSet::new(),
        }
    }
    pub fn allow(domains: impl IntoIterator<Item = String>) -> Self {
        Self {
            allowed_domains: domains.into_iter().collect(),
        }
    }
}

/// Subprocess policy for a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubprocessPolicy {
    Denied,
    Allowed,
}

/// Complete permission set for a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionSet {
    pub filesystem: FsPermissions,
    pub network: NetworkPermissions,
    pub subprocess: SubprocessPolicy,
}

impl PermissionSet {
    /// No permissions (read-only, no network, no subprocess).
    pub fn minimal() -> Self {
        Self {
            filesystem: FsPermissions::none(),
            network: NetworkPermissions::none(),
            subprocess: SubprocessPolicy::Denied,
        }
    }
}

// ─── ToolMeta ───────────────────────────────────────────────────────────────

/// Metadata for a registered tool (snapshot — no dyn Tool reference).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMeta {
    pub schema: ToolSchema,
    pub permissions: PermissionSet,
    pub timeout: Duration,
    /// Source file path (for hot-loaded tools).
    pub source_path: Option<String>,
}

// ─── Execution context ──────────────────────────────────────────────────────

/// Context passed to tool::execute().
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// ID of the calling agent.
    pub agent_id: String,
    /// Permissions granted for this execution.
    pub permissions: PermissionSet,
}

impl ToolContext {
    pub fn new(agent_id: impl Into<String>, permissions: PermissionSet) -> Self {
        Self {
            agent_id: agent_id.into(),
            permissions,
        }
    }
}

// ─── HotLoading ─────────────────────────────────────────────────────────────

/// Configuration for hot-loading tool scripts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotLoadingConfig {
    /// Directory to watch for tool scripts.
    pub watch_dir: String,
    /// File extensions to load (e.g., ["lua", "js"]).
    pub extensions: Vec<String>,
    /// Debounce delay in milliseconds (default 50ms).
    pub debounce_ms: u64,
    /// Maximum tool execution timeout (default 30s).
    pub default_timeout_secs: u64,
}

impl Default for HotLoadingConfig {
    fn default() -> Self {
        Self {
            watch_dir: "tools".to_string(),
            extensions: vec!["lua".to_string()],
            debounce_ms: 50,
            default_timeout_secs: 30,
        }
    }
}

// ─── Audit log ──────────────────────────────────────────────────────────────

/// Audit log entry for a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub agent_id: String,
    pub tool_name: String,
    pub success: bool,
    pub duration_ms: u64,
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_result_ok() {
        let output = serde_json::json!({"key": "value"});
        let result = ToolResult::ok(output.clone(), 42);
        assert!(result.success);
        assert_eq!(result.output.as_ref().unwrap(), &output);
        assert!(result.error.is_none());
        assert_eq!(result.duration_ms, 42);
    }

    #[test]
    fn test_tool_result_err() {
        let error = ToolError::timeout();
        let result = ToolResult::err(error, 5000);
        assert!(!result.success);
        assert!(result.output.is_none());
        let err = result.error.as_ref().unwrap();
        assert_eq!(err.code, ToolErrorCode::Timeout);
        assert_eq!(result.duration_ms, 5000);
    }

    #[test]
    fn test_tool_error_variants() {
        let e1 = ToolError::invalid_args("bad input");
        assert_eq!(e1.code, ToolErrorCode::InvalidParameter);
        assert_eq!(e1.message, "bad input");

        let e2 = ToolError::permission_denied("no access");
        assert_eq!(e2.code, ToolErrorCode::PermissionDenied);

        let e3 = ToolError::timeout();
        assert_eq!(e3.code, ToolErrorCode::Timeout);

        let e4 = ToolError::internal("crash");
        assert_eq!(e4.code, ToolErrorCode::InternalError);
        assert_eq!(e4.message, "crash");
    }

    #[test]
    fn test_permission_set_minimal() {
        let perms = PermissionSet::minimal();
        assert!(perms.filesystem.read_paths.is_empty());
        assert!(perms.filesystem.write_paths.is_empty());
        assert!(perms.network.allowed_domains.is_empty());
        assert_eq!(perms.subprocess, SubprocessPolicy::Denied);
    }

    #[test]
    fn test_hot_loading_config_default() {
        let config = HotLoadingConfig::default();
        assert_eq!(config.watch_dir, "tools");
        assert_eq!(config.extensions, vec!["lua"]);
        assert_eq!(config.debounce_ms, 50);
        assert_eq!(config.default_timeout_secs, 30);
    }
}
