//! Async audit log system for tool execution events.
//!
//! Provides persistent audit logging with async file writes and
//! in-memory index of recent events.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod writer;

pub use writer::{AuditLogWriter, AuditLogWriterHandle};

/// Types of audit events that can be logged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEvent {
    /// Tool execution started.
    ToolCall {
        timestamp_ms: u64,
        agent_id: String,
        tool_name: String,
        args: Option<serde_json::Value>,
    },
    /// Tool execution completed.
    ToolResult {
        timestamp_ms: u64,
        agent_id: String,
        tool_name: String,
        success: bool,
        duration_ms: u64,
        error_code: Option<String>,
    },
    /// Permission check performed.
    PermissionCheck {
        timestamp_ms: u64,
        agent_id: String,
        tool_name: String,
        permission: String,
        granted: bool,
    },
    /// Mode switch event (Safe Mode ↔ Power Mode).
    ModeSwitch {
        timestamp_ms: u64,
        agent_id: String,
        from_mode: String,
        to_mode: String,
        reason: String,
    },
}

impl AuditEvent {
    /// Get the event type name for logging.
    pub fn event_type(&self) -> &'static str {
        match self {
            AuditEvent::ToolCall { .. } => "TOOL_CALL",
            AuditEvent::ToolResult { .. } => "TOOL_RESULT",
            AuditEvent::PermissionCheck { .. } => "PERMISSION_CHECK",
            AuditEvent::ModeSwitch { .. } => "MODE_SWITCH",
        }
    }

    /// Get the timestamp of the event.
    pub fn timestamp_ms(&self) -> u64 {
        match self {
            AuditEvent::ToolCall { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::ToolResult { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::PermissionCheck { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::ModeSwitch { timestamp_ms, .. } => *timestamp_ms,
        }
    }

    /// Get the agent ID associated with the event.
    pub fn agent_id(&self) -> &str {
        match self {
            AuditEvent::ToolCall { agent_id, .. } => agent_id,
            AuditEvent::ToolResult { agent_id, .. } => agent_id,
            AuditEvent::PermissionCheck { agent_id, .. } => agent_id,
            AuditEvent::ModeSwitch { agent_id, .. } => agent_id,
        }
    }
}

/// Configuration for the audit log system.
#[derive(Debug, Clone)]
pub struct AuditLogConfig {
    /// Directory where audit logs are stored.
    pub log_dir: PathBuf,
    /// Maximum number of entries to keep in memory index.
    pub max_memory_entries: usize,
    /// Flush interval in seconds.
    pub flush_interval_secs: u64,
    /// Maximum log file size in bytes before rotation.
    pub max_file_size_bytes: u64,
    /// Log file name.
    pub log_filename: String,
}

impl Default for AuditLogConfig {
    fn default() -> Self {
        // Default to ~/.local/share/claw-kernel/logs/audit.log
        let log_dir = dirs::data_dir()
            .map(|d| d.join("claw-kernel").join("logs"))
            .unwrap_or_else(|| PathBuf::from("/tmp/claw-kernel/logs"));

        Self {
            log_dir,
            max_memory_entries: 1000,
            flush_interval_secs: 1,
            max_file_size_bytes: 10 * 1024 * 1024, // 10MB
            log_filename: "audit.log".to_string(),
        }
    }
}

impl AuditLogConfig {
    /// Create a new config with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the log directory.
    pub fn with_log_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.log_dir = dir.into();
        self
    }

    /// Set the maximum number of memory entries.
    pub fn with_max_memory_entries(mut self, max: usize) -> Self {
        self.max_memory_entries = max;
        self
    }

    /// Set the flush interval in seconds.
    pub fn with_flush_interval(mut self, secs: u64) -> Self {
        self.flush_interval_secs = secs;
        self
    }

    /// Get the full log file path.
    pub fn log_path(&self) -> PathBuf {
        self.log_dir.join(&self.log_filename)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_types() {
        let call = AuditEvent::ToolCall {
            timestamp_ms: 1,
            agent_id: "a1".to_string(),
            tool_name: "test".to_string(),
            args: None,
        };
        assert_eq!(call.event_type(), "TOOL_CALL");

        let result = AuditEvent::ToolResult {
            timestamp_ms: 2,
            agent_id: "a1".to_string(),
            tool_name: "test".to_string(),
            success: true,
            duration_ms: 100,
            error_code: None,
        };
        assert_eq!(result.event_type(), "TOOL_RESULT");

        let perm = AuditEvent::PermissionCheck {
            timestamp_ms: 3,
            agent_id: "a1".to_string(),
            tool_name: "test".to_string(),
            permission: "fs:read".to_string(),
            granted: true,
        };
        assert_eq!(perm.event_type(), "PERMISSION_CHECK");

        let mode = AuditEvent::ModeSwitch {
            timestamp_ms: 4,
            agent_id: "a1".to_string(),
            from_mode: "safe".to_string(),
            to_mode: "power".to_string(),
            reason: "user_request".to_string(),
        };
        assert_eq!(mode.event_type(), "MODE_SWITCH");
    }

    #[test]
    fn test_audit_log_config_default() {
        let config = AuditLogConfig::default();
        assert_eq!(config.max_memory_entries, 1000);
        assert_eq!(config.flush_interval_secs, 1);
        assert_eq!(config.max_file_size_bytes, 10 * 1024 * 1024);
        assert_eq!(config.log_filename, "audit.log");
    }

    #[test]
    fn test_audit_log_config_builder() {
        let config = AuditLogConfig::new()
            .with_max_memory_entries(500)
            .with_flush_interval(5);

        assert_eq!(config.max_memory_entries, 500);
        assert_eq!(config.flush_interval_secs, 5);
    }
}
