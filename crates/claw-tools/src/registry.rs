use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;

use crate::{
    error::RegistryError,
    traits::Tool,
    types::{LogEntry, SubprocessPolicy, ToolContext, ToolError, ToolMeta, ToolResult},
};

/// Thread-safe tool registry with permission checking and timeout execution.
pub struct ToolRegistry {
    tools: DashMap<String, Arc<dyn Tool>>,
    audit_log: DashMap<u64, LogEntry>, // key → entry
    max_audit_entries: usize,
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: DashMap::new(),
            audit_log: DashMap::new(),
            max_audit_entries: 10_000,
        }
    }

    /// Set the maximum number of audit log entries retained in memory.
    ///
    /// When the log exceeds this limit, the oldest 10 % of entries are evicted.
    pub fn with_max_audit_entries(mut self, max: usize) -> Self {
        self.max_audit_entries = max;
        self
    }

    /// Register a tool. Returns `RegistryError::AlreadyExists` if already registered.
    pub fn register(&self, tool: Arc<dyn Tool>) -> Result<(), RegistryError> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            return Err(RegistryError::AlreadyExists(name));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    /// Unregister a tool. Returns `RegistryError::ToolNotFound` if not registered.
    pub fn unregister(&self, name: &str) -> Result<(), RegistryError> {
        if self.tools.remove(name).is_none() {
            return Err(RegistryError::ToolNotFound(name.to_string()));
        }
        Ok(())
    }

    /// Get tool metadata (schema + permissions) without executing.
    pub fn tool_meta(&self, name: &str) -> Option<ToolMeta> {
        self.tools.get(name).map(|t| ToolMeta {
            schema: t.schema().clone(),
            permissions: t.permissions().clone(),
            timeout: t.timeout(),
            source_path: None,
        })
    }

    /// List all registered tool names.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|e| e.key().clone()).collect()
    }

    /// Count of registered tools.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Execute a tool with permission checking and timeout.
    ///
    /// Permission checking:
    /// - If tool requires `SubprocessPolicy::Allowed` but context grants `Denied` →
    ///   `RegistryError::PermissionDenied`
    ///
    /// Timeout: uses `tool.timeout()` wrapped in `tokio::time::timeout()`.
    pub async fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, RegistryError> {
        // 1. Find tool
        let tool = self
            .tools
            .get(name)
            .map(|e| Arc::clone(&*e))
            .ok_or_else(|| RegistryError::ToolNotFound(name.to_string()))?;

        // 2. Permission check: subprocess policy
        if tool.permissions().subprocess == SubprocessPolicy::Allowed
            && ctx.permissions.subprocess == SubprocessPolicy::Denied
        {
            return Err(RegistryError::PermissionDenied {
                tool: name.to_string(),
                permission: "subprocess".to_string(),
            });
        }

        // 2b. Filesystem permission check
        {
            let tool_fs = &tool.permissions().filesystem;
            let ctx_fs = &ctx.permissions.filesystem;
            if !tool_fs.read_paths.is_empty() && !tool_fs.read_paths.is_subset(&ctx_fs.read_paths) {
                return Err(RegistryError::PermissionDenied {
                    tool: name.to_string(),
                    permission: "filesystem:read".to_string(),
                });
            }
            if !tool_fs.write_paths.is_empty()
                && !tool_fs.write_paths.is_subset(&ctx_fs.write_paths)
            {
                return Err(RegistryError::PermissionDenied {
                    tool: name.to_string(),
                    permission: "filesystem:write".to_string(),
                });
            }
        }

        // 2c. Network permission check
        {
            let tool_net = &tool.permissions().network;
            let ctx_net = &ctx.permissions.network;
            if !tool_net.allowed_domains.is_empty()
                && !tool_net.allowed_domains.is_subset(&ctx_net.allowed_domains)
            {
                return Err(RegistryError::PermissionDenied {
                    tool: name.to_string(),
                    permission: "network".to_string(),
                });
            }
        }

        // 3. Execute with timeout
        let tool_timeout = tool.timeout();
        let start = Instant::now();

        let result = timeout(tool_timeout, tool.execute(args, &ctx)).await;

        let elapsed_ms = start.elapsed().as_millis() as u64;

        let tool_result = match result {
            Ok(r) => r,
            Err(_) => {
                // Timed out
                let entry = self.make_log_entry(&ctx.agent_id, name, false, elapsed_ms);
                self.insert_log(entry);
                return Ok(ToolResult::err(ToolError::timeout(), elapsed_ms));
            }
        };

        // 4. Audit log
        let entry = self.make_log_entry(&ctx.agent_id, name, tool_result.success, elapsed_ms);
        self.insert_log(entry);

        Ok(tool_result)
    }

    /// Recent audit log entries (last N entries by timestamp).
    pub fn recent_log(&self, n: usize) -> Vec<LogEntry> {
        let mut entries: Vec<LogEntry> = self.audit_log.iter().map(|e| e.value().clone()).collect();
        entries.sort_by_key(|e| e.timestamp_ms);
        entries.into_iter().rev().take(n).collect()
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn make_log_entry(
        &self,
        agent_id: &str,
        tool_name: &str,
        success: bool,
        duration_ms: u64,
    ) -> LogEntry {
        LogEntry {
            timestamp_ms: Self::now_ms(),
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            success,
            duration_ms,
        }
    }

    fn insert_log(&self, entry: LogEntry) {
        // Use timestamp as key; add a small uniqueness tweak via duration to
        // avoid key collisions when multiple calls happen within the same ms.
        let key = entry.timestamp_ms.wrapping_add(entry.duration_ms ^ 0xDEAD);
        self.audit_log.insert(key, entry);

        // Evict the oldest 10 % when over capacity.
        if self.audit_log.len() > self.max_audit_entries {
            let mut keys: Vec<u64> = self.audit_log.iter().map(|e| *e.key()).collect();
            keys.sort_unstable();
            let to_remove = (self.max_audit_entries / 10).max(1);
            for &k in keys.iter().take(to_remove) {
                self.audit_log.remove(&k);
            }
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        FsPermissions, NetworkPermissions, PermissionSet, ToolContext, ToolResult, ToolSchema,
    };
    use async_trait::async_trait;
    use std::time::Duration;

    // ── Mock tools ────────────────────────────────────────────────────────────

    struct EchoTool {
        schema: ToolSchema,
        perms: PermissionSet,
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echo"
        }
        fn schema(&self) -> &ToolSchema {
            &self.schema
        }
        fn permissions(&self) -> &PermissionSet {
            &self.perms
        }
        async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::ok(args, 0)
        }
    }

    struct SlowTool {
        schema: ToolSchema,
        perms: PermissionSet,
    }

    #[async_trait]
    impl Tool for SlowTool {
        fn name(&self) -> &str {
            "slow"
        }
        fn description(&self) -> &str {
            "Slow tool"
        }
        fn schema(&self) -> &ToolSchema {
            &self.schema
        }
        fn permissions(&self) -> &PermissionSet {
            &self.perms
        }
        fn timeout(&self) -> Duration {
            Duration::from_millis(1) // very short timeout
        }
        async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
            tokio::time::sleep(Duration::from_secs(60)).await;
            ToolResult::ok(serde_json::json!("done"), 0)
        }
    }

    fn make_echo_tool() -> Arc<dyn Tool> {
        Arc::new(EchoTool {
            schema: ToolSchema::new("echo", "Echo", serde_json::json!({})),
            perms: PermissionSet::minimal(),
        })
    }

    fn make_slow_tool() -> Arc<dyn Tool> {
        Arc::new(SlowTool {
            schema: ToolSchema::new("slow", "Slow tool", serde_json::json!({})),
            perms: PermissionSet::minimal(),
        })
    }

    fn default_ctx() -> ToolContext {
        ToolContext::new("agent-1", PermissionSet::minimal())
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_registry_new() {
        let reg = ToolRegistry::new();
        assert_eq!(reg.tool_count(), 0);
        assert!(reg.tool_names().is_empty());
    }

    #[test]
    fn test_registry_register_and_count() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        assert_eq!(reg.tool_count(), 1);
    }

    #[test]
    fn test_registry_register_duplicate_fails() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        let err = reg.register(make_echo_tool()).unwrap_err();
        assert!(matches!(err, RegistryError::AlreadyExists(_)));
    }

    #[test]
    fn test_registry_unregister() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        reg.unregister("echo").unwrap();
        assert_eq!(reg.tool_count(), 0);
    }

    #[test]
    fn test_registry_unregister_nonexistent_fails() {
        let reg = ToolRegistry::new();
        let err = reg.unregister("nonexistent").unwrap_err();
        assert!(matches!(err, RegistryError::ToolNotFound(_)));
    }

    #[test]
    fn test_registry_tool_names() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        let names = reg.tool_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"echo".to_string()));
    }

    #[test]
    fn test_registry_tool_meta() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        let meta = reg.tool_meta("echo").expect("should have meta");
        assert_eq!(meta.schema.name, "echo");
        assert_eq!(meta.timeout, Duration::from_secs(30));
        assert!(meta.source_path.is_none());
    }

    #[tokio::test]
    async fn test_registry_execute_echo_tool() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        let args = serde_json::json!({"msg": "hello"});
        let result = reg
            .execute("echo", args.clone(), default_ctx())
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output.as_ref().unwrap(), &args);
    }

    #[tokio::test]
    async fn test_registry_execute_nonexistent_tool_fails() {
        let reg = ToolRegistry::new();
        let err = reg
            .execute("ghost", serde_json::json!({}), default_ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, RegistryError::ToolNotFound(_)));
    }

    #[tokio::test]
    async fn test_registry_execute_timeout() {
        let reg = ToolRegistry::new();
        reg.register(make_slow_tool()).unwrap();
        let result = reg
            .execute("slow", serde_json::json!({}), default_ctx())
            .await
            .unwrap();
        // Should return a ToolResult with Timeout error (not a RegistryError)
        assert!(!result.success);
        let err = result.error.as_ref().expect("should have error");
        assert_eq!(err.code, crate::types::ToolErrorCode::Timeout);
    }

    // ── FS permission tests ───────────────────────────────────────────────────

    /// A mock tool whose `name()` delegates to `schema.name`, allowing tests to
    /// create tools with distinct names without defining a new struct each time.
    struct NamedTool {
        schema: ToolSchema,
        perms: PermissionSet,
    }

    #[async_trait]
    impl Tool for NamedTool {
        fn name(&self) -> &str {
            &self.schema.name
        }
        fn description(&self) -> &str {
            &self.schema.description
        }
        fn schema(&self) -> &ToolSchema {
            &self.schema
        }
        fn permissions(&self) -> &PermissionSet {
            &self.perms
        }
        async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::ok(args, 0)
        }
    }

    fn make_fs_read_tool(path: &str) -> Arc<dyn Tool> {
        Arc::new(NamedTool {
            schema: ToolSchema::new("echo_fs_read", "FS read tool", serde_json::json!({})),
            perms: PermissionSet {
                filesystem: FsPermissions::read_only(vec![path.to_string()]),
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Denied,
            },
        })
    }

    fn make_fs_write_tool(path: &str) -> Arc<dyn Tool> {
        Arc::new(NamedTool {
            schema: ToolSchema::new("echo_fs_write", "FS write tool", serde_json::json!({})),
            perms: PermissionSet {
                filesystem: FsPermissions {
                    read_paths: std::collections::HashSet::new(),
                    write_paths: vec![path.to_string()].into_iter().collect(),
                },
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Denied,
            },
        })
    }

    fn make_network_tool(domain: &str) -> Arc<dyn Tool> {
        Arc::new(NamedTool {
            schema: ToolSchema::new("echo_net", "Network tool", serde_json::json!({})),
            perms: PermissionSet {
                filesystem: FsPermissions::none(),
                network: NetworkPermissions::allow(vec![domain.to_string()]),
                subprocess: SubprocessPolicy::Denied,
            },
        })
    }

    #[tokio::test]
    async fn test_registry_fs_read_permission_denied() {
        let reg = ToolRegistry::new();
        // Tool requires read access to /secret — context grants nothing.
        reg.register(make_fs_read_tool("/secret")).unwrap();
        let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
        let err = reg
            .execute("echo_fs_read", serde_json::json!({}), ctx)
            .await
            .unwrap_err();
        assert!(
            matches!(
                &err,
                RegistryError::PermissionDenied { permission, .. }
                if permission == "filesystem:read"
            ),
            "expected PermissionDenied(filesystem:read), got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_registry_fs_write_permission_denied() {
        let reg = ToolRegistry::new();
        // Tool requires write access to /data — context grants nothing.
        reg.register(make_fs_write_tool("/data")).unwrap();
        let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
        let err = reg
            .execute("echo_fs_write", serde_json::json!({}), ctx)
            .await
            .unwrap_err();
        assert!(
            matches!(
                &err,
                RegistryError::PermissionDenied { permission, .. }
                if permission == "filesystem:write"
            ),
            "expected PermissionDenied(filesystem:write), got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_registry_network_permission_denied() {
        let reg = ToolRegistry::new();
        // Tool requires api.example.com — context grants nothing.
        reg.register(make_network_tool("api.example.com")).unwrap();
        let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
        let err = reg
            .execute("echo_net", serde_json::json!({}), ctx)
            .await
            .unwrap_err();
        assert!(
            matches!(
                &err,
                RegistryError::PermissionDenied { permission, .. }
                if permission == "network"
            ),
            "expected PermissionDenied(network), got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_registry_fs_permission_granted() {
        let reg = ToolRegistry::new();
        // Tool requires read access to /data/logs — context grants exactly that.
        reg.register(make_fs_read_tool("/data/logs")).unwrap();
        let ctx = ToolContext::new(
            "agent-1",
            PermissionSet {
                filesystem: FsPermissions::read_only(vec!["/data/logs".to_string()]),
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Denied,
            },
        );
        let result = reg
            .execute("echo_fs_read", serde_json::json!({}), ctx)
            .await
            .expect("should succeed when permissions are granted");
        assert!(result.success);
    }

    // ── 4B: 审计日志上限测试 ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_registry_audit_log_max_entries() {
        // max_audit_entries = 10 → after 20 calls the log should have been evicted
        // at least once; total entries must not exceed max + to_remove (11).
        let reg = ToolRegistry::new().with_max_audit_entries(10);
        reg.register(make_echo_tool()).unwrap();
        let ctx = default_ctx();

        for i in 0..20u64 {
            reg.execute("echo", serde_json::json!({"i": i}), ctx.clone())
                .await
                .unwrap();
        }

        // After eviction(s) the log length must be ≤ 11 (max + 10 % slack).
        let log_len = reg.recent_log(100).len();
        assert!(
            log_len <= 11,
            "expected audit log ≤ 11 entries after eviction, got {log_len}"
        );
    }
}
