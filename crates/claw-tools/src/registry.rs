use dashmap::DashMap;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Global monotonic counter for audit log entry IDs.
/// Using SeqCst ordering for maximum safety across all threads.
static AUDIT_COUNTER: AtomicU64 = AtomicU64::new(1);

use crate::{
    error::RegistryError,
    traits::Tool,
    types::{LogEntry, SubprocessPolicy, ToolContext, ToolError, ToolMeta, ToolResult},
};

/// Thread-safe tool registry with permission checking and timeout execution.
pub struct ToolRegistry {
    tools: DashMap<String, Arc<dyn Tool>>,
    audit_log: RwLock<BTreeMap<u64, LogEntry>>, // key → entry (ordered by ID)
    max_audit_entries: usize,
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: DashMap::new(),
            audit_log: RwLock::new(BTreeMap::new()),
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

    /// Generate a unique monotonic audit ID using a global AtomicU64 counter.
    ///
    /// This approach eliminates clock skew risk (NTP sync, manual time changes)
    /// by using a purely monotonic counter instead of timestamp-based IDs.
    /// Uses SeqCst ordering for maximum thread safety.
    fn generate_audit_id(&self) -> u64 {
        AUDIT_COUNTER.fetch_add(1, Ordering::SeqCst)
    }

    /// Register a tool. Returns `RegistryError::AlreadyExists` if already registered.
    pub fn register(&self, tool: Box<dyn Tool>) -> Result<(), RegistryError> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            return Err(RegistryError::AlreadyExists(name));
        }
        self.tools.insert(name, Arc::from(tool));
        Ok(())
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).map(|t| Arc::clone(&*t))
    }

    /// Unregister a tool. Returns `RegistryError::ToolNotFound` if not registered.
    pub fn unregister(&self, name: &str) -> Result<(), RegistryError> {
        if self.tools.remove(name).is_none() {
            return Err(RegistryError::ToolNotFound(name.to_string()));
        }
        Ok(())
    }

    /// Update (or register) a tool.
    /// If the tool exists, it is replaced; otherwise it is registered.
    pub fn update(&self, name: &str, tool: Arc<dyn Tool>) -> Result<(), RegistryError> {
        self.tools.insert(name.to_string(), tool);
        Ok(())
    }

    /// Get tool metadata (schema + permissions) without executing.
    pub fn tool_meta(&self, name: &str) -> Option<ToolMeta> {
        self.tools.get(name).map(|t| ToolMeta {
            schema: t.schema().clone(),
            permissions: t.permissions().clone(),
            timeout: t.timeout(),
            source: crate::types::ToolSource::Native,
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

    /// Execute a tool with permission checking, timeout, and audit logging.
    ///
    /// This is the main entry point for tool execution. It performs the following steps:
    /// 1. Looks up the tool by name
    /// 2. Validates permissions (filesystem, network, subprocess)
    /// 3. Executes the tool with a timeout using `JoinSet` for cancellation safety
    /// 4. Records the execution in the audit log
    ///
    /// # Permission Checking
    ///
    /// - **Subprocess**: If tool requires `SubprocessPolicy::Allowed` but context grants `Denied`,
    ///   returns `RegistryError::PermissionDenied`
    /// - **Filesystem**: Tool's read/write paths must be a subset of context's allowed paths
    /// - **Network**: Tool's allowed domains must be a subset of context's allowed domains
    ///
    /// # Timeout Handling
    ///
    /// Uses `tool.timeout()` (default 30s) wrapped in `tokio::time::timeout()`.
    /// When timeout occurs, the task is cancelled via `CancellationToken` and
    /// a `ToolResult` with `ToolErrorCode::Timeout` is returned.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the tool to execute
    /// * `args` - JSON value containing the tool arguments
    /// * `ctx` - Tool context containing agent ID and permissions
    ///
    /// # Returns
    ///
    /// - `Ok(ToolResult)` - Tool executed (check `ToolResult::success` for outcome)
    /// - `Err(RegistryError)` - Tool not found or permission denied
    ///
    /// # Example
    ///
    /// ```
    /// use claw_tools::{registry::ToolRegistry, types::{ToolContext, PermissionSet}};
    /// use serde_json::json;
    ///
    /// async fn example() {
    ///     let registry = ToolRegistry::new();
    ///     // ... register tools ...
    ///
    ///     let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
    ///     let result = registry.execute("echo", json!({"message": "hello"}), ctx).await;
    ///
    ///     match result {
    ///         Ok(tool_result) if tool_result.success => {
    ///             println!("Success: {:?}", tool_result.output);
    ///         }
    ///         Ok(tool_result) => {
    ///             println!("Tool failed: {:?}", tool_result.error);
    ///         }
    ///         Err(e) => {
    ///             println!("Execution error: {}", e);
    ///         }
    ///     }
    /// }
    /// ```
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

        // 3. Execute with timeout and cancellation support using JoinSet (RES-001)
        let tool_timeout = tool.timeout();
        let start = Instant::now();
        let cancellation_token = CancellationToken::new();

        // Create a JoinSet to manage the tool execution task
        let mut join_set = JoinSet::new();

        // Spawn the tool execution into the JoinSet
        let tool_clone = Arc::clone(&tool);
        let args_clone = args.clone();
        let ctx_clone = ctx.clone();
        let cancel_clone = cancellation_token.clone();

        join_set.spawn(async move {
            // Future that wraps tool execution with cancellation check
            tokio::select! {
                result = tool_clone.execute(args_clone, &ctx_clone) => {
                    result
                }
                _ = cancel_clone.cancelled() => {
                    // Cancelled by timeout - return timeout error
                    ToolResult::err(ToolError::timeout(), 0)
                }
            }
        });

        // Wait for completion with timeout using JoinSet
        let tool_result = match tokio::time::timeout(tool_timeout, join_set.join_next()).await {
            Ok(Some(Ok(result))) => {
                // Task completed successfully
                result
            }
            Ok(Some(Err(join_err))) => {
                // Task panicked or was cancelled
                if join_err.is_cancelled() || join_err.is_panic() {
                    ToolResult::err(
                        ToolError::internal(format!("Task error: {}", join_err)),
                        start.elapsed().as_millis() as u64,
                    )
                } else {
                    ToolResult::err(
                        ToolError::internal(format!("Task join error: {}", join_err)),
                        start.elapsed().as_millis() as u64,
                    )
                }
            }
            Ok(None) => {
                // JoinSet is empty (should not happen as we only spawn one task)
                ToolResult::err(
                    ToolError::internal("Task not spawned"),
                    start.elapsed().as_millis() as u64,
                )
            }
            Err(_) => {
                // Timeout occurred - cancel the token and shutdown all tasks in JoinSet
                cancellation_token.cancel();
                join_set.shutdown().await;

                let elapsed_ms = start.elapsed().as_millis() as u64;
                let entry = self.make_log_entry(&ctx.agent_id, name, false, elapsed_ms);
                self.insert_log(entry).await;
                return Ok(ToolResult::err(ToolError::timeout(), elapsed_ms));
            }
        };

        let elapsed_ms = start.elapsed().as_millis() as u64;

        // 4. Audit log
        let entry = self.make_log_entry(&ctx.agent_id, name, tool_result.success, elapsed_ms);
        self.insert_log(entry).await;

        Ok(tool_result)
    }

    /// Recent audit log entries (last N entries by timestamp).
    pub async fn recent_log(&self, n: usize) -> Vec<LogEntry> {
        let guard = self.audit_log.read().await;
        guard.values().rev().take(n).cloned().collect()
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

    async fn insert_log(&self, entry: LogEntry) {
        // Use monotonic counter-based ID to eliminate clock skew risk.
        let key = self.generate_audit_id();

        let mut guard = self.audit_log.write().await;
        guard.insert(key, entry);

        // Evict the oldest 10 % when over capacity (smallest IDs are oldest).
        // BTreeMap provides O(1) access to first (oldest) entry via first_key_value().
        // Removal is O(log n) per entry, making this O(k log n) for k entries removed.
        if guard.len() > self.max_audit_entries {
            let to_remove = (self.max_audit_entries / 10).max(1);
            for _ in 0..to_remove {
                if let Some((&oldest_key, _)) = guard.first_key_value() {
                    guard.remove(&oldest_key);
                } else {
                    break;
                }
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

    fn make_echo_tool() -> Box<dyn Tool> {
        Box::new(EchoTool {
            schema: ToolSchema::new("echo", "Echo", serde_json::json!({})),
            perms: PermissionSet::minimal(),
        })
    }

    fn make_slow_tool() -> Box<dyn Tool> {
        Box::new(SlowTool {
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
        assert!(matches!(meta.source, crate::types::ToolSource::Native))
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

    fn make_fs_read_tool(path: &str) -> Box<dyn Tool> {
        Box::new(NamedTool {
            schema: ToolSchema::new("echo_fs_read", "FS read tool", serde_json::json!({})),
            perms: PermissionSet {
                filesystem: FsPermissions::read_only(vec![path.to_string()]),
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Denied,
            },
        })
    }

    fn make_fs_write_tool(path: &str) -> Box<dyn Tool> {
        Box::new(NamedTool {
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

    fn make_network_tool(domain: &str) -> Box<dyn Tool> {
        Box::new(NamedTool {
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
        let log_len = reg.recent_log(100).await.len();
        assert!(
            log_len <= 11,
            "expected audit log ≤ 11 entries after eviction, got {log_len}"
        );
    }

    // ─── JoinSet 超时测试 (Agent 4: Red Phase) ────────────────────────────────

    /// Test: 工具超时取消不会引发 Panic
    ///
    /// 验证使用 JoinSet 实现后，超时取消工具执行是安全的，不会 panic
    #[tokio::test]
    async fn test_registry_execute_timeout_joinset() {
        let reg = ToolRegistry::new();
        reg.register(make_slow_tool()).unwrap();

        // 执行会超时的工具
        let result = reg
            .execute("slow", serde_json::json!({}), default_ctx())
            .await;

        // 验证：应该返回 Ok(ToolResult) 而不是 Err
        assert!(result.is_ok(), "超时应该返回 Ok(ToolResult)，不应返回 Err");

        let tool_result = result.unwrap();
        // 验证：ToolResult 应该标记为失败且包含超时错误
        assert!(!tool_result.success, "超时工具应返回 success=false");
        let err = tool_result.error.as_ref().expect("应该有错误信息");
        assert_eq!(
            err.code,
            crate::types::ToolErrorCode::Timeout,
            "错误类型应该是 Timeout"
        );

        // 关键验证：此测试不应 panic，证明 JoinSet 的超时取消是安全的
    }

    /// Test: 并发执行多个工具验证 JoinSet 能正确管理多个任务
    ///
    /// 这个测试验证 JoinSet 能够正确管理并发执行的多个工具任务
    #[tokio::test]
    async fn test_registry_concurrent_tools_execution() {
        use std::sync::Arc;

        let reg = Arc::new(ToolRegistry::new());

        // 注册多个工具
        reg.register(make_echo_tool()).unwrap();

        // 创建另一个快速工具
        struct QuickTool;
        #[async_trait]
        impl Tool for QuickTool {
            fn name(&self) -> &str {
                "quick"
            }
            fn description(&self) -> &str {
                "Quick tool"
            }
            fn schema(&self) -> &ToolSchema {
                static SCHEMA: std::sync::OnceLock<ToolSchema> = std::sync::OnceLock::new();
                SCHEMA.get_or_init(|| ToolSchema::new("quick", "Quick tool", serde_json::json!({})))
            }
            fn permissions(&self) -> &PermissionSet {
                static PERMS: std::sync::OnceLock<PermissionSet> = std::sync::OnceLock::new();
                PERMS.get_or_init(PermissionSet::minimal)
            }
            async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
                // 短暂延迟模拟实际工作
                tokio::time::sleep(Duration::from_millis(10)).await;
                ToolResult::ok(args, 0)
            }
        }

        reg.register(Box::new(QuickTool)).unwrap();

        let ctx = default_ctx();

        // 并发执行多个工具调用
        let mut handles = vec![];
        for i in 0..5u64 {
            let reg_clone = Arc::clone(&reg);
            let ctx_clone = ctx.clone();
            handles.push(tokio::spawn(async move {
                reg_clone
                    .execute("echo", serde_json::json!({"i": i}), ctx_clone)
                    .await
            }));
        }

        // 等待所有任务完成
        let mut success_count = 0;
        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result.is_ok(), "并发执行不应返回错误");
            if result.unwrap().success {
                success_count += 1;
            }
        }

        assert_eq!(success_count, 5, "所有并发工具调用都应该成功");

        // 验证审计日志记录了所有调用
        let log = reg.recent_log(10).await;
        assert_eq!(log.len(), 5, "审计日志应该记录5条记录");
    }
}
