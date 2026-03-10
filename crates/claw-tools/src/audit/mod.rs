//! Async audit log system for tool execution events.
//!
//! Provides persistent audit logging with async file writes, HMAC-SHA256
//! per-record signatures (tamper detection), and an in-memory index of
//! recent events for fast IPC queries.
//!
//! # Tamper detection
//!
//! When `AuditLogConfig::hmac_key` is set, every record written to disk is
//! wrapped in `AuditRecord { payload, signature }`.  The `payload` is the
//! JSON-serialised `AuditEvent` and `signature` is a hex-encoded
//! HMAC-SHA256 over that payload.  A verifier can replay the file and
//! recompute signatures to detect any post-write tampering.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

mod writer;

pub use writer::{AuditLogWriter, AuditLogWriterHandle};

/// In-memory ring buffer of recent audit events, queryable via IPC (`audit.list`).
///
/// Holds at most `max_entries` events. When full, the oldest entry is dropped.
/// Thread-safe via `Mutex`; intended to be shared as `Arc<AuditStore>`.
#[derive(Debug)]
pub struct AuditStore {
    entries: Mutex<VecDeque<AuditEvent>>,
    max_entries: usize,
}

impl AuditStore {
    /// Create a new store with the given capacity.
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(max_entries)),
            max_entries,
        }
    }

    /// Push an event into the ring buffer, evicting the oldest if full.
    pub fn push(&self, event: AuditEvent) {
        if let Ok(mut q) = self.entries.lock() {
            if q.len() >= self.max_entries {
                q.pop_front();
            }
            q.push_back(event);
        }
    }

    /// Query stored events, optionally filtered.
    ///
    /// - `limit`: max number of entries to return (most-recent-first). Defaults to 100.
    /// - `agent_id`: if set, only events from this agent are returned.
    /// - `since_ms`: if set, only events with `timestamp_ms >= since_ms` are returned.
    pub fn list(
        &self,
        limit: usize,
        agent_id: Option<&str>,
        since_ms: Option<u64>,
    ) -> Vec<AuditEvent> {
        let q = match self.entries.lock() {
            Ok(q) => q,
            Err(_) => return vec![],
        };
        q.iter()
            .rev()
            .filter(|e| {
                if let Some(aid) = agent_id {
                    if e.agent_id() != aid {
                        return false;
                    }
                }
                if let Some(since) = since_ms {
                    if e.timestamp_ms() < since {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .cloned()
            .collect()
    }
}

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
    /// Agent spawned by the orchestrator.
    AgentSpawned {
        timestamp_ms: u64,
        agent_id: String,
        agent_name: String,
        restart_policy: String,
    },
    /// Script read a file via FsBridge.
    ScriptFsRead {
        timestamp_ms: u64,
        agent_id: String,
        path: String,
        bytes: usize,
    },
    /// Script wrote a file via FsBridge.
    ScriptFsWrite {
        timestamp_ms: u64,
        agent_id: String,
        path: String,
        bytes: usize,
    },
    /// Script executed a glob pattern via FsBridge.
    ScriptFsGlob {
        timestamp_ms: u64,
        agent_id: String,
        pattern: String,
        matches: usize,
    },
    /// Agent entered Power Mode (from claw-pal security layer).
    ///
    /// Bridges the PAL-layer `SecurityAuditEvent` (Safe→Power transition) into
    /// the unified audit stream so all security events are queryable in one place.
    SecurityModeEntered {
        timestamp_ms: u64,
        agent_id: String,
        /// Hex-encoded first 8 bytes of the Argon2 hash (never the plaintext key).
        power_key_hash_prefix: String,
    },
    /// Agent exited Power Mode (from claw-pal security layer).
    ///
    /// Written when [`claw_pal::security::PowerModeGuard`] is dropped.
    SecurityModeExited {
        timestamp_ms: u64,
        agent_id: String,
        /// How long the agent was in Power Mode, in milliseconds.
        duration_ms: u64,
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
            AuditEvent::AgentSpawned { .. } => "AGENT_SPAWNED",
            AuditEvent::ScriptFsRead { .. } => "SCRIPT_FS_READ",
            AuditEvent::ScriptFsWrite { .. } => "SCRIPT_FS_WRITE",
            AuditEvent::ScriptFsGlob { .. } => "SCRIPT_FS_GLOB",
            AuditEvent::SecurityModeEntered { .. } => "SECURITY_MODE_ENTERED",
            AuditEvent::SecurityModeExited { .. } => "SECURITY_MODE_EXITED",
        }
    }

    /// Get the timestamp of the event.
    pub fn timestamp_ms(&self) -> u64 {
        match self {
            AuditEvent::ToolCall { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::ToolResult { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::PermissionCheck { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::ModeSwitch { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::AgentSpawned { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::ScriptFsRead { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::ScriptFsWrite { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::ScriptFsGlob { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::SecurityModeEntered { timestamp_ms, .. } => *timestamp_ms,
            AuditEvent::SecurityModeExited { timestamp_ms, .. } => *timestamp_ms,
        }
    }

    /// Get the agent ID associated with the event.
    pub fn agent_id(&self) -> &str {
        match self {
            AuditEvent::ToolCall { agent_id, .. } => agent_id,
            AuditEvent::ToolResult { agent_id, .. } => agent_id,
            AuditEvent::PermissionCheck { agent_id, .. } => agent_id,
            AuditEvent::ModeSwitch { agent_id, .. } => agent_id,
            AuditEvent::AgentSpawned { agent_id, .. } => agent_id,
            AuditEvent::ScriptFsRead { agent_id, .. } => agent_id,
            AuditEvent::ScriptFsWrite { agent_id, .. } => agent_id,
            AuditEvent::ScriptFsGlob { agent_id, .. } => agent_id,
            AuditEvent::SecurityModeEntered { agent_id, .. } => agent_id,
            AuditEvent::SecurityModeExited { agent_id, .. } => agent_id,
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
    /// Optional 32-byte HMAC-SHA256 key for per-record tamper detection.
    ///
    /// When `Some`, every record written to disk is wrapped in
    /// `{ "payload": <json>, "signature": "<hex>" }` where the signature is
    /// HMAC-SHA256(key, payload_bytes).  Leave `None` to use plain JSON lines.
    pub hmac_key: Option<[u8; 32]>,
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
            hmac_key: None,
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

    /// Set a 32-byte HMAC-SHA256 key for per-record tamper detection.
    ///
    /// Once set, every record written to disk will be signed and the log file
    /// switches to a JSON-lines format of `AuditRecord` objects instead of
    /// plain human-readable text.
    pub fn with_hmac_key(mut self, key: [u8; 32]) -> Self {
        self.hmac_key = Some(key);
        self
    }

    /// Get the full log file path.
    pub fn log_path(&self) -> PathBuf {
        self.log_dir.join(&self.log_filename)
    }
}

// ─── ToolsAuditSink — bridges claw-pal security events into AuditStore ───────

/// An [`claw_pal::audit::AuditSink`]-compatible sink that forwards
/// PAL-layer security events (`SecurityAuditEvent`) into a `claw-tools`
/// [`AuditStore`].
///
/// # Why this exists
///
/// `claw-pal` defines `AuditSink` and `SecurityAuditEvent` without depending on
/// `claw-tools` (to avoid a circular dependency).  `ToolsAuditSink` lives in
/// `claw-tools` and wraps an `Arc<AuditStore>`, acting as the bridge between the
/// two layers.
///
/// # Usage
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use claw_tools::audit::{AuditStore, ToolsAuditSink};
/// use claw_pal::audit::AuditSinkHandle;
///
/// let store = Arc::new(AuditStore::new(1000));
/// let sink: AuditSinkHandle = ToolsAuditSink::new_handle(Arc::clone(&store));
///
/// // Pass `sink` to PowerModeGuard::enter(...)
/// ```
pub struct ToolsAuditSink {
    store: Arc<AuditStore>,
}

impl ToolsAuditSink {
    /// Create a new sink wrapping the given store.
    pub fn new(store: Arc<AuditStore>) -> Self {
        Self { store }
    }

    /// Translate a PAL `SecurityAuditEvent` into an `AuditEvent` and push it.
    ///
    /// Mapping rules:
    /// - `from_mode == "safe"` → `AuditEvent::SecurityModeEntered`
    ///   (the `reason` field is used as `power_key_hash_prefix` for correlation)
    /// - `from_mode == "power"` → `AuditEvent::SecurityModeExited`
    ///   (the `reason` field encodes `session_ended_after_NNNms`; we parse `NNN`)
    /// - Other transitions → `AuditEvent::ModeSwitch` (forward as-is)
    pub fn push_security_event(&self, event: &SecurityAuditEventRepr) {
        let audit_event = if event.from_mode == "safe" && event.to_mode == "power" {
            AuditEvent::SecurityModeEntered {
                timestamp_ms: event.timestamp_ms,
                agent_id: event.agent_id.clone(),
                // Use first 8 chars of reason as a correlation handle.
                // For PowerModeGuard the reason is "power_key_verified".
                power_key_hash_prefix: event.reason.chars().take(8).collect(),
            }
        } else if event.from_mode == "power" && event.to_mode == "safe" {
            // reason format: "session_ended_after_NNNms"
            let duration_ms = event
                .reason
                .trim_start_matches("session_ended_after_")
                .trim_end_matches("ms")
                .parse::<u64>()
                .unwrap_or(0);
            AuditEvent::SecurityModeExited {
                timestamp_ms: event.timestamp_ms,
                agent_id: event.agent_id.clone(),
                duration_ms,
            }
        } else {
            AuditEvent::ModeSwitch {
                timestamp_ms: event.timestamp_ms,
                agent_id: event.agent_id.clone(),
                from_mode: event.from_mode.clone(),
                to_mode: event.to_mode.clone(),
                reason: event.reason.clone(),
            }
        };
        self.store.push(audit_event);
    }
}

/// Minimal representation of a PAL security audit event.
///
/// This struct mirrors `claw_pal::audit::SecurityAuditEvent` without depending on
/// the `claw-pal` crate.  When integrating `ToolsAuditSink` with `claw-pal`,
/// implement `claw_pal::audit::AuditSink` for a newtype that converts
/// `SecurityAuditEvent` into this repr and calls `push_security_event`.
///
/// See the `examples/` directory for a complete integration example.
#[derive(Debug, Clone)]
pub struct SecurityAuditEventRepr {
    pub timestamp_ms: u64,
    pub agent_id: String,
    pub from_mode: String,
    pub to_mode: String,
    pub reason: String,
}

impl SecurityAuditEventRepr {
    pub fn new(
        timestamp_ms: u64,
        agent_id: impl Into<String>,
        from_mode: impl Into<String>,
        to_mode: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            timestamp_ms,
            agent_id: agent_id.into(),
            from_mode: from_mode.into(),
            to_mode: to_mode.into(),
            reason: reason.into(),
        }
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

        let spawned = AuditEvent::AgentSpawned {
            timestamp_ms: 5,
            agent_id: "a2".to_string(),
            agent_name: "worker".to_string(),
            restart_policy: "on_failure".to_string(),
        };
        assert_eq!(spawned.event_type(), "AGENT_SPAWNED");
        assert_eq!(spawned.agent_id(), "a2");
        assert_eq!(spawned.timestamp_ms(), 5);

        let entered = AuditEvent::SecurityModeEntered {
            timestamp_ms: 6,
            agent_id: "a3".to_string(),
            power_key_hash_prefix: "abcdef01".to_string(),
        };
        assert_eq!(entered.event_type(), "SECURITY_MODE_ENTERED");
        assert_eq!(entered.agent_id(), "a3");
        assert_eq!(entered.timestamp_ms(), 6);

        let exited = AuditEvent::SecurityModeExited {
            timestamp_ms: 7,
            agent_id: "a3".to_string(),
            duration_ms: 5000,
        };
        assert_eq!(exited.event_type(), "SECURITY_MODE_EXITED");
        assert_eq!(exited.agent_id(), "a3");
        assert_eq!(exited.timestamp_ms(), 7);
    }

    #[test]
    fn test_tools_audit_sink_safe_to_power() {
        let store = Arc::new(AuditStore::new(10));
        let sink = ToolsAuditSink::new(Arc::clone(&store));

        let repr = SecurityAuditEventRepr::new(
            1_000,
            "agent-x",
            "safe",
            "power",
            "power_key_verified",
        );
        sink.push_security_event(&repr);

        let entries = store.list(10, Some("agent-x"), None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type(), "SECURITY_MODE_ENTERED");
        assert_eq!(entries[0].timestamp_ms(), 1_000);
        if let AuditEvent::SecurityModeEntered { power_key_hash_prefix, .. } = &entries[0] {
            assert_eq!(power_key_hash_prefix, "power_ke");
        } else {
            panic!("expected SecurityModeEntered");
        }
    }

    #[test]
    fn test_tools_audit_sink_power_to_safe() {
        let store = Arc::new(AuditStore::new(10));
        let sink = ToolsAuditSink::new(Arc::clone(&store));

        let repr = SecurityAuditEventRepr::new(
            2_000,
            "agent-y",
            "power",
            "safe",
            "session_ended_after_3500ms",
        );
        sink.push_security_event(&repr);

        let entries = store.list(10, Some("agent-y"), None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type(), "SECURITY_MODE_EXITED");
        if let AuditEvent::SecurityModeExited { duration_ms, .. } = &entries[0] {
            assert_eq!(*duration_ms, 3500);
        } else {
            panic!("expected SecurityModeExited");
        }
    }

    #[test]
    fn test_tools_audit_sink_other_transition_maps_to_mode_switch() {
        let store = Arc::new(AuditStore::new(10));
        let sink = ToolsAuditSink::new(Arc::clone(&store));

        let repr = SecurityAuditEventRepr::new(3_000, "agent-z", "debug", "safe", "manual");
        sink.push_security_event(&repr);

        let entries = store.list(10, Some("agent-z"), None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type(), "MODE_SWITCH");
    }

    #[test]
    fn test_audit_store_filters_by_agent() {
        let store = AuditStore::new(20);
        store.push(AuditEvent::SecurityModeEntered {
            timestamp_ms: 1,
            agent_id: "alpha".to_string(),
            power_key_hash_prefix: "00000000".to_string(),
        });
        store.push(AuditEvent::SecurityModeExited {
            timestamp_ms: 2,
            agent_id: "beta".to_string(),
            duration_ms: 100,
        });

        let alpha = store.list(10, Some("alpha"), None);
        assert_eq!(alpha.len(), 1);
        assert_eq!(alpha[0].event_type(), "SECURITY_MODE_ENTERED");

        let beta = store.list(10, Some("beta"), None);
        assert_eq!(beta.len(), 1);
        assert_eq!(beta[0].event_type(), "SECURITY_MODE_EXITED");
    }

    #[test]
    fn test_audit_log_config_default() {
        let config = AuditLogConfig::default();
        assert_eq!(config.max_memory_entries, 1000);
        assert_eq!(config.flush_interval_secs, 1);
        assert_eq!(config.max_file_size_bytes, 10 * 1024 * 1024);
        assert_eq!(config.log_filename, "audit.log");
        assert!(config.hmac_key.is_none());
    }

    #[test]
    fn test_audit_log_config_builder() {
        let key = [42u8; 32];
        let config = AuditLogConfig::new()
            .with_max_memory_entries(500)
            .with_flush_interval(5)
            .with_hmac_key(key);

        assert_eq!(config.max_memory_entries, 500);
        assert_eq!(config.flush_interval_secs, 5);
        assert_eq!(config.hmac_key, Some(key));
    }
}
